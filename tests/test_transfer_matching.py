"""Transfers from two matching transactions on two accounts."""

from datetime import date

import pytest
from fastapi import FastAPI
from fastapi.testclient import TestClient
from sqlalchemy import select
from sqlalchemy.orm import Session

from feljosit.models import (
    Account,
    AccountType,
    Category,
    MatchType,
    RuleField,
    Transaction,
    TransferMethod,
    TransferPair,
    TransferStatus,
    User,
)
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


# Transfer pairs (table transfer_pairs).


def _pairs(db: Session) -> list[TransferPair]:
    return list(db.scalars(select(TransferPair).order_by(TransferPair.id)))


def test_match_records_the_pair(db: Session, user: User) -> None:
    private, card = _account(db, user, "Private"), _account(db, user, "Card")
    sent = _tx(db, private, date(2026, 10, 1), -3000)
    received = _tx(db, card, date(2026, 10, 2), 3000)
    transfers.match_transfers(db, user.id)
    [pair] = _pairs(db)
    assert (pair.outflow_id, pair.inflow_id) == (sent.id, received.id)
    assert (pair.method, pair.status) == (TransferMethod.MATCH, TransferStatus.ACTIVE)
    found = transfers.partners(db, [sent, received])
    assert found[sent.id].tx is received and found[received.id].tx is sent


@pytest.mark.parametrize("category", [None, "Groceries"])
def test_not_a_transfer_changes_both_and_is_remembered(
    db: Session, user: User, category: str | None
) -> None:
    private, card = _account(db, user, "Private"), _account(db, user, "Card")
    sent = _tx(db, private, date(2026, 10, 1), -3000)
    received = _tx(db, card, date(2026, 10, 2), 3000)
    transfers.match_transfers(db, user.id)

    category_id = _category(db, user, category).id if category else None
    partner = categorise.set_category(db, sent, category_id)
    assert partner is received
    assert not sent.is_transfer and not received.is_transfer
    assert sent.category_id == category_id and received.category_id is None
    [pair] = _pairs(db)
    assert pair.status == TransferStatus.REJECTED
    # The matching does not make the rejected pair again.
    if category is None:
        assert transfers.match_transfers(db, user.id) == []
        assert categorise.apply_to_uncategorised(db, user.id) == 0
    assert not received.is_transfer
    assert len(_pairs(db)) == 1


def test_rejected_pair_allows_a_different_partner(db: Session, user: User) -> None:
    private, card, savings = (_account(db, user, n) for n in ("Private", "Card", "Savings"))
    sent = _tx(db, private, date(2026, 10, 1), -3000)
    near = _tx(db, card, date(2026, 10, 2), 3000)
    far = _tx(db, savings, date(2026, 10, 4), 3000)
    assert transfers.match_transfers(db, user.id) == [(sent, near)]
    categorise.set_category(db, near, None)
    assert transfers.match_transfers(db, user.id) == [(sent, far)]


def test_uncategorised_ends_a_transfer_without_pair(db: Session, user: User) -> None:
    private = _account(db, user, "Private")
    tx = _tx(db, private, date(2026, 10, 1), -3000)
    tx.is_transfer = True  # Selected manually as "Transfer".
    assert categorise.set_category(db, tx, None) is None
    assert not tx.is_transfer


def test_delete_ends_a_matched_pair(db: Session, user: User) -> None:
    private, card = _account(db, user, "Private"), _account(db, user, "Card")
    sent = _tx(db, private, date(2026, 10, 1), -3000)
    received = _tx(db, card, date(2026, 10, 2), 3000)
    transfers.match_transfers(db, user.id)
    transfers.before_delete(db, sent)
    db.delete(sent)
    db.flush()
    assert not received.is_transfer
    assert _pairs(db) == []


def _iban_account(db: Session, user: User, name: str, iban: str) -> Account:
    return accounts.create_account(
        db,
        user.id,
        name=name,
        account_type=AccountType.CHECKING,
        currency="CHF",
        budget_currency="CHF",
        iban=iban,
    )


def test_import_records_iban_pairs(db: Session, user: User) -> None:
    private = _iban_account(db, user, "Private", "CH8900235000012345678")
    savings = _iban_account(db, user, "Savings", "CH0300700110000123456")
    import_statement_file(
        db, user.id, filename="p.xml", data=fixture_bytes("camt053_v04_private.xml")
    )
    # Only one side is imported: a transfer without a pair.
    assert _pairs(db) == []
    import_statement_file(
        db, user.id, filename="s.xml", data=fixture_bytes("camt053_v08_savings.xml")
    )
    [pair] = _pairs(db)
    assert pair.method == TransferMethod.IBAN
    assert (pair.outflow.account_id, pair.inflow.account_id) == (private.id, savings.id)
    assert pair.outflow.amount_minor == -50000 and pair.inflow.amount_minor == 50000
    # "Not a transfer" also works for a pair from the IBAN.
    categorise.set_category(db, pair.outflow, None)
    assert not pair.inflow.is_transfer


def test_transaction_list_shows_partner_and_filters(
    client: tuple[TestClient, str], app: FastAPI
) -> None:
    http, token = client
    db = app.state.session_factory()
    user = db.scalar(select(User).where(User.username == "anna"))
    assert user is not None
    private, card = _account(db, user, "Private"), _account(db, user, "Card")
    _tx(db, private, date(2026, 10, 1), -3000)
    received_id = _tx(db, card, date(2026, 10, 2), 3000).id
    _tx(db, private, date(2026, 10, 3), -999)
    transfers.match_transfers(db, user.id)
    db.commit()
    db.close()

    page = http.get("/transactions").text
    assert "⇄" in page and "Card, 02.10.2026" in page and "found from the amount" in page
    page = http.get("/transactions?matched=true").text
    assert "2 transactions." in page
    # "Uncategorised" on one side ends the pair and tells the user.
    response = http.post(
        f"/transactions/{received_id}/category",
        data={"csrf_token": token, "category_id": "", "return_to": "/transactions?matched=true"},
    )
    assert response.status_code == 200
    assert "are not a transfer any more" in response.text
    assert "0 transactions." in response.text  # The filter stays.
