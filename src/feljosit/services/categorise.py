"""Automatic transaction categorisation.

The order of the methods:

1. Transfers. If the counterparty IBAN is the IBAN of another account of the same
   user, and both accounts are in the budget (or both are tracking accounts), the
   transaction is a transfer. A transfer has no category. A transfer from a budget
   account to a tracking account (for example, to pillar 3a) needs a category,
   because the money leaves the budget.
2. Rules. The user makes rules (field, match type, pattern). The rule with the
   lowest priority number applies first. The rules use simple text comparison and
   not regular expressions. This prevents ReDoS attacks.
3. History. If the user categorised a transaction with the same counterparty IBAN
   or counterparty name before, the most recent category applies.

Between 1 and 2, :mod:`feljosit.services.transfers` finds the transfers that have
no IBAN: two matching transactions on two accounts.
"""

from __future__ import annotations

from collections.abc import Iterable
from dataclasses import dataclass

from sqlalchemy import select
from sqlalchemy.orm import Session

from feljosit.models import (
    Account,
    CategorisationRule,
    Category,
    MatchType,
    RuleField,
    Transaction,
)
from feljosit.services.errors import NotFoundError, ServiceError
from feljosit.services.transfers import match_transfers, reject_transfer


@dataclass(frozen=True)
class _Candidate:
    counterparty: str
    counterparty_iban: str | None
    description: str


def _field_value(candidate: _Candidate, field: RuleField) -> str:
    match field:
        case RuleField.COUNTERPARTY:
            return candidate.counterparty
        case RuleField.COUNTERPARTY_IBAN:
            return candidate.counterparty_iban or ""
        case RuleField.DESCRIPTION:
            return candidate.description


def rule_matches(rule: CategorisationRule, candidate: _Candidate) -> bool:
    value = _field_value(candidate, rule.field).casefold()
    pattern = rule.pattern.casefold()
    if not pattern:
        return False
    if rule.match_type is MatchType.EQUALS:
        return value == pattern
    return pattern in value


class Categoriser:
    """Categorise transactions for one user. Load the rules and the history one time."""

    def __init__(self, db: Session, user_id: int) -> None:
        self._rules = list(
            db.scalars(
                select(CategorisationRule)
                .where(CategorisationRule.user_id == user_id)
                .order_by(CategorisationRule.priority, CategorisationRule.id)
            )
        )
        self._own_ibans = {
            iban: (account_id, on_budget)
            for account_id, iban, on_budget in db.execute(
                select(Account.id, Account.iban, Account.on_budget).where(
                    Account.user_id == user_id, Account.iban.is_not(None)
                )
            )
        }
        self._by_iban: dict[str, int] = {}
        self._by_name: dict[str, int] = {}
        history = db.execute(
            select(Transaction.counterparty, Transaction.counterparty_iban, Transaction.category_id)
            .where(Transaction.user_id == user_id, Transaction.category_id.is_not(None))
            .order_by(Transaction.booking_date, Transaction.id)
        )
        for name, iban, category_id in history:
            if category_id is None:  # pragma: no cover - excluded by the query
                continue
            if iban:
                self._by_iban[iban] = category_id
            if name:
                self._by_name[name.casefold()] = category_id

    def is_transfer(self, account: Account, counterparty_iban: str | None) -> bool:
        if not counterparty_iban:
            return False
        other = self._own_ibans.get(counterparty_iban)
        if other is None:
            return False
        other_id, other_on_budget = other
        return other_id != account.id and other_on_budget == account.on_budget

    def suggest(
        self, counterparty: str, counterparty_iban: str | None, description: str
    ) -> int | None:
        candidate = _Candidate(counterparty, counterparty_iban, description)
        for rule in self._rules:
            if rule_matches(rule, candidate):
                return rule.category_id
        if counterparty_iban and counterparty_iban in self._by_iban:
            return self._by_iban[counterparty_iban]
        return self._by_name.get(counterparty.casefold()) if counterparty else None

    def learn(self, tx: Transaction) -> None:
        if tx.category_id is None:
            return
        if tx.counterparty_iban:
            self._by_iban[tx.counterparty_iban] = tx.category_id
        if tx.counterparty:
            self._by_name[tx.counterparty.casefold()] = tx.category_id


