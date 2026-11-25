import pytest

from feljosit.iban import format_iban, is_qr_iban, is_valid_iban, normalise_iban


@pytest.mark.parametrize(
    "iban",
    ["CH9300762011623852957", "ch93 0076 2011 6238 5295 7", "DE89370400440532013000"],
)
def test_valid(iban: str) -> None:
    assert is_valid_iban(iban)


@pytest.mark.parametrize(
    "iban",
    ["CH9300762011623852958", "CH93007620116238529", "XX", "", "CH93-0076-2011-6238-5295-7"],
)
def test_invalid(iban: str) -> None:
    assert not is_valid_iban(iban)


def test_format_and_normalise() -> None:
    assert normalise_iban(" ch93 0076 2011 6238 5295 7 ") == "CH9300762011623852957"
    assert format_iban("CH9300762011623852957") == "CH93 0076 2011 6238 5295 7"


def test_qr_iban() -> None:
    assert is_qr_iban("CH0430000001234567890")
    assert not is_qr_iban("CH9300762011623852957")
