"""IBAN validation (ISO 13616)."""

from __future__ import annotations

import re

_IBAN_RE = re.compile(r"^[A-Z]{2}\d{2}[A-Z0-9]{11,30}$")

#: Lengths for countries that Swiss users see most frequently.
_LENGTHS = {"CH": 21, "LI": 21, "DE": 22, "AT": 20, "FR": 27, "IT": 27}


def normalise_iban(value: str) -> str:
    """Remove spaces and change the IBAN to upper case."""
    return re.sub(r"\s+", "", value).upper()


def is_valid_iban(value: str) -> bool:
    """Return true if the value is a valid IBAN (format, length and check digits)."""
    iban = normalise_iban(value)
    if not _IBAN_RE.fullmatch(iban):
        return False
    expected = _LENGTHS.get(iban[:2])
    if expected is not None and len(iban) != expected:
        return False
    rearranged = iban[4:] + iban[:4]
    digits = "".join(str(int(ch, 36)) for ch in rearranged)
    return int(digits) % 97 == 1


def format_iban(value: str) -> str:
    """Format an IBAN in groups of four characters."""
    iban = normalise_iban(value)
    return " ".join(iban[i : i + 4] for i in range(0, len(iban), 4))


def is_qr_iban(value: str) -> bool:
    """Return true if the value is a Swiss QR-IBAN (IID 30000 to 31999)."""
    iban = normalise_iban(value)
    if iban[:2] not in {"CH", "LI"} or len(iban) != 21:
        return False
    iid = int(iban[4:9]) if iban[4:9].isdigit() else -1
    return 30000 <= iid <= 31999
