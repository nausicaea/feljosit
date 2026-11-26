"""The counterparty account number in the transaction list."""

from fastapi import FastAPI
from fastapi.testclient import TestClient
from sqlalchemy import select

from feljosit.models import Transaction
from tests.conftest import fixture_bytes

Client = tuple[TestClient, str]
MUSTER_IBAN = "CH93 0076 2011 6238 5295 7"


def _import(http: TestClient, token: str) -> None:
    response = http.post(
        "/import",
        data={"csrf_token": token, "account_id": ""},
        files={"statement": ("s.xml", fixture_bytes("camt053_v04_private.xml"), "text/xml")},
    )
    assert "7 new transactions" in response.text


def test_account_numbers_are_hidden_by_default(client: Client) -> None:
    http, token = client
    _import(http, token)
    page = http.get("/transactions").text
    assert "Muster AG" in page
    assert MUSTER_IBAN not in page
    assert 'name="iban" value="true"' in page
    assert 'name="iban" value="true" checked' not in page


def test_show_account_numbers(client: Client) -> None:
    http, token = client
    _import(http, token)
    page = http.get("/transactions?iban=true").text
    assert f"IBAN {MUSTER_IBAN}" in page
    assert 'name="iban" value="true" checked' in page
    # The option is not a filter: there is no "Remove the filters" link.
    assert "Remove the filters" not in page
    # With a filter, the link removes the filter and keeps the option.
    page = http.get("/transactions?iban=true&q=muster").text
    assert 'href="/transactions?iban=true">Remove the filters' in page
    # The forms of each row go back to the list with the option.
    assert 'name="return_to" value="/transactions?q=muster&amp;iban=true&amp;page=1"' in page


def test_option_stays_after_a_change(client: Client, app: FastAPI) -> None:
    http, token = client
    _import(http, token)
    db = app.state.session_factory()
    tx = db.scalar(select(Transaction).where(Transaction.counterparty == "Muster AG"))
    assert tx is not None
    db.close()
    response = http.post(
        f"/transactions/{tx.id}/memo",
        data={"csrf_token": token, "memo": "Bonus", "return_to": "/transactions?iban=true&page=1"},
        follow_redirects=False,
    )
    assert response.status_code == 303
    assert response.headers["location"] == "/transactions?iban=true&page=1"


def test_search_for_an_account_number(client: Client) -> None:
    http, token = client
    _import(http, token)
    for text in ["CH93 0076", "ch9300762011", "2011 6238"]:
        page = http.get("/transactions", params={"q": text}).text
        assert "1 transactions." in page, text
        assert "Muster AG" in page
    # A short text or a text without a digit does not search the IBANs. Else "ch"
    # finds all transactions with a Swiss IBAN.
    page = http.get("/transactions", params={"q": "ch"}).text
    assert "Krankenkasse Beispiel" not in page
    # Special characters of LIKE are not wildcards.
    page = http.get("/transactions", params={"q": "CH9%"}).text
    assert "0 transactions." in page
