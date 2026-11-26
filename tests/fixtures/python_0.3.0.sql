-- A database of Feljosit 0.3.0 (the Python version), with synthetic data.
-- The test python_database.rs checks that the Rust version can use it.
BEGIN TRANSACTION;
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
INSERT INTO "accounts" VALUES(1,1,'Private','checking',1,'CHF','CH8900235000012345678',0,'2026-11-26 18:37:27.876848');
CREATE TABLE alembic_version (
	version_num VARCHAR(32) NOT NULL, 
	CONSTRAINT alembic_version_pkc PRIMARY KEY (version_num)
);
INSERT INTO "alembic_version" VALUES('0002');
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
INSERT INTO "budget_assignments" VALUES(1,1,13,'2026-10-01',50000);
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
INSERT INTO "categories" VALUES(1,1,1,'Salary','income',0,0);
INSERT INTO "categories" VALUES(2,1,1,'13th salary and bonus','income',0,1);
INSERT INTO "categories" VALUES(3,1,1,'Other income','income',0,2);
INSERT INTO "categories" VALUES(4,1,2,'Rent and service charges','expense',0,0);
INSERT INTO "categories" VALUES(5,1,2,'Health insurance (KVG/VVG)','expense',0,1);
INSERT INTO "categories" VALUES(6,1,2,'Electricity and utilities','expense',0,2);
INSERT INTO "categories" VALUES(7,1,2,'Phone and internet','expense',0,3);
INSERT INTO "categories" VALUES(8,1,2,'Serafe fee','expense',0,4);
INSERT INTO "categories" VALUES(9,1,2,'Public transport (GA/Half Fare)','expense',0,5);
INSERT INTO "categories" VALUES(10,1,3,'Taxes','expense',0,0);
INSERT INTO "categories" VALUES(11,1,3,'Household and liability insurance','expense',0,1);
INSERT INTO "categories" VALUES(12,1,3,'Vehicle insurance','expense',0,2);
INSERT INTO "categories" VALUES(13,1,4,'Groceries','expense',0,0);
INSERT INTO "categories" VALUES(14,1,4,'Eating out','expense',0,1);
INSERT INTO "categories" VALUES(15,1,4,'Household','expense',0,2);
INSERT INTO "categories" VALUES(16,1,4,'Clothing','expense',0,3);
INSERT INTO "categories" VALUES(17,1,4,'Health costs (deductible)','expense',0,4);
INSERT INTO "categories" VALUES(18,1,4,'Leisure','expense',0,5);
INSERT INTO "categories" VALUES(19,1,5,'Pillar 3a','expense',0,0);
INSERT INTO "categories" VALUES(20,1,5,'Emergency fund','expense',0,1);
INSERT INTO "categories" VALUES(21,1,5,'Holidays','expense',0,2);
INSERT INTO "categories" VALUES(22,1,5,'Gifts','expense',0,3);
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
INSERT INTO "categorisation_rules" VALUES(1,1,'counterparty','contains','migros',13,100);
CREATE TABLE category_groups (
	id INTEGER NOT NULL, 
	user_id INTEGER NOT NULL, 
	name VARCHAR(100) NOT NULL, 
	sort_order INTEGER NOT NULL, 
	CONSTRAINT pk_category_groups PRIMARY KEY (id), 
	CONSTRAINT fk_category_groups_user_id_users FOREIGN KEY(user_id) REFERENCES users (id) ON DELETE CASCADE
);
INSERT INTO "category_groups" VALUES(1,1,'Income',0);
INSERT INTO "category_groups" VALUES(2,1,'Fixed costs',1);
INSERT INTO "category_groups" VALUES(3,1,'Taxes and insurance',2);
INSERT INTO "category_groups" VALUES(4,1,'Daily life',3);
INSERT INTO "category_groups" VALUES(5,1,'Savings goals',4);
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
INSERT INTO "import_batches" VALUES(1,1,'private.xml','64c1fb9d926d7a10b7ea8e2873c21078c62f9ce7fdd1462a2384f56e0ee5a196','MSG-2026-10-001',7,0,'2026-11-26 18:37:27.882798');
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
INSERT INTO "transactions" VALUES(1,1,1,'2026-09-30','2026-09-30',100000,'CHF','Starting balance',NULL,'',NULL,NULL,'',1,0,'starting_balance','start:1',NULL,'2026-11-26 18:37:27.891119');
INSERT INTO "transactions" VALUES(2,1,1,'2026-10-01','2026-10-01',-210000,'CHF','Immo Beispiel GmbH','CH3880808001234567890','Miete Oktober 2026',NULL,'20261001-RENT-01','',NULL,0,'camt053','camt:e981c135cc9b8627903487d8f37b128dd5515055adbd8291a4dd4d62fa67d958',1,'2026-11-26 18:37:27.891125');
INSERT INTO "transactions" VALUES(3,1,1,'2026-10-05','2026-10-05',-41235,'CHF','Krankenkasse Beispiel','CH0430000001234567890','','210000000003139471430009017','20261005-QR-77','',NULL,0,'camt053','camt:1048fe2379b86cb7ad0690f801bbb20f513b6e9d3e5f6ec81685cec336d98f97',1,'2026-11-26 18:37:27.891126');
INSERT INTO "transactions" VALUES(4,1,1,'2026-10-10','2026-10-09',-8760,'CHF','Debit card payment MIGROS ZUERICH',NULL,'Debit card payment MIGROS ZUERICH',NULL,'20261010-CARD-3','',NULL,0,'camt053','camt:0088e8ec95e4ce81fd7ddf304a4ba925535fcd09357378de91170d87d4d26b0c',1,'2026-11-26 18:37:27.891126');
INSERT INTO "transactions" VALUES(5,1,1,'2026-10-15','2026-10-15',-10000,'CHF','Swisscom (Schweiz) AG',NULL,'Rechnung Oktober',NULL,'20261015-BATCH-9-1','',NULL,0,'camt053','camt:3b2f8804ba10c72732745fe346200cdece33d3919e769e00a313299b1ae79580',1,'2026-11-26 18:37:27.891127');
INSERT INTO "transactions" VALUES(6,1,1,'2026-10-15','2026-10-15',-5000,'CHF','Serafe AG',NULL,'',NULL,'20261015-BATCH-9-2','',NULL,0,'camt053','camt:d780528b85194a529ed951c10678bcaf70bbed50c0fba69034b128b9083d9905',1,'2026-11-26 18:37:27.891127');
INSERT INTO "transactions" VALUES(7,1,1,'2026-10-20','2026-10-20',-50000,'CHF','Anna Muster','CH0300700110000123456','Sparen',NULL,'20261020-TRF-1','',NULL,0,'camt053','camt:166618b16d58cb0f37aa41eb0a4b58d0e83b58bbacd429ccf1cddf24d0bbcd3b',1,'2026-11-26 18:37:27.891128');
INSERT INTO "transactions" VALUES(8,1,1,'2026-10-25','2026-10-25',650000,'CHF','Muster AG','CH9300762011623852957','Lohn Oktober 2026',NULL,'20261025-SAL-1','',NULL,0,'camt053','camt:76ef8092779418e55f65d3b1fe5e7fd370ed42a5850a21f6f7926ce9b623dfdc',1,'2026-11-26 18:37:27.891128');
CREATE TABLE transfer_pairs (
	id INTEGER NOT NULL, 
	user_id INTEGER NOT NULL, 
	outflow_id INTEGER NOT NULL, 
	inflow_id INTEGER NOT NULL, 
	method VARCHAR(10) NOT NULL, 
	status VARCHAR(10) NOT NULL, 
	created_at DATETIME NOT NULL, 
	CONSTRAINT pk_transfer_pairs PRIMARY KEY (id), 
	CONSTRAINT fk_transfer_pairs_inflow_id_transactions FOREIGN KEY(inflow_id) REFERENCES transactions (id) ON DELETE CASCADE, 
	CONSTRAINT fk_transfer_pairs_outflow_id_transactions FOREIGN KEY(outflow_id) REFERENCES transactions (id) ON DELETE CASCADE, 
	CONSTRAINT fk_transfer_pairs_user_id_users FOREIGN KEY(user_id) REFERENCES users (id) ON DELETE CASCADE, 
	CONSTRAINT uq_transfer_pairs_outflow_id UNIQUE (outflow_id, inflow_id)
);
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
INSERT INTO "users" VALUES(1,'anna','$argon2id$v=19$m=65536,t=3,p=4$5jg6mhm3UL6009L2W0x7FQ$a/eW80z+p4Ong81N5pMNoGFcYYgCY1CHXV56qvZrLh0',0,NULL,'2026-11-26 18:37:27.868310');
CREATE INDEX ix_accounts_user_id ON accounts (user_id);
CREATE INDEX ix_category_groups_user_id ON category_groups (user_id);
CREATE INDEX ix_import_batches_user_id ON import_batches (user_id);
CREATE INDEX ix_sessions_user_id ON sessions (user_id);
CREATE INDEX ix_categories_user_id ON categories (user_id);
CREATE INDEX ix_budget_assignments_user_id ON budget_assignments (user_id);
CREATE INDEX ix_categorisation_rules_user_id ON categorisation_rules (user_id);
CREATE INDEX ix_transactions_account_id ON transactions (account_id);
CREATE INDEX ix_transactions_category_id ON transactions (category_id);
CREATE INDEX ix_transactions_user_date ON transactions (user_id, booking_date);
CREATE INDEX ix_transfer_pairs_inflow_id ON transfer_pairs (inflow_id);
CREATE INDEX ix_transfer_pairs_user_id ON transfer_pairs (user_id);
COMMIT;
