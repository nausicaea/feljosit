"""Default categories for a new user. They are typical for a Swiss household."""

from __future__ import annotations

from sqlalchemy.orm import Session

from feljosit.models import Category, CategoryGroup, CategoryKind

DEFAULT_CATEGORIES: tuple[tuple[str, CategoryKind, tuple[str, ...]], ...] = (
    ("Income", CategoryKind.INCOME, ("Salary", "13th salary and bonus", "Other income")),
    (
        "Fixed costs",
        CategoryKind.EXPENSE,
        (
            "Rent and service charges",
            "Health insurance (KVG/VVG)",
            "Electricity and utilities",
            "Phone and internet",
            "Serafe fee",
            "Public transport (GA/Half Fare)",
        ),
    ),
    (
        "Taxes and insurance",
        CategoryKind.EXPENSE,
        ("Taxes", "Household and liability insurance", "Vehicle insurance"),
    ),
    (
        "Daily life",
        CategoryKind.EXPENSE,
        (
            "Groceries",
            "Eating out",
            "Household",
            "Clothing",
            "Health costs (deductible)",
            "Leisure",
        ),
    ),
    (
        "Savings goals",
        CategoryKind.EXPENSE,
        ("Pillar 3a", "Emergency fund", "Holidays", "Gifts"),
    ),
)


def create_default_categories(db: Session, user_id: int) -> None:
    for group_order, (group_name, kind, names) in enumerate(DEFAULT_CATEGORIES):
        group = CategoryGroup(user_id=user_id, name=group_name, sort_order=group_order)
        db.add(group)
        db.flush()
        for order, name in enumerate(names):
            db.add(
                Category(user_id=user_id, group_id=group.id, name=name, kind=kind, sort_order=order)
            )
