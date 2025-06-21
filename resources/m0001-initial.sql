-- Table of all tranasction/protocol event summaries that affect an account, or contract.
CREATE TABLE IF NOT EXISTS summaries(
             id SERIAL8 PRIMARY KEY UNIQUE,
             block BYTEA NOT NULL,
             timestamp INT8 NOT NULL,
             height INT8 NOT NULL,
             -- Summary of the item. Either a user-generated transaction, a chain-update transaction (create PLT transaction)
             -- or a protocol event (such as validator rewards, transaction fee rewards, finalization rewards, foundation rewards in payday blocks).
             summary JSONB NOT NULL);

-- Table associates summaries to affected accounts.
CREATE TABLE IF NOT EXISTS ati(
             id SERIAL8,
             account BYTEA NOT NULL,
             -- `Id` of the row in the summaries table.
             summary INT8 NOT NULL,
             -- The primary key enables to query efficiently "Give me all summaries associated to a given account".
             CONSTRAINT ati_pkey PRIMARY KEY (account, id),
             CONSTRAINT ati_summary_fkey FOREIGN KEY(summary) REFERENCES summaries(id) ON DELETE RESTRICT ON UPDATE RESTRICT
             );

-- Table associates summaries to affected contracts.
CREATE TABLE IF NOT EXISTS cti(
             id SERIAL8,
             index INT8 NOT NULL,
             subindex INT8 NOT NULL,
             -- `Id` of the row in the summaries table.
             summary INT8 NOT NULL,
             -- The primary key enables to query efficiently "Give me all summaries associated to a given contract".
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
-- execute the query "Give me all tokens in the given contract".
CREATE INDEX IF NOT EXISTS cis2_tokens_address ON cis2_tokens (index, subindex);
