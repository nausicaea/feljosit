"""Find transfers from two matching transactions on two accounts.

The import finds most transfers from the counterparty IBAN (see
:mod:`feljosit.services.categorise`). Some statements do not show the IBAN of the
other account, for example the statement of a credit card. This module finds a
transfer from the two transactions of the movement: an outflow on one account and
an inflow of the same amount on a different account, some days apart.

The rules prevent wrong transfers, because a wrong transfer hides an expense:

- Only uncategorised transactions that are not transfers. A category that the
  user set never changes. Starting balances are not included.
- The two accounts belong to the user and have the same budget setting (two budget
  accounts or two tracking accounts, see ADR-0008).
- The same currency and the opposite amount.
- The booking dates are at most ``MAX_DAYS`` days apart.
- The pair is unambiguous: each transaction is the only nearest candidate of the
  other. If two candidates have the same distance in days, no transaction is marked.
- The user did not reject the pair before.

The table ``transfer_pairs`` records each pair, also the pairs that the import
finds from the IBAN. Thus the user can see the partner of a transfer, and "not a
transfer" changes the two transactions together.
"""

from __future__ import annotations

from collections import defaultdict
from dataclasses import dataclass
from datetime import date, timedelta

from sqlalchemy import select
from sqlalchemy.orm import Session

from feljosit.models import (
    Account,
    Transaction,
    TransactionSource,
    TransferMethod,
    TransferPair,
    TransferStatus,
)

MAX_DAYS = 4
"""The maximum number of days between the two bookings of one transfer."""


@dataclass(frozen=True)
class _Candidate:
    tx: Transaction
    on_budget: bool


def _best(candidate: _Candidate, others: list[_Candidate]) -> _Candidate | None:
    """The only nearest possible partner of ``candidate``, or ``None``."""
    possible = [
        (abs((other.tx.booking_date - candidate.tx.booking_date).days), other)
        for other in others
        if other.tx.account_id != candidate.tx.account_id
        and other.on_budget == candidate.on_budget
        and abs((other.tx.booking_date - candidate.tx.booking_date).days) <= MAX_DAYS
    ]
    if not possible:
        return None
    nearest = min(days for days, _ in possible)
    best = [other for days, other in possible if days == nearest]
    return best[0] if len(best) == 1 else None


def match_transfers(
    db: Session, user_id: int, start: date | None = None, end: date | None = None
) -> list[tuple[Transaction, Transaction]]:
    """Mark the matching pairs of transactions as transfers. Return the pairs.

    ``start`` and ``end`` limit the transactions that can start a pair (for
    example the dates of one import). A partner can be up to ``MAX_DAYS`` days
    outside this range.
    """
    query = (
        select(Transaction, Account.on_budget)
        .join(Account, Transaction.account_id == Account.id)
        .where(
            Transaction.user_id == user_id,
            Transaction.is_transfer.is_(False),
            Transaction.category_id.is_(None),
            Transaction.source != TransactionSource.STARTING_BALANCE,
            Transaction.amount_minor != 0,
        )
    )
    if start is not None:
        query = query.where(Transaction.booking_date >= start - timedelta(days=MAX_DAYS))
    if end is not None:
        query = query.where(Transaction.booking_date <= end + timedelta(days=MAX_DAYS))

    # Group by currency and absolute amount: only these can match.
    outflows: dict[tuple[str, int], list[_Candidate]] = defaultdict(list)
    inflows: dict[tuple[str, int], list[_Candidate]] = defaultdict(list)
    for tx, on_budget in db.execute(query):
        key = (tx.currency, abs(tx.amount_minor))
        (outflows if tx.amount_minor < 0 else inflows)[key].append(_Candidate(tx, on_budget))

    def in_range(tx: Transaction) -> bool:
        return (start is None or tx.booking_date >= start) and (
            end is None or tx.booking_date <= end
        )

    rejected = _rejected_pairs(db, user_id)
    pairs: list[tuple[Transaction, Transaction]] = []
    for key, sent in outflows.items():
        received = inflows.get(key, [])
        for out in sent:
            allowed = [c for c in received if (out.tx.id, c.tx.id) not in rejected]
            partner = _best(out, allowed)
            if partner is None:
                continue
            others = [c for c in sent if (c.tx.id, partner.tx.id) not in rejected]
            if _best(partner, others) is not out:
                continue
            if not (in_range(out.tx) or in_range(partner.tx)):
                continue
            out.tx.is_transfer = True
            partner.tx.is_transfer = True
            _add_pair(db, out.tx, partner.tx, TransferMethod.MATCH)
            pairs.append((out.tx, partner.tx))
    db.flush()
    return pairs


