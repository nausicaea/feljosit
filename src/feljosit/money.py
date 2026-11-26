"""Money values.

The application keeps all amounts as integers in the minor unit of the currency
(for CHF: Rappen). This prevents rounding errors from binary floating point.
"""

from __future__ import annotations

import re
from decimal import Decimal, InvalidOperation

#: ISO 4217 minor unit exponents. The default is 2.
_EXPONENTS: dict[str, int] = {
    "BHD": 3,
    "CLP": 0,
    "ISK": 0,
    "JOD": 3,
    "JPY": 0,
    "KRW": 0,
    "KWD": 3,
    "OMR": 3,
    "TND": 3,
}

_CURRENCY_RE = re.compile(r"^[A-Z]{3}$")
_USER_AMOUNT_RE = re.compile(r"^[+-]?\d{1,12}(\.\d{1,3})?$")


class MoneyError(ValueError):
    """The money value is not valid."""


def validate_currency(code: str) -> str:
    """Return the currency code in upper case. Raise :class:`MoneyError` if it is not valid."""
    code = code.strip().upper()
    if not _CURRENCY_RE.fullmatch(code):
        raise MoneyError(f"Currency code is not valid: {code!r}")
    return code


def exponent(currency: str) -> int:
    """Return the number of decimal places for the currency."""
    return _EXPONENTS.get(currency.upper(), 2)


def to_minor(value: Decimal | str, currency: str) -> int:
    """Convert a decimal amount to minor units.

    Raise :class:`MoneyError` if the amount has more decimal places than the currency permits.
    """
    try:
        dec = value if isinstance(value, Decimal) else Decimal(value)
    except InvalidOperation as exc:
        raise MoneyError(f"Amount is not a number: {value!r}") from exc
    if not dec.is_finite():
        raise MoneyError("Amount is not finite")
    exp = exponent(currency)
    scaled = dec.scaleb(exp)
    if scaled != scaled.to_integral_value():
        raise MoneyError(f"Amount {value} has too many decimal places for {currency}")
    return int(scaled)


def parse_user_decimal(text: str) -> Decimal:
    """Parse an amount that a person typed, for example ``1'234.50`` or ``-12``.

    The function accepts Swiss thousands separators (apostrophe, right single quote,
    space) and a comma as the decimal separator. The result has no currency.
    """
    cleaned = text.strip()
    for sep in ("'", "’", " ", " ", "_"):
        cleaned = cleaned.replace(sep, "")
    if cleaned.count(",") == 1 and "." not in cleaned:
        cleaned = cleaned.replace(",", ".")
    if not _USER_AMOUNT_RE.fullmatch(cleaned):
        raise MoneyError(f"Amount is not valid: {text!r}")
    return Decimal(cleaned)


def parse_user_amount(text: str, currency: str) -> int:
    """Parse an amount that a person typed (see :func:`parse_user_decimal`) to minor units."""
    return to_minor(parse_user_decimal(text), currency)


def format_minor(amount: int, currency: str, *, with_currency: bool = False) -> str:
    """Format minor units in the Swiss style, for example ``-1'234.50``."""
    exp = exponent(currency)
    sign = "-" if amount < 0 else ""
    absolute = abs(amount)
    major, minor = divmod(absolute, 10**exp) if exp else (absolute, 0)
    grouped = f"{major:,}".replace(",", "'")
    text = f"{sign}{grouped}.{minor:0{exp}d}" if exp else f"{sign}{grouped}"
    return f"{currency} {text}" if with_currency else text


def minor_to_decimal(amount: int, currency: str) -> Decimal:
    """Convert minor units back to a :class:`~decimal.Decimal`."""
    return Decimal(amount).scaleb(-exponent(currency))
