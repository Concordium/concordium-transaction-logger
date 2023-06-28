-- Table of all tracked contracts. Only keps as a reference for tables with relations to contracts.
CREATE TABLE IF NOT EXISTS contracts (
  index INT8 NOT NULL,
  subindex INT8 NOT NULL,

  CONSTRAINT contracts_pkey
    PRIMARY KEY (index, subindex)
);

-- Table of relations between contracts and events logged as part of a transaction.
CREATE TABLE IF NOT EXISTS contract_events (
  id SERIAL8 PRIMARY KEY,
  index INT8 NOT NULL,
  subindex INT8 NOT NULL,
  event JSONB NOT NULL,
  summary INT8 NOT NULL,

  CONSTRAINT ce_contract_fkey
    FOREIGN KEY (index, subindex) REFERENCES contracts (index, subindex) ON DELETE CASCADE ON UPDATE CASCADE,
  CONSTRAINT ce_summary_fkey
    FOREIGN KEY (summary) REFERENCES summaries (id) ON DELETE CASCADE ON UPDATE CASCADE
);

-- Table of relations between contracts and summaries.
CREATE TABLE IF NOT EXISTS contracts_summaries (
  index INT8 NOT NULL,
  subindex INT8 NOT NULL,
  summary INT8 NOT NULL,

  CONSTRAINT cs_contract_fkey
    FOREIGN KEY (index, subindex) REFERENCES contracts (index, subindex) ON DELETE CASCADE ON UPDATE CASCADE,
  CONSTRAINT cs_summary_fkey
    FOREIGN KEY (summary) REFERENCES summaries (id) ON DELETE CASCADE ON UPDATE CASCADE,
  CONSTRAINT cs_unique
    UNIQUE (index, subindex, summary)
);
