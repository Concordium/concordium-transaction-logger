CREATE TABLE IF NOT EXISTS account_public_key_bindings(
    idx BIGSERIAL PRIMARY KEY ,
    address BYTEA UNIQUE NOT NULL,
    public_key BYTEA,
    credential_index INT NOT NULL,
    key_index INT NOT NULL,
    is_simple_account BOOLEAN NOT NULL,
    active BOOLEAN NOT NULL
)