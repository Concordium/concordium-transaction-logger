-- Table associates summaries to affected PLTs (protocol level tokens).
CREATE TABLE IF NOT EXISTS pltti(
             id SERIAL8,
             token_id TEXT NOT NULL,
             -- `Id` of the row in the summaries table.
             summary INT8 NOT NULL,
             -- The primary key enables to query efficiently "Give me all summaries associated to a given PLT".
             CONSTRAINT pltti_pkey PRIMARY KEY (token_id, id),
             CONSTRAINT pltti_summary_fkey FOREIGN KEY(summary) REFERENCES summaries(id) ON DELETE RESTRICT ON UPDATE RESTRICT);

-- Table containing all the PLT tokens.
CREATE TABLE IF NOT EXISTS plt_tokens(
             id SERIAL8 UNIQUE PRIMARY KEY,
             token_id TEXT NOT NULL UNIQUE,
             total_supply NUMERIC NOT NULL DEFAULT 0);