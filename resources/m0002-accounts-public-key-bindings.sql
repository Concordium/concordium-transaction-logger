-- Table that stores the public keys associated with the account addresses
CREATE TABLE IF NOT EXISTS account_public_key_bindings(
    id BIGSERIAL PRIMARY KEY ,
    -- The account address.
    address BYTEA NOT NULL,
    -- The public key.
    public_key BYTEA NOT NULL,
    -- The index of a public key by a credential.
    credential_index INT NOT NULL,
    key_index INT NOT NULL,
    -- True if there is only one public key associated with a given account.
    is_simple_account BOOLEAN NOT NULL,
);

CREATE INDEX IF NOT EXISTS idx_account_public_key_bindings_public_key ON account_public_key_bindings (public_key);