-- Revision 0003: login limits for each client and temporary passwords (ADR-0024).

ALTER TABLE users ADD COLUMN must_change_password BOOLEAN NOT NULL DEFAULT 0;

CREATE TABLE login_failures (
	user_id INTEGER NOT NULL,
	client VARCHAR(64) NOT NULL,
	failed_logins INTEGER NOT NULL,
	locked_until DATETIME,
	updated_at DATETIME NOT NULL,
	CONSTRAINT pk_login_failures PRIMARY KEY (user_id, client),
	CONSTRAINT fk_login_failures_user_id_users FOREIGN KEY(user_id) REFERENCES users (id) ON DELETE CASCADE
);

CREATE INDEX ix_login_failures_updated_at ON login_failures (updated_at);
