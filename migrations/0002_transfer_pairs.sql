-- Revision 0002: transfer pairs (ADR-0016).

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

CREATE INDEX ix_transfer_pairs_inflow_id ON transfer_pairs (inflow_id);

CREATE INDEX ix_transfer_pairs_user_id ON transfer_pairs (user_id);
