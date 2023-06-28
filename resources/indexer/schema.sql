CREATE TABLE IF NOT EXISTS summaries (
  id SERIAL8 PRIMARY KEY,
  block BYTEA NOT NULL,
  timestamp INT8 NOT NULL,
  height INT8 NOT NULL,
  type INT2 NOT NULL,
  summary JSONB NOT NULL
);

CREATE TABLE IF NOT EXISTS accounts (
  address BYTEA PRIMARY KEY
);

CREATE TABLE IF NOT EXISTS accounts_summaries (
  account BYTEA NOT NULL,
  summary INT8 NOT NULL,

  CONSTRAINT as_account_fkey FOREIGN KEY(account) REFERENCES accounts(address) ON DELETE CASCADE ON UPDATE CASCADE,
  CONSTRAINT as_summary_fkey FOREIGN KEY(summary) REFERENCES summaries(id) ON DELETE CASCADE ON UPDATE CASCADE,
  UNIQUE (account, summary)
);

CREATE TABLE IF NOT EXISTS account_data_registrations (
  id serial8 PRIMARY KEY,
  account BYTEA NOT NULL,
  summary INT8 UNIQUE NOT NULL,
  data JSONB NOT NULL,

  CONSTRAINT adr_account_fkey FOREIGN KEY(account) REFERENCES accounts(address) ON DELETE CASCADE ON UPDATE CASCADE,
  CONSTRAINT adr_summary_fkey FOREIGN KEY(summary) REFERENCES summaries(id) ON DELETE CASCADE ON UPDATE CASCADE
);

CREATE TABLE IF NOT EXISTS contracts (
  index INT8 NOT NULL,
  subindex INT8 NOT NULL,

  CONSTRAINT contracts_pkey PRIMARY KEY (index, subindex)
);

CREATE TABLE IF NOT EXISTS contract_events (
  id SERIAL8 PRIMARY KEY,
  index INT8 NOT NULL,
  subindex INT8 NOT NULL,
  event JSONB NOT NULL,
  summary INT8,

  CONSTRAINT ce_index_fkey FOREIGN KEY(index) REFERENCES contracts(index) ON DELETE CASCADE ON UPDATE CASCADE,
  CONSTRAINT ce_subindex_fkey FOREIGN KEY(index) REFERENCES contracts(index) ON DELETE CASCADE ON UPDATE CASCADE,
  CONSTRAINT ce_summary_fkey FOREIGN KEY(summary) REFERENCES summaries(id) ON DELETE CASCADE ON UPDATE CASCADE,
  UNIQUE (index, subindex),
  UNIQUE (index, subindex, summary)
);

CREATE TABLE IF NOT EXISTS contracts_summaries (
  index INT8 NOT NULL,
  subindex INT8 NOT NULL,
  summary INT8 NOT NULL,

  CONSTRAINT cs_index_fkey FOREIGN KEY(index) REFERENCES contracts(index) ON DELETE CASCADE ON UPDATE CASCADE,
  CONSTRAINT cs_subindex_fkey FOREIGN KEY(index) REFERENCES contracts(index) ON DELETE CASCADE ON UPDATE CASCADE,
  CONSTRAINT cs_summary_fkey FOREIGN KEY(summary) REFERENCES summaries(id) ON DELETE CASCADE ON UPDATE CASCADE,
  UNIQUE (index, subindex),
  UNIQUE (index, subindex, summary)
);

CREATE TABLE IF NOT EXISTS cis2_tokens (
  id SERIAL8 PRIMARY KEY,
  index INT8 NOT NULL,
  subindex INT8 NOT NULL,
  token_id BYTEA NOT NULL,
  total_supply NUMERIC(80,0) NOT NULL,

  CONSTRAINT c2t_index_fkey FOREIGN KEY(index) REFERENCES contracts(index) ON DELETE CASCADE ON UPDATE CASCADE,
  CONSTRAINT c2t_subindex_fkey FOREIGN KEY(index) REFERENCES contracts(index) ON DELETE CASCADE ON UPDATE CASCADE,
  UNIQUE (index, subindex),
  UNIQUE (index, subindex, summary)
);

CREATE TABLE IF NOT EXISTS config (
  config_hash BYTEA NOT NULL
);
