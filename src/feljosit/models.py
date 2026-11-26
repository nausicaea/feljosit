"""Database models (SQLAlchemy 2)."""

from __future__ import annotations

import enum
from datetime import UTC, date, datetime

from sqlalchemy import (
    Boolean,
    Date,
    DateTime,
    Enum,
    ForeignKey,
    Index,
    Integer,
    MetaData,
    String,
    Text,
    UniqueConstraint,
)
from sqlalchemy.engine import Dialect
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column, relationship
from sqlalchemy.types import TypeDecorator

NAMING_CONVENTION = {
    "ix": "ix_%(column_0_label)s",
    "uq": "uq_%(table_name)s_%(column_0_name)s",
    "ck": "ck_%(table_name)s_%(constraint_name)s",
    "fk": "fk_%(table_name)s_%(column_0_name)s_%(referred_table_name)s",
    "pk": "pk_%(table_name)s",
}


def utcnow() -> datetime:
    return datetime.now(UTC)


class UTCDateTime(TypeDecorator[datetime]):
    """Store a timezone-aware datetime as UTC. SQLite does not keep the timezone."""

    impl = DateTime
    cache_ok = True

    def process_bind_param(self, value: datetime | None, dialect: Dialect) -> datetime | None:
        if value is None:
            return None
        if value.tzinfo is None:
            raise ValueError("Naive datetime values are not permitted")
        return value.astimezone(UTC).replace(tzinfo=None)

    def process_result_value(self, value: datetime | None, dialect: Dialect) -> datetime | None:
        return None if value is None else value.replace(tzinfo=UTC)


def _enum(cls: type[enum.StrEnum], length: int) -> Enum:
    """Store the enum value (not the name) as a string column."""
    return Enum(
        cls,
        native_enum=False,
        length=length,
        values_callable=lambda members: [m.value for m in members],
        validate_strings=True,
    )


class Base(DeclarativeBase):
    metadata = MetaData(naming_convention=NAMING_CONVENTION)


class AccountType(enum.StrEnum):
    CHECKING = "checking"
    SAVINGS = "savings"
    CREDIT_CARD = "credit_card"
    CASH = "cash"
    PILLAR_3A = "pillar_3a"
    INVESTMENT = "investment"
    LOAN = "loan"

    @property
    def label(self) -> str:
        return _ACCOUNT_LABELS[self]

    @property
    def on_budget_default(self) -> bool:
        return self in {
            AccountType.CHECKING,
            AccountType.SAVINGS,
            AccountType.CREDIT_CARD,
            AccountType.CASH,
        }


_ACCOUNT_LABELS = {
    AccountType.CHECKING: "Private account",
    AccountType.SAVINGS: "Savings account",
    AccountType.CREDIT_CARD: "Credit card",
    AccountType.CASH: "Cash",
    AccountType.PILLAR_3A: "Pillar 3a",
    AccountType.INVESTMENT: "Investment (custody account)",
    AccountType.LOAN: "Loan or mortgage",
}


class CategoryKind(enum.StrEnum):
    EXPENSE = "expense"
    INCOME = "income"


class RuleField(enum.StrEnum):
    COUNTERPARTY = "counterparty"
    COUNTERPARTY_IBAN = "counterparty_iban"
    DESCRIPTION = "description"


class MatchType(enum.StrEnum):
    CONTAINS = "contains"
    EQUALS = "equals"


class TransactionSource(enum.StrEnum):
    CAMT053 = "camt053"
    MANUAL = "manual"
    STARTING_BALANCE = "starting_balance"


class TransferMethod(enum.StrEnum):
    """How the application found a transfer pair."""

    #: The counterparty IBAN is the IBAN of the other account.
    IBAN = "iban"
    #: Two matching transactions (see ADR-0016).
    MATCH = "match"


class TransferStatus(enum.StrEnum):
    ACTIVE = "active"
    #: The user said that the two transactions are not a transfer. The matching does
    #: not make this pair again.
    REJECTED = "rejected"


class User(Base):
    __tablename__ = "users"

    id: Mapped[int] = mapped_column(primary_key=True)
    username: Mapped[str] = mapped_column(String(64), unique=True)
    password_hash: Mapped[str] = mapped_column(String(255))
    failed_logins: Mapped[int] = mapped_column(Integer, default=0)
    locked_until: Mapped[datetime | None] = mapped_column(UTCDateTime())
    created_at: Mapped[datetime] = mapped_column(UTCDateTime(), default=utcnow)


