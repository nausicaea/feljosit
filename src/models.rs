//! The records of the database and the values of the enumerations.

use crate::dates::Date;
use rusqlite::{Connection, OptionalExtension, Row};
use serde::Serialize;

use crate::db::parse_date;

/// Define an enumeration that the database stores as text.
macro_rules! text_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident = $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name { $($(#[$vmeta])* $variant),+ }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self { $($name::$variant => $text),+ }
            }

            pub fn parse(text: &str) -> Option<$name> {
                match text { $($text => Some($name::$variant),)+ _ => None }
            }
        }

        impl rusqlite::ToSql for $name {
            fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
                Ok(self.as_str().into())
            }
        }

        impl rusqlite::types::FromSql for $name {
            fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
                let text = value.as_str()?;
                $name::parse(text).ok_or(rusqlite::types::FromSqlError::InvalidType)
            }
        }

        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }
    };
}

text_enum!(AccountType {
    Checking = "checking",
    Savings = "savings",
    CreditCard = "credit_card",
    Cash = "cash",
    Pillar3a = "pillar_3a",
    Investment = "investment",
    Loan = "loan",
});

impl AccountType {
    pub fn label(self) -> &'static str {
        match self {
            AccountType::Checking => "Private account",
            AccountType::Savings => "Savings account",
            AccountType::CreditCard => "Credit card",
            AccountType::Cash => "Cash",
            AccountType::Pillar3a => "Pillar 3a",
            AccountType::Investment => "Investment (custody account)",
            AccountType::Loan => "Loan or mortgage",
        }
    }

    pub fn on_budget_default(self) -> bool {
        matches!(
            self,
            AccountType::Checking | AccountType::Savings | AccountType::CreditCard | AccountType::Cash
        )
    }
}

text_enum!(CategoryKind { Expense = "expense", Income = "income" });

text_enum!(RuleField {
    Counterparty = "counterparty",
    CounterpartyIban = "counterparty_iban",
    Description = "description",
});

text_enum!(MatchType { Contains = "contains", Equals = "equals" });

text_enum!(TransactionSource {
    Camt053 = "camt053",
    CardCsv = "card_csv",
    Manual = "manual",
    StartingBalance = "starting_balance",
});

text_enum!(
    /// How the application found a transfer pair.
    TransferMethod {
        /// The counterparty IBAN is the IBAN of the other account.
        Iban = "iban",
        /// Two matching transactions (see ADR-0016).
        Match = "match",
    }
);

text_enum!(
    TransferStatus {
        Active = "active",
        /// The user said that the two transactions are not a transfer. The matching
        /// does not make this pair again.
        Rejected = "rejected",
    }
);

#[derive(Debug, Clone, Serialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    #[serde(skip)]
    pub password_hash: String,
    pub failed_logins: i64,
    pub locked_until: Option<String>,
    /// The password is temporary. The user must change it before other pages open.
    pub must_change_password: bool,
}

impl User {
    pub const COLUMNS: &'static str =
        "id, username, password_hash, failed_logins, locked_until, must_change_password";

    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<User> {
        Ok(User {
            id: row.get(0)?,
            username: row.get(1)?,
            password_hash: row.get(2)?,
            failed_logins: row.get(3)?,
            locked_until: row.get(4)?,
            must_change_password: row.get(5)?,
        })
    }

    pub fn by_username(conn: &Connection, username: &str) -> rusqlite::Result<Option<User>> {
        conn.query_row(
            &format!("SELECT {} FROM users WHERE username = ?1", User::COLUMNS),
            [username],
            User::from_row,
        )
        .optional()
    }

