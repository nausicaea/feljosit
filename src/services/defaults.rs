//! Default categories for a new user. They are typical for a Swiss household.

use rusqlite::{Connection, params};

use crate::models::CategoryKind;

pub const DEFAULT_CATEGORIES: &[(&str, CategoryKind, &[&str])] = &[
    ("Income", CategoryKind::Income, &["Salary", "13th salary and bonus", "Other income"]),
    (
        "Fixed costs",
        CategoryKind::Expense,
        &[
            "Rent and service charges",
            "Health insurance (KVG/VVG)",
            "Electricity and utilities",
            "Phone and internet",
            "Serafe fee",
            "Public transport (GA/Half Fare)",
        ],
    ),
    (
        "Taxes and insurance",
        CategoryKind::Expense,
        &["Taxes", "Household and liability insurance", "Vehicle insurance"],
    ),
    (
        "Daily life",
        CategoryKind::Expense,
        &["Groceries", "Eating out", "Household", "Clothing", "Health costs (deductible)", "Leisure"],
    ),
    ("Savings goals", CategoryKind::Expense, &["Pillar 3a", "Emergency fund", "Holidays", "Gifts"]),
];

pub fn create_default_categories(conn: &Connection, user_id: i64) -> rusqlite::Result<()> {
    for (group_order, (group_name, kind, names)) in DEFAULT_CATEGORIES.iter().enumerate() {
        conn.execute(
            "INSERT INTO category_groups (user_id, name, sort_order) VALUES (?1, ?2, ?3)",
            params![user_id, group_name, group_order as i64],
        )?;
        let group_id = conn.last_insert_rowid();
        for (order, name) in names.iter().enumerate() {
            conn.execute(
                "INSERT INTO categories (user_id, group_id, name, kind, hidden, sort_order) \
                 VALUES (?1, ?2, ?3, ?4, 0, ?5)",
                params![user_id, group_id, name, kind, order as i64],
            )?;
        }
    }
    Ok(())
}