class Session(Base):
    """A login session. The table keeps only a SHA-256 hash of the session token."""

    __tablename__ = "sessions"

    token_hash: Mapped[str] = mapped_column(String(64), primary_key=True)
    user_id: Mapped[int] = mapped_column(ForeignKey("users.id", ondelete="CASCADE"), index=True)
    csrf_token: Mapped[str] = mapped_column(String(64))
    #: A message for the next page ("flash message"), for example after a redirect.
    flash_message: Mapped[str | None] = mapped_column(String(500))
    flash_is_error: Mapped[bool] = mapped_column(Boolean, default=False)
    created_at: Mapped[datetime] = mapped_column(UTCDateTime(), default=utcnow)
    expires_at: Mapped[datetime] = mapped_column(UTCDateTime())

    user: Mapped[User] = relationship()


class Account(Base):
    __tablename__ = "accounts"
    __table_args__ = (UniqueConstraint("user_id", "iban"),)

    id: Mapped[int] = mapped_column(primary_key=True)
    user_id: Mapped[int] = mapped_column(ForeignKey("users.id", ondelete="CASCADE"), index=True)
    name: Mapped[str] = mapped_column(String(100))
    type: Mapped[AccountType] = mapped_column(_enum(AccountType, 20))
    on_budget: Mapped[bool] = mapped_column(Boolean, default=True)
    currency: Mapped[str] = mapped_column(String(3), default="CHF")
    iban: Mapped[str | None] = mapped_column(String(34))
    closed: Mapped[bool] = mapped_column(Boolean, default=False)
    created_at: Mapped[datetime] = mapped_column(UTCDateTime(), default=utcnow)


class CategoryGroup(Base):
    __tablename__ = "category_groups"

    id: Mapped[int] = mapped_column(primary_key=True)
    user_id: Mapped[int] = mapped_column(ForeignKey("users.id", ondelete="CASCADE"), index=True)
    name: Mapped[str] = mapped_column(String(100))
    sort_order: Mapped[int] = mapped_column(Integer, default=0)

    categories: Mapped[list[Category]] = relationship(
        back_populates="group", order_by="Category.sort_order, Category.name"
    )


class Category(Base):
    __tablename__ = "categories"

    id: Mapped[int] = mapped_column(primary_key=True)
    user_id: Mapped[int] = mapped_column(ForeignKey("users.id", ondelete="CASCADE"), index=True)
    group_id: Mapped[int] = mapped_column(ForeignKey("category_groups.id", ondelete="CASCADE"))
    name: Mapped[str] = mapped_column(String(100))
    kind: Mapped[CategoryKind] = mapped_column(
        _enum(CategoryKind, 10), default=CategoryKind.EXPENSE
    )
    hidden: Mapped[bool] = mapped_column(Boolean, default=False)
    sort_order: Mapped[int] = mapped_column(Integer, default=0)

    group: Mapped[CategoryGroup] = relationship(back_populates="categories")


class ImportBatch(Base):
    """One imported file. A file can contain statements for more than one account."""

    __tablename__ = "import_batches"
    __table_args__ = (UniqueConstraint("user_id", "file_sha256"),)

    id: Mapped[int] = mapped_column(primary_key=True)
    user_id: Mapped[int] = mapped_column(ForeignKey("users.id", ondelete="CASCADE"), index=True)
    filename: Mapped[str] = mapped_column(String(255))
    file_sha256: Mapped[str] = mapped_column(String(64))
    message_id: Mapped[str | None] = mapped_column(String(35))
    new_count: Mapped[int] = mapped_column(Integer, default=0)
    duplicate_count: Mapped[int] = mapped_column(Integer, default=0)
    imported_at: Mapped[datetime] = mapped_column(UTCDateTime(), default=utcnow)