def _rejected_pairs(db: Session, user_id: int) -> set[tuple[int, int]]:
    return {
        (outflow_id, inflow_id)
        for outflow_id, inflow_id in db.execute(
            select(TransferPair.outflow_id, TransferPair.inflow_id).where(
                TransferPair.user_id == user_id,
                TransferPair.status == TransferStatus.REJECTED,
            )
        )
    }


def _add_pair(
    db: Session, outflow: Transaction, inflow: Transaction, method: TransferMethod
) -> TransferPair:
    pair = TransferPair(
        user_id=outflow.user_id,
        outflow_id=outflow.id,
        inflow_id=inflow.id,
        method=method,
        status=TransferStatus.ACTIVE,
    )
    db.add(pair)
    return pair


def active_pair(db: Session, tx: Transaction) -> TransferPair | None:
    """The active pair of a transaction, or ``None``."""
    return db.scalar(
        select(TransferPair).where(
            TransferPair.status == TransferStatus.ACTIVE,
            (TransferPair.outflow_id == tx.id) | (TransferPair.inflow_id == tx.id),
        )
    )


@dataclass(frozen=True)
class Partner:
    """The other transaction of a transfer pair, for the transaction list."""

    tx: Transaction
    method: TransferMethod


def partners(db: Session, transactions: list[Transaction]) -> dict[int, Partner]:
    """The partners of the transactions that are in an active pair, by transaction ID."""
    ids = [tx.id for tx in transactions]
    if not ids:
        return {}
    result: dict[int, Partner] = {}
    pairs = db.scalars(
        select(TransferPair).where(
            TransferPair.status == TransferStatus.ACTIVE,
            TransferPair.outflow_id.in_(ids) | TransferPair.inflow_id.in_(ids),
        )
    )
    for pair in pairs:
        result[pair.outflow_id] = Partner(pair.inflow, pair.method)
        result[pair.inflow_id] = Partner(pair.outflow, pair.method)
    return result


def pair_iban_transfers(db: Session, transactions: list[Transaction]) -> int:
    """Record the pairs of transfers that the import found from the IBAN.

    For each transfer without a pair, the partner is the only nearest transfer on the
    account with the counterparty IBAN, with the opposite amount, at most
    ``MAX_DAYS`` days apart and without a pair. Return the number of new pairs.
    """
    count = 0
    for tx in transactions:
        if not tx.is_transfer or not tx.counterparty_iban or active_pair(db, tx) is not None:
            continue
        other = db.scalar(
            select(Account).where(
                Account.user_id == tx.user_id, Account.iban == tx.counterparty_iban
            )
        )
        if other is None:
            continue
        rows = db.scalars(
            select(Transaction).where(
                Transaction.account_id == other.id,
                Transaction.is_transfer.is_(True),
                Transaction.currency == tx.currency,
                Transaction.amount_minor == -tx.amount_minor,
                Transaction.booking_date >= tx.booking_date - timedelta(days=MAX_DAYS),
                Transaction.booking_date <= tx.booking_date + timedelta(days=MAX_DAYS),
            )
        )
        candidates = [c for c in rows if active_pair(db, c) is None]
        if not candidates:
            continue
        nearest = min(abs((c.booking_date - tx.booking_date).days) for c in candidates)
        best = [c for c in candidates if abs((c.booking_date - tx.booking_date).days) == nearest]
        if len(best) != 1:
            continue
        outflow, inflow = (tx, best[0]) if tx.amount_minor < 0 else (best[0], tx)
        _add_pair(db, outflow, inflow, TransferMethod.IBAN)
        db.flush()
        count += 1
    return count


def reject_transfer(db: Session, tx: Transaction) -> Transaction | None:
    """The user says that ``tx`` is not a transfer. Return the partner, if any.

    Both transactions of the pair stop being transfers. They stay uncategorised. The
    pair gets the status "rejected", so that the matching does not make it again.
    """
    tx.is_transfer = False
    pair = active_pair(db, tx)
    if pair is None:
        return None
    pair.status = TransferStatus.REJECTED
    partner = pair.inflow if pair.outflow_id == tx.id else pair.outflow
    partner.is_transfer = False
    partner.category_id = None
    db.flush()
    return partner


def before_delete(db: Session, tx: Transaction) -> None:
    """Prepare the deletion of a transaction.

    The pair is deleted with the transaction. If the application found the pair from
    matching amounts, the partner is not a transfer any more.
    """
    pair = active_pair(db, tx)
    if pair is not None and pair.method == TransferMethod.MATCH:
        partner = pair.inflow if pair.outflow_id == tx.id else pair.outflow
        partner.is_transfer = False
