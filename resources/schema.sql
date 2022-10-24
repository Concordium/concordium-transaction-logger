-- Table of all tranasction summaries.
CREATE TABLE IF NOT EXISTS summaries(
             id SERIAL8 PRIMARY KEY UNIQUE,
             block BYTEA NOT NULL,
             timestamp INT8 NOT NULL,
             height INT8 NOT NULL,
             summary JSONB NOT NULL);

-- Index of transactions that affect a given account.
CREATE TABLE IF NOT EXISTS ati(
             id SERIAL8,
             account BYTEA NOT NULL,
             summary INT8 NOT NULL,
             CONSTRAINT ati_pkey PRIMARY KEY (account, id),
             CONSTRAINT ati_summary_fkey FOREIGN KEY(summary) REFERENCES summaries(id) ON DELETE RESTRICT ON UPDATE RESTRICT
             );

-- Index of transactions that affect a given contract.
CREATE TABLE IF NOT EXISTS cti(
             id SERIAL8,
             index INT8 NOT NULL,
             subindex INT8 NOT NULL,
             summary INT8 NOT NULL,
             CONSTRAINT cti_pkey PRIMARY KEY (index, subindex, id),
             CONSTRAINT cti_summary_fkey FOREIGN KEY(summary) REFERENCES summaries(id) ON DELETE RESTRICT ON UPDATE RESTRICT);

-- Table containing all the CIS2 tokens.
CREATE TABLE IF NOT EXISTS cis2_tokens(
             id SERIAL8 UNIQUE PRIMARY KEY,
             index INT8 NOT NULL,
             subindex INT8 NOT NULL,
             token_id BYTEA NOT NULL,
             total_supply NUMERIC(80, 0) NOT NULL,
             UNIQUE (index, subindex, token_id));

-- Index by contract address containing the token. This is needed to efficiently
-- execute the query (give me all tokens in the given contract).
CREATE INDEX IF NOT EXISTS cis2_tokens_address ON cis2_tokens (index, subindex);
