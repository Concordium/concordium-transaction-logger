CREATE TABLE IF NOT EXISTS account_public_key_bindings(
    id BIGSERIAL PRIMARY KEY ,
    address BYTEA NOT NULL,
    public_key BYTEA NOT NULL,
    credential_index INT NOT NULL,
    key_index INT NOT NULL,
    is_simple_account BOOLEAN NOT NULL,
    active BOOLEAN NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_account_public_key_bindings_public_key ON account_public_key_bindings (public_key);