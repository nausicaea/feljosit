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
"""

from __future__ import annotations

from collections import defaultdict
from dataclasses import dataclass
from datetime import date, timedelta

from sqlalchemy import select
from sqlalchemy.orm import Session

from feljosit.models import Account, Transaction, TransactionSource

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

    pairs: list[tuple[Transaction, Transaction]] = []
    for key, sent in outflows.items():
        received = inflows.get(key, [])
        for out in sent:
            partner = _best(out, received)
            if partner is None or _best(partner, sent) is not out:
                continue
            if not (in_range(out.tx) or in_range(partner.tx)):
                continue
            out.tx.is_transfer = True
            partner.tx.is_transfer = True
            pairs.append((out.tx, partner.tx))
    db.flush()
    return pairs
