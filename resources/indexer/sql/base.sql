-- Executed for any configuration

-- Table of all tracked tranasction summaries.
CREATE TABLE IF NOT EXISTS summaries (
  id SERIAL8 PRIMARY KEY,
  block BYTEA NOT NULL,
  timestamp INT8 NOT NULL,
  height INT8 NOT NULL, -- To know where to start from on restart.
  type INT2 NOT NULL, -- To facilitate more flexible querying.
  summary JSONB NOT NULL
  data JSONB, -- Optional, parsed from transactions of type "RegisterData".
);

-- Table of active configuration used for the service.
CREATE TABLE IF NOT EXISTS config (
  config_hash BYTEA NOT NULL -- Used to validate configuration on service startup.
);