    pub fn by_id(conn: &Connection, id: i64) -> rusqlite::Result<Option<User>> {
        conn.query_row(&format!("SELECT {} FROM users WHERE id = ?1", User::COLUMNS), [id], User::from_row)
            .optional()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Account {
    pub id: i64,
    pub user_id: i64,
    pub name: String,
    #[serde(rename = "type")]
    pub account_type: AccountType,
    pub on_budget: bool,
    pub currency: String,
    pub iban: Option<String>,
    pub closed: bool,
}

impl Account {
    pub const COLUMNS: &'static str = "id, user_id, name, type, on_budget, currency, iban, closed";

    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Account> {
        Ok(Account {
            id: row.get(0)?,
            user_id: row.get(1)?,
            name: row.get(2)?,
            account_type: row.get(3)?,
            on_budget: row.get(4)?,
            currency: row.get(5)?,
            iban: row.get(6)?,
            closed: row.get(7)?,
        })
    }

    pub fn get(conn: &Connection, id: i64) -> rusqlite::Result<Option<Account>> {
        conn.query_row(
            &format!("SELECT {} FROM accounts WHERE id = ?1", Account::COLUMNS),
            [id],
            Account::from_row,
        )
        .optional()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Category {
    pub id: i64,
    pub user_id: i64,
    pub group_id: i64,
    pub name: String,
    pub kind: CategoryKind,
    pub hidden: bool,
    pub sort_order: i64,
}

impl Category {
    pub const COLUMNS: &'static str = "id, user_id, group_id, name, kind, hidden, sort_order";

    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Category> {
        Ok(Category {
            id: row.get(0)?,
            user_id: row.get(1)?,
            group_id: row.get(2)?,
            name: row.get(3)?,
            kind: row.get(4)?,
            hidden: row.get(5)?,
            sort_order: row.get(6)?,
        })
    }

    pub fn get(conn: &Connection, id: i64) -> rusqlite::Result<Option<Category>> {
        conn.query_row(
            &format!("SELECT {} FROM categories WHERE id = ?1", Category::COLUMNS),
            [id],
            Category::from_row,
        )
        .optional()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CategoryGroup {
    pub id: i64,
    pub user_id: i64,
    pub name: String,
    pub sort_order: i64,
    /// The categories of the group, in the order of the budget page.
    pub categories: Vec<Category>,
}

impl CategoryGroup {
    pub fn get(conn: &Connection, id: i64) -> rusqlite::Result<Option<CategoryGroup>> {
        conn.query_row(
            "SELECT id, user_id, name, sort_order FROM category_groups WHERE id = ?1",
            [id],
            |row| {
                Ok(CategoryGroup {
                    id: row.get(0)?,
                    user_id: row.get(1)?,
                    name: row.get(2)?,
                    sort_order: row.get(3)?,
                    categories: Vec::new(),
                })
            },
        )
        .optional()
    }

    /// The groups of a user with their categories, in the order of the budget page.
    pub fn list(conn: &Connection, user_id: i64) -> rusqlite::Result<Vec<CategoryGroup>> {
        let mut stmt = conn.prepare(
            "SELECT id, user_id, name, sort_order FROM category_groups WHERE user_id = ?1 \
             ORDER BY sort_order, name",
        )?;
        let mut groups: Vec<CategoryGroup> = stmt
            .query_map([user_id], |row| {
                Ok(CategoryGroup {
                    id: row.get(0)?,
                    user_id: row.get(1)?,
                    name: row.get(2)?,
                    sort_order: row.get(3)?,
                    categories: Vec::new(),
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM categories WHERE user_id = ?1 ORDER BY sort_order, name",
            Category::COLUMNS
        ))?;
        let categories = stmt.query_map([user_id], Category::from_row)?;
        for category in categories {
            let category = category?;
            if let Some(group) = groups.iter_mut().find(|g| g.id == category.group_id) {
                group.categories.push(category);
            }
        }
        Ok(groups)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Transaction {
    pub id: i64,
    pub user_id: i64,
    pub account_id: i64,
    pub booking_date: Date,
    pub value_date: Option<Date>,
    pub amount_minor: i64,
    pub currency: String,
    pub counterparty: String,
    pub counterparty_iban: Option<String>,
    pub description: String,
    pub reference: Option<String>,
    pub bank_reference: Option<String>,
    pub memo: String,
    pub category_id: Option<i64>,
    pub is_transfer: bool,
    pub source: TransactionSource,
    pub fingerprint: String,
    pub import_batch_id: Option<i64>,
}

impl Transaction {
    pub const COLUMNS: &'static str = "transactions.id, transactions.user_id, \
        transactions.account_id, transactions.booking_date, transactions.value_date, \
        transactions.amount_minor, transactions.currency, transactions.counterparty, \
        transactions.counterparty_iban, transactions.description, transactions.reference, \
        transactions.bank_reference, transactions.memo, transactions.category_id, \
        transactions.is_transfer, transactions.source, transactions.fingerprint, \
        transactions.import_batch_id";

    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Transaction> {
        let booking: String = row.get(3)?;
        let value: Option<String> = row.get(4)?;
        Ok(Transaction {
            id: row.get(0)?,
            user_id: row.get(1)?,
            account_id: row.get(2)?,
            booking_date: parse_date(&booking),
            value_date: value.as_deref().map(parse_date),
            amount_minor: row.get(5)?,
            currency: row.get(6)?,
            counterparty: row.get(7)?,
            counterparty_iban: row.get(8)?,
            description: row.get(9)?,
            reference: row.get(10)?,
            bank_reference: row.get(11)?,
            memo: row.get(12)?,
            category_id: row.get(13)?,
            is_transfer: row.get(14)?,
            source: row.get(15)?,
            fingerprint: row.get(16)?,
            import_batch_id: row.get(17)?,
        })
    }

    pub fn get(conn: &Connection, id: i64) -> rusqlite::Result<Option<Transaction>> {
        conn.query_row(
            &format!("SELECT {} FROM transactions WHERE id = ?1", Transaction::COLUMNS),
            [id],
            Transaction::from_row,
        )
        .optional()
    }
}

/// The values of a new transaction.
#[derive(Debug, Clone)]
pub struct NewTransaction<'a> {
    pub user_id: i64,
    pub account_id: i64,
    pub booking_date: Date,
    pub value_date: Option<Date>,
    pub amount_minor: i64,
    pub currency: &'a str,
    pub counterparty: &'a str,
    pub counterparty_iban: Option<&'a str>,
    pub description: &'a str,
    pub reference: Option<&'a str>,
    pub bank_reference: Option<&'a str>,
    pub memo: &'a str,
    pub category_id: Option<i64>,
    pub is_transfer: bool,
    pub source: TransactionSource,
    pub fingerprint: &'a str,
    pub import_batch_id: Option<i64>,
}

impl NewTransaction<'_> {
    /// Add the transaction. Return its ID.
    pub fn insert(&self, conn: &Connection) -> rusqlite::Result<i64> {
        conn.execute(
            "INSERT INTO transactions (user_id, account_id, booking_date, value_date, \
             amount_minor, currency, counterparty, counterparty_iban, description, reference, \
             bank_reference, memo, category_id, is_transfer, source, fingerprint, \
             import_batch_id, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            rusqlite::params![
                self.user_id,
                self.account_id,
                crate::db::date_text(self.booking_date),
                self.value_date.map(crate::db::date_text),
                self.amount_minor,
                self.currency,
                self.counterparty,
                self.counterparty_iban,
                self.description,
                self.reference,
                self.bank_reference,
                self.memo,
                self.category_id,
                self.is_transfer,
                self.source,
                self.fingerprint,
                self.import_batch_id,
                crate::db::now_text(),
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CategorisationRule {
    pub id: i64,
    pub user_id: i64,
    pub field: RuleField,
    pub match_type: MatchType,
    pub pattern: String,
    pub category_id: i64,
    pub priority: i64,
}

impl CategorisationRule {
    pub const COLUMNS: &'static str = "id, user_id, field, match_type, pattern, category_id, priority";

    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<CategorisationRule> {
        Ok(CategorisationRule {
            id: row.get(0)?,
            user_id: row.get(1)?,
            field: row.get(2)?,
            match_type: row.get(3)?,
            pattern: row.get(4)?,
            category_id: row.get(5)?,
            priority: row.get(6)?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct TransferPair {
    pub id: i64,
    pub user_id: i64,
    pub outflow_id: i64,
    pub inflow_id: i64,
    pub method: TransferMethod,
    pub status: TransferStatus,
}

impl TransferPair {
    pub const COLUMNS: &'static str = "id, user_id, outflow_id, inflow_id, method, status";

    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<TransferPair> {
        Ok(TransferPair {
            id: row.get(0)?,
            user_id: row.get(1)?,
            outflow_id: row.get(2)?,
            inflow_id: row.get(3)?,
            method: row.get(4)?,
            status: row.get(5)?,
        })
    }

    /// The other transaction of the pair.
    pub fn partner_of(&self, tx_id: i64) -> i64 {
        if self.outflow_id == tx_id { self.inflow_id } else { self.outflow_id }
    }
}
