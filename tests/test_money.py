from decimal import Decimal

import pytest

from feljosit.money import MoneyError, format_minor, parse_user_amount, to_minor, validate_currency


@pytest.mark.parametrize(
    ("text", "expected"),
    [
        ("0", 0),
        ("12", 1200),
        ("12.5", 1250),
        ("-12.05", -1205),
        ("1'234.50", 123450),
        ("1’234.50", 123450),
        ("1 234,50", 123450),
        ("+7.10", 710),
    ],
)
def test_parse_user_amount(text: str, expected: int) -> None:
    assert parse_user_amount(text, "CHF") == expected


@pytest.mark.parametrize("text", ["", "abc", "1.234", "1e5", "NaN", "--1", "1.2.3", "1,2,3"])
def test_parse_user_amount_rejects_invalid(text: str) -> None:
    with pytest.raises(MoneyError):
        parse_user_amount(text, "CHF")


def test_to_minor_respects_currency_exponent() -> None:
    assert to_minor(Decimal("1500"), "JPY") == 1500
    assert to_minor("1.234", "KWD") == 1234
    with pytest.raises(MoneyError):
        to_minor("1.5", "JPY")


@pytest.mark.parametrize(
    ("amount", "expected"),
    [(0, "0.00"), (5, "0.05"), (-123456789, "-1'234'567.89"), (100000, "1'000.00")],
)
def test_format_minor(amount: int, expected: str) -> None:
    assert format_minor(amount, "CHF") == expected


def test_format_with_currency() -> None:
    assert format_minor(1250, "CHF", with_currency=True) == "CHF 12.50"


def test_validate_currency() -> None:
    assert validate_currency(" chf ") == "CHF"
    with pytest.raises(MoneyError):
        validate_currency("CH")
