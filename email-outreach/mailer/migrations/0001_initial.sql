-- Outreach mailer schema.

CREATE TABLE IF NOT EXISTS consumed_receipts (
    receipt_id   BYTEA PRIMARY KEY,
    consumed_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS sends (
    id                    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    recipients_json       TEXT NOT NULL,
    content_hash_hex      TEXT NOT NULL,
    status                TEXT NOT NULL CHECK (status IN ('sent', 'rejected')),
    rejection_reason      TEXT,
    maat_receipt_id       BYTEA NOT NULL,
    maat_delegation_id    BYTEA NOT NULL,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_sends_created_at ON sends (created_at DESC);
CREATE INDEX IF NOT EXISTS idx_sends_receipt_id ON sends (maat_receipt_id);