def owned_category(db: Session, user_id: int, category_id: int) -> Category:
    category = db.get(Category, category_id)
    if category is None or category.user_id != user_id:
        raise NotFoundError("category")
    return category


def set_category(db: Session, tx: Transaction, category_id: int | None) -> Transaction | None:
    """Set the category of a transaction (``None``: uncategorised).

    If the transaction is a transfer, it stops being a transfer. If it is in a
    transfer pair, the other transaction of the pair stops being a transfer too (see
    :func:`feljosit.services.transfers.reject_transfer`). Return that transaction.
    """
    if category_id is not None:
        owned_category(db, tx.user_id, category_id)
        if not tx.account.on_budget:
            raise ServiceError("Transactions in tracking accounts have no category.")
    partner = reject_transfer(db, tx) if tx.is_transfer else None
    tx.category_id = category_id
    return partner


def _checked_rule_values(
    db: Session, user_id: int, pattern: str, priority: int, category_id: int
) -> str:
    """Check the values of a rule. Return the pattern without spaces at the ends."""
    pattern = pattern.strip()
    if not 1 <= len(pattern) <= 140:
        raise ServiceError("The pattern must have 1 to 140 characters.")
    if not 0 <= priority <= 10_000:
        raise ServiceError("The priority must be between 0 and 10000.")
    owned_category(db, user_id, category_id)
    return pattern


def owned_rule(db: Session, user_id: int, rule_id: int) -> CategorisationRule:
    rule = db.get(CategorisationRule, rule_id)
    if rule is None or rule.user_id != user_id:
        raise NotFoundError("rule")
    return rule


def create_rule(
    db: Session,
    user_id: int,
    *,
    field: RuleField,
    match_type: MatchType,
    pattern: str,
    category_id: int,
    priority: int = 100,
) -> CategorisationRule:
    pattern = _checked_rule_values(db, user_id, pattern, priority, category_id)
    rule = CategorisationRule(
        user_id=user_id,
        field=field,
        match_type=match_type,
        pattern=pattern,
        category_id=category_id,
        priority=priority,
    )
    db.add(rule)
    db.flush()
    return rule


def update_rule(
    db: Session,
    rule: CategorisationRule,
    *,
    field: RuleField,
    match_type: MatchType,
    pattern: str,
    category_id: int,
    priority: int,
) -> None:
    """Change all values of a rule.

    The change applies to the next imports. Transactions that have a category
    already keep it. "Apply the rules to uncategorised transactions" applies the
    changed rule to the uncategorised transactions.
    """
    pattern = _checked_rule_values(db, rule.user_id, pattern, priority, category_id)
    rule.field = field
    rule.match_type = match_type
    rule.pattern = pattern
    rule.category_id = category_id
    rule.priority = priority
    db.flush()


def apply_to_uncategorised(db: Session, user_id: int) -> int:
    """Find transfers and apply the rules and the history to the uncategorised transactions.

    The matching of transfers (:mod:`feljosit.services.transfers`) applies first, to
    all dates. Return the number of changed transactions.
    """
    matched = 2 * len(match_transfers(db, user_id))
    categoriser = Categoriser(db, user_id)
    txs: Iterable[Transaction] = db.scalars(
        select(Transaction)
        .join(Account, Transaction.account_id == Account.id)
        .where(
            Transaction.user_id == user_id,
            Transaction.category_id.is_(None),
            Transaction.is_transfer.is_(False),
            Account.on_budget.is_(True),
        )
        .order_by(Transaction.booking_date)
    )
    changed = 0
    for tx in txs:
        if categoriser.is_transfer(tx.account, tx.counterparty_iban):
            tx.is_transfer = True
            changed += 1
            continue
        suggestion = categoriser.suggest(tx.counterparty, tx.counterparty_iban, tx.description)
        if suggestion is not None:
            tx.category_id = suggestion
            changed += 1
    return matched + changed
