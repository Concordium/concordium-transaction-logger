CREATE TABLE IF NOT EXISTS account_public_key_bindings(
    idx BIGINT PRIMARY KEY,
    public_key CHAR(64),
    address BYTEA UNIQUE NOT NULL,
    credential_index INT NOT NULL,
    key_index INT NOT NULL,
    is_simple_account BOOLEAN NOT NULL
)