class Transaction(Base):
    __tablename__ = "transactions"
    __table_args__ = (
        UniqueConstraint("account_id", "fingerprint"),
        Index("ix_transactions_user_date", "user_id", "booking_date"),
    )

    id: Mapped[int] = mapped_column(primary_key=True)
    user_id: Mapped[int] = mapped_column(ForeignKey("users.id", ondelete="CASCADE"))
    account_id: Mapped[int] = mapped_column(
        ForeignKey("accounts.id", ondelete="CASCADE"), index=True
    )
    booking_date: Mapped[date] = mapped_column(Date)
    value_date: Mapped[date | None] = mapped_column(Date)
    #: Signed amount in minor units. A negative value is an outflow.
    amount_minor: Mapped[int] = mapped_column(Integer)
    currency: Mapped[str] = mapped_column(String(3))
    counterparty: Mapped[str] = mapped_column(String(140), default="")
    counterparty_iban: Mapped[str | None] = mapped_column(String(34))
    description: Mapped[str] = mapped_column(Text, default="")
    reference: Mapped[str | None] = mapped_column(String(35))
    bank_reference: Mapped[str | None] = mapped_column(String(35))
    memo: Mapped[str] = mapped_column(String(500), default="")
    category_id: Mapped[int | None] = mapped_column(
        ForeignKey("categories.id", ondelete="SET NULL"), index=True
    )
    #: A transfer between two accounts of the same user. It has no category.
    is_transfer: Mapped[bool] = mapped_column(Boolean, default=False)
    source: Mapped[TransactionSource] = mapped_column(_enum(TransactionSource, 20))
    fingerprint: Mapped[str] = mapped_column(String(80))
    import_batch_id: Mapped[int | None] = mapped_column(
        ForeignKey("import_batches.id", ondelete="SET NULL")
    )
    created_at: Mapped[datetime] = mapped_column(UTCDateTime(), default=utcnow)

    account: Mapped[Account] = relationship()
    category: Mapped[Category | None] = relationship()


class TransferPair(Base):
    """The two transactions of one transfer: an outflow and an inflow on two accounts.

    A transaction is in at most one active pair. The services make sure of this. A
    transfer can also have no pair, for example if only one side is imported.
    """

    __tablename__ = "transfer_pairs"
    __table_args__ = (UniqueConstraint("outflow_id", "inflow_id"),)

    id: Mapped[int] = mapped_column(primary_key=True)
    user_id: Mapped[int] = mapped_column(ForeignKey("users.id", ondelete="CASCADE"), index=True)
    outflow_id: Mapped[int] = mapped_column(ForeignKey("transactions.id", ondelete="CASCADE"))
    inflow_id: Mapped[int] = mapped_column(
        ForeignKey("transactions.id", ondelete="CASCADE"), index=True
    )
    method: Mapped[TransferMethod] = mapped_column(_enum(TransferMethod, 10))
    status: Mapped[TransferStatus] = mapped_column(
        _enum(TransferStatus, 10), default=TransferStatus.ACTIVE
    )
    created_at: Mapped[datetime] = mapped_column(UTCDateTime(), default=utcnow)

    outflow: Mapped[Transaction] = relationship(foreign_keys=[outflow_id])
    inflow: Mapped[Transaction] = relationship(foreign_keys=[inflow_id])


class BudgetAssignment(Base):
    """The amount that a user assigns to a category for one month."""

    __tablename__ = "budget_assignments"
    __table_args__ = (UniqueConstraint("category_id", "month"),)

    id: Mapped[int] = mapped_column(primary_key=True)
    user_id: Mapped[int] = mapped_column(ForeignKey("users.id", ondelete="CASCADE"), index=True)
    category_id: Mapped[int] = mapped_column(ForeignKey("categories.id", ondelete="CASCADE"))
    #: The first day of the month.
    month: Mapped[date] = mapped_column(Date)
    amount_minor: Mapped[int] = mapped_column(Integer, default=0)


class CategorisationRule(Base):
    __tablename__ = "categorisation_rules"

    id: Mapped[int] = mapped_column(primary_key=True)
    user_id: Mapped[int] = mapped_column(ForeignKey("users.id", ondelete="CASCADE"), index=True)
    field: Mapped[RuleField] = mapped_column(_enum(RuleField, 20))
    match_type: Mapped[MatchType] = mapped_column(_enum(MatchType, 10))
    pattern: Mapped[str] = mapped_column(String(140))
    category_id: Mapped[int] = mapped_column(ForeignKey("categories.id", ondelete="CASCADE"))
    priority: Mapped[int] = mapped_column(Integer, default=100)

    category: Mapped[Category] = relationship()
