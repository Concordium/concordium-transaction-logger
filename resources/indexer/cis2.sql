-- Table of CIS2 tokens.
CREATE TABLE IF NOT EXISTS cis2_tokens (
  id SERIAL8 PRIMARY KEY,
  index INT8 NOT NULL,
  subindex INT8 NOT NULL,
  token_id BYTEA NOT NULL,
  total_supply NUMERIC(80,0) NOT NULL,

  CONSTRAINT c2t_index_fkey
    FOREIGN KEY (index, subindex) REFERENCES contracts (index, subindex) ON DELETE CASCADE ON UPDATE CASCADE,
  CONSTRAINT c2t_unique
    UNIQUE (index, subindex, token_id)
);
