-- Executed if configuration includes tracking of any accounts

-- Table of all tracked accounts. Only kept as a reference for tables with relations to accounts
CREATE TABLE IF NOT EXISTS accounts (
  address BYTEA PRIMARY KEY
);

-- Table of relations between accounts and summaries
CREATE TABLE IF NOT EXISTS accounts_summaries (
  account BYTEA NOT NULL,
  summary INT8 NOT NULL,

  CONSTRAINT as_account_fkey
    FOREIGN KEY (account) REFERENCES accounts (address) ON DELETE CASCADE ON UPDATE CASCADE,
  CONSTRAINT as_summary_fkey
    FOREIGN KEY (summary) REFERENCES summaries (id) ON DELETE CASCADE ON UPDATE CASCADE,
  CONSTRAINT as_unique
    UNIQUE (account, summary)
);
