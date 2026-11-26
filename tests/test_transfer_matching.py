"""Transfers from two matching transactions on two accounts."""

from datetime import date

import pytest
from sqlalchemy import select
from sqlalchemy.orm import Session

from feljosit.models import Account, AccountType, Category, MatchType, RuleField, Transaction, User
from feljosit.services import accounts, categorise, transfers
from feljosit.services.importer import import_statement_file
from tests.conftest import fixture_bytes


def _account(
    db: Session,
    user: User,
    name: str,
    *,
    account_type: AccountType = AccountType.CHECKING,
    currency: str = "CHF",
) -> Account:
    return accounts.create_account(
        db,
        user.id,
        name=name,
        account_type=account_type,
        currency=currency,
        budget_currency="CHF",
        on_budget=None if currency == "CHF" else False,
    )


def _tx(
    db: Session, account: Account, on: date, amount: int, category_id: int | None = None
) -> Transaction:
    return accounts.add_manual_transaction(
        db, account, booking_date=on, amount_minor=amount, counterparty="x", category_id=category_id
    )


def _category(db: Session, user: User, name: str) -> Category:
    category = db.scalar(select(Category).where(Category.user_id == user.id, Category.name == name))
    assert category is not None
    return category


def test_matching_pair_is_a_transfer(db: Session, user: User) -> None:
    private, card = _account(db, user, "Private"), _account(db, user, "Card")
    sent = _tx(db, private, date(2026, 10, 30), -45000)
    received = _tx(db, card, date(2026, 11, 2), 45000)  # 3 days later: a weekend.
    other = _tx(db, card, date(2026, 11, 2), 12000)
    pairs = transfers.match_transfers(db, user.id)
    assert pairs == [(sent, received)]
    assert sent.is_transfer and received.is_transfer
    assert not other.is_transfer


@pytest.mark.parametrize(
    "case",
    [
        "too_far",
        "same_account",
        "budget_and_tracking",
        "categorised",
        "other_currency",
        "same_sign",
    ],
)
def test_no_transfer(db: Session, user: User, case: str) -> None:
    private, card = _account(db, user, "Private"), _account(db, user, "Card")
    on = date(2026, 10, 1)
    if case == "too_far":
        _tx(db, private, on, -1000)
        _tx(db, card, date(2026, 10, 6), 1000)
    elif case == "same_account":
        _tx(db, private, on, -1000)
        _tx(db, private, on, 1000)
    elif case == "budget_and_tracking":
        pillar = _account(db, user, "3a", account_type=AccountType.PILLAR_3A)
        _tx(db, private, on, -1000)
        _tx(db, pillar, on, 1000)
    elif case == "categorised":
        _tx(db, private, on, -1000, _category(db, user, "Groceries").id)
        _tx(db, card, on, 1000)
    elif case == "other_currency":
        eur = _account(db, user, "EUR", currency="EUR")
        usd = _account(db, user, "USD", currency="USD")
        _tx(db, eur, on, -1000)
        _tx(db, usd, on, 1000)
    elif case == "same_sign":
        _tx(db, private, on, -1000)
        _tx(db, card, on, -1000)
    assert transfers.match_transfers(db, user.id) == []
    assert not any(t.is_transfer for t in db.scalars(select(Transaction)))


def test_ambiguous_candidates_are_not_matched(db: Session, user: User) -> None:
    private, card, savings = (_account(db, user, n) for n in ("Private", "Card", "Savings"))
    on = date(2026, 10, 10)
    _tx(db, private, on, -5000)
    # Two inflows with the same distance: it is not clear which one is the transfer.
    _tx(db, card, date(2026, 10, 11), 5000)
    _tx(db, savings, date(2026, 10, 9), 5000)
    assert transfers.match_transfers(db, user.id) == []


def test_competing_outflows_are_not_matched(db: Session, user: User) -> None:
    private, card, savings = (_account(db, user, n) for n in ("Private", "Card", "Savings"))
    _tx(db, private, date(2026, 10, 10), -5000)
    _tx(db, savings, date(2026, 10, 12), -5000)
    _tx(db, card, date(2026, 10, 11), 5000)  # The same distance to both outflows.
    assert transfers.match_transfers(db, user.id) == []


def test_nearest_candidate_wins(db: Session, user: User) -> None:
    private, card, savings = (_account(db, user, n) for n in ("Private", "Card", "Savings"))
    sent = _tx(db, private, date(2026, 10, 10), -5000)
    near = _tx(db, card, date(2026, 10, 11), 5000)
    far = _tx(db, savings, date(2026, 10, 13), 5000)
    assert transfers.match_transfers(db, user.id) == [(sent, near)]
    assert not far.is_transfer


def test_starting_balances_are_not_matched(db: Session, user: User) -> None:
    accounts.create_account(
        db,
        user.id,
        name="Private",
        account_type=AccountType.CHECKING,
        currency="CHF",
        budget_currency="CHF",
        starting_balance_minor=100000,
        starting_date=date(2026, 1, 1),
    )
    card = _account(db, user, "Card")
    _tx(db, card, date(2026, 1, 1), -100000)
    assert transfers.match_transfers(db, user.id) == []


def test_date_range(db: Session, user: User) -> None:
    private, card = _account(db, user, "Private"), _account(db, user, "Card")
    _tx(db, private, date(2026, 1, 10), -1000)
    _tx(db, card, date(2026, 1, 10), 1000)
    old = transfers.match_transfers(db, user.id, date(2026, 6, 1), date(2026, 6, 30))
    assert old == []
    # A partner can be some days before the range.
    sent = _tx(db, private, date(2026, 5, 30), -2000)
    received = _tx(db, card, date(2026, 6, 1), 2000)
    assert transfers.match_transfers(db, user.id, date(2026, 6, 1), date(2026, 6, 30)) == [
        (sent, received)
    ]


def test_apply_rules_finds_transfers(db: Session, user: User) -> None:
    private, card = _account(db, user, "Private"), _account(db, user, "Card")
    sent = _tx(db, private, date(2026, 10, 1), -3000)
    _tx(db, card, date(2026, 10, 2), 3000)
    assert categorise.apply_to_uncategorised(db, user.id) == 2
    assert sent.is_transfer


def test_import_matches_before_rules(db: Session, user: User) -> None:
    """A payment to an account without IBAN in the statement is a transfer.

    The matching applies before the rules: the rule does not give a category.
    """
    accounts.create_account(
        db,
        user.id,
        name="Private",
        account_type=AccountType.CHECKING,
        currency="CHF",
        budget_currency="CHF",
        iban="CH8900235000012345678",
    )
    card = _account(db, user, "Card")
    # The statement has a payment of 412.35 on 05.10.2026.
    received = _tx(db, card, date(2026, 10, 6), 41235)
    categorise.create_rule(
        db,
        user.id,
        field=RuleField.COUNTERPARTY,
        match_type=MatchType.CONTAINS,
        pattern="Krankenkasse",
        category_id=_category(db, user, "Health insurance (KVG/VVG)").id,
    )
    result = import_statement_file(
        db, user.id, filename="s.xml", data=fixture_bytes("camt053_v04_private.xml")
    )
    [stmt] = result.statements
    assert stmt.matched_transfer_count == 1
    assert stmt.transfer_count == 1
    payment = db.scalar(
        select(Transaction).where(Transaction.counterparty == "Krankenkasse Beispiel")
    )
    assert payment is not None
    assert payment.is_transfer and payment.category_id is None
    assert received.is_transfer
    # The other transactions get categories as before.
    assert stmt.categorised_count == 0  # No history and no other rule.
