-- Revision 0001: initial schema. The same schema as the Alembic migration of the
-- Python version, thus both versions can use the same database file.

CREATE TABLE users (
	id INTEGER NOT NULL,
	username VARCHAR(64) NOT NULL,
	password_hash VARCHAR(255) NOT NULL,
	failed_logins INTEGER NOT NULL,
	locked_until DATETIME,
	created_at DATETIME NOT NULL,
	CONSTRAINT pk_users PRIMARY KEY (id),
	CONSTRAINT uq_users_username UNIQUE (username)
);

CREATE TABLE accounts (
	id INTEGER NOT NULL,
	user_id INTEGER NOT NULL,
	name VARCHAR(100) NOT NULL,
	type VARCHAR(20) NOT NULL,
	on_budget BOOLEAN NOT NULL,
	currency VARCHAR(3) NOT NULL,
	iban VARCHAR(34),
	closed BOOLEAN NOT NULL,
	created_at DATETIME NOT NULL,
	CONSTRAINT pk_accounts PRIMARY KEY (id),
	CONSTRAINT fk_accounts_user_id_users FOREIGN KEY(user_id) REFERENCES users (id) ON DELETE CASCADE,
	CONSTRAINT uq_accounts_user_id UNIQUE (user_id, iban)
);

CREATE INDEX ix_accounts_user_id ON accounts (user_id);

CREATE TABLE category_groups (
	id INTEGER NOT NULL,
	user_id INTEGER NOT NULL,
	name VARCHAR(100) NOT NULL,
	sort_order INTEGER NOT NULL,
	CONSTRAINT pk_category_groups PRIMARY KEY (id),
	CONSTRAINT fk_category_groups_user_id_users FOREIGN KEY(user_id) REFERENCES users (id) ON DELETE CASCADE
);

CREATE INDEX ix_category_groups_user_id ON category_groups (user_id);

CREATE TABLE import_batches (
	id INTEGER NOT NULL,
	user_id INTEGER NOT NULL,
	filename VARCHAR(255) NOT NULL,
	file_sha256 VARCHAR(64) NOT NULL,
	message_id VARCHAR(35),
	new_count INTEGER NOT NULL,
	duplicate_count INTEGER NOT NULL,
	imported_at DATETIME NOT NULL,
	CONSTRAINT pk_import_batches PRIMARY KEY (id),
	CONSTRAINT fk_import_batches_user_id_users FOREIGN KEY(user_id) REFERENCES users (id) ON DELETE CASCADE,
	CONSTRAINT uq_import_batches_user_id UNIQUE (user_id, file_sha256)
);

CREATE INDEX ix_import_batches_user_id ON import_batches (user_id);

CREATE TABLE sessions (
	token_hash VARCHAR(64) NOT NULL,
	user_id INTEGER NOT NULL,
	csrf_token VARCHAR(64) NOT NULL,
	flash_message VARCHAR(500),
	flash_is_error BOOLEAN NOT NULL,
	created_at DATETIME NOT NULL,
	expires_at DATETIME NOT NULL,
	CONSTRAINT pk_sessions PRIMARY KEY (token_hash),
	CONSTRAINT fk_sessions_user_id_users FOREIGN KEY(user_id) REFERENCES users (id) ON DELETE CASCADE
);

CREATE INDEX ix_sessions_user_id ON sessions (user_id);

CREATE TABLE categories (
	id INTEGER NOT NULL,
	user_id INTEGER NOT NULL,
	group_id INTEGER NOT NULL,
	name VARCHAR(100) NOT NULL,
	kind VARCHAR(10) NOT NULL,
	hidden BOOLEAN NOT NULL,
	sort_order INTEGER NOT NULL,
	CONSTRAINT pk_categories PRIMARY KEY (id),
	CONSTRAINT fk_categories_group_id_category_groups FOREIGN KEY(group_id) REFERENCES category_groups (id) ON DELETE CASCADE,
	CONSTRAINT fk_categories_user_id_users FOREIGN KEY(user_id) REFERENCES users (id) ON DELETE CASCADE
);

CREATE INDEX ix_categories_user_id ON categories (user_id);

CREATE TABLE budget_assignments (
	id INTEGER NOT NULL,
	user_id INTEGER NOT NULL,
	category_id INTEGER NOT NULL,
	month DATE NOT NULL,
	amount_minor INTEGER NOT NULL,
	CONSTRAINT pk_budget_assignments PRIMARY KEY (id),
	CONSTRAINT fk_budget_assignments_category_id_categories FOREIGN KEY(category_id) REFERENCES categories (id) ON DELETE CASCADE,
	CONSTRAINT fk_budget_assignments_user_id_users FOREIGN KEY(user_id) REFERENCES users (id) ON DELETE CASCADE,
	CONSTRAINT uq_budget_assignments_category_id UNIQUE (category_id, month)
);

CREATE INDEX ix_budget_assignments_user_id ON budget_assignments (user_id);

CREATE TABLE categorisation_rules (
	id INTEGER NOT NULL,
	user_id INTEGER NOT NULL,
	field VARCHAR(20) NOT NULL,
	match_type VARCHAR(10) NOT NULL,
	pattern VARCHAR(140) NOT NULL,
	category_id INTEGER NOT NULL,
	priority INTEGER NOT NULL,
	CONSTRAINT pk_categorisation_rules PRIMARY KEY (id),
	CONSTRAINT fk_categorisation_rules_category_id_categories FOREIGN KEY(category_id) REFERENCES categories (id) ON DELETE CASCADE,
	CONSTRAINT fk_categorisation_rules_user_id_users FOREIGN KEY(user_id) REFERENCES users (id) ON DELETE CASCADE
);

CREATE INDEX ix_categorisation_rules_user_id ON categorisation_rules (user_id);

CREATE TABLE transactions (
	id INTEGER NOT NULL,
	user_id INTEGER NOT NULL,
	account_id INTEGER NOT NULL,
	booking_date DATE NOT NULL,
	value_date DATE,
	amount_minor INTEGER NOT NULL,
	currency VARCHAR(3) NOT NULL,
	counterparty VARCHAR(140) NOT NULL,
	counterparty_iban VARCHAR(34),
	description TEXT NOT NULL,
	reference VARCHAR(35),
	bank_reference VARCHAR(35),
	memo VARCHAR(500) NOT NULL,
	category_id INTEGER,
	is_transfer BOOLEAN NOT NULL,
	source VARCHAR(20) NOT NULL,
	fingerprint VARCHAR(80) NOT NULL,
	import_batch_id INTEGER,
	created_at DATETIME NOT NULL,
	CONSTRAINT pk_transactions PRIMARY KEY (id),
	CONSTRAINT fk_transactions_account_id_accounts FOREIGN KEY(account_id) REFERENCES accounts (id) ON DELETE CASCADE,
	CONSTRAINT fk_transactions_category_id_categories FOREIGN KEY(category_id) REFERENCES categories (id) ON DELETE SET NULL,
	CONSTRAINT fk_transactions_import_batch_id_import_batches FOREIGN KEY(import_batch_id) REFERENCES import_batches (id) ON DELETE SET NULL,
	CONSTRAINT fk_transactions_user_id_users FOREIGN KEY(user_id) REFERENCES users (id) ON DELETE CASCADE,
	CONSTRAINT uq_transactions_account_id UNIQUE (account_id, fingerprint)
);

CREATE INDEX ix_transactions_account_id ON transactions (account_id);

CREATE INDEX ix_transactions_category_id ON transactions (category_id);

CREATE INDEX ix_transactions_user_date ON transactions (user_id, booking_date);
