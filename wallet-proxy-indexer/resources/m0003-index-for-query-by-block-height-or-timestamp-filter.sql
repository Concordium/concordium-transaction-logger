-- Optimizes queries fetching summaries for a specific account, 
-- and filtering entries by block height.   
CREATE INDEX IF NOT EXISTS idx_summaries_id_height
ON summaries(id, height);
-- Optimizes queries fetching summaries for a specific account, 
-- and filtering entries by timestamp.
CREATE INDEX IF NOT EXISTS idx_summaries_id_timestamp
ON summaries(id, timestamp);

-- Optimizes queries fetching summaries for a specific account, 
-- and efficient ordering by id.
CREATE INDEX IF NOT EXISTS idx_ati_account_summary_id ON ati(account, summary, id);
