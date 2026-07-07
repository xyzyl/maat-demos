CREATE TABLE IF NOT EXISTS consumed_receipts (
    receipt_id   BYTEA PRIMARY KEY,
    consumed_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS deploys (
    id                    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    environment           TEXT NOT NULL,
    service               TEXT NOT NULL,
    commit_hash           TEXT NOT NULL,
    status                TEXT NOT NULL CHECK (status IN ('deployed', 'rejected')),
    rejection_reason      TEXT,
    maat_receipt_id       BYTEA NOT NULL,
    maat_delegation_id    BYTEA NOT NULL,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_deploys_created_at ON deploys (created_at DESC);
CREATE INDEX IF NOT EXISTS idx_deploys_env_service ON deploys (environment, service);
