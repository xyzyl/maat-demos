-- commerce-store database — separate from the gateway's database.
-- Run on `maat_commerce_store` (NOT `maat_gateway`).
--
-- All amounts are in minor units (e.g., cents for USD) with an explicit
-- currency code, matching the Maat protocol's MaxValue shape so amount
-- comparisons between receipts and cart totals are unambiguous.

CREATE TABLE products (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    sku          TEXT NOT NULL UNIQUE,
    name         TEXT NOT NULL,
    description  TEXT NOT NULL DEFAULT '',
    price_cents  BIGINT NOT NULL CHECK (price_cents > 0),
    currency     TEXT NOT NULL DEFAULT 'USD',
    image_url    TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_products_sku ON products (sku);

CREATE TABLE customers (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email               TEXT NOT NULL UNIQUE,
    stripe_customer_id  TEXT NOT NULL UNIQUE,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- One row per checkout attempt — successful or failed.
CREATE TABLE transactions (
    id                       UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id              UUID NOT NULL REFERENCES customers(id),
    product_id               UUID NOT NULL REFERENCES products(id),
    amount_cents             BIGINT NOT NULL,
    currency                 TEXT NOT NULL,
    status                   TEXT NOT NULL CHECK (status IN ('succeeded','failed','rejected')),

    -- Stripe trace
    stripe_payment_intent_id TEXT,
    stripe_error             TEXT,

    -- Maat trace — all transactions reference the receipt that authorized
    -- (or attempted to authorize) the action. For 'rejected' rows the
    -- receipt was valid but didn't match the cart; for 'succeeded' rows
    -- the receipt cleared every check.
    maat_receipt_id          BYTEA NOT NULL,
    maat_delegation_id       BYTEA NOT NULL,
    rejection_reason         TEXT,

    created_at               TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_transactions_customer ON transactions (customer_id, created_at DESC);
CREATE INDEX idx_transactions_status ON transactions (status, created_at DESC);

-- Receipt replay protection: the store remembers which receipt IDs it has
-- already processed. Distinct from the transactions table because a
-- receipt might exist in transactions multiple times (once succeeded, then
-- presented again — the second presentation is the replay we reject).
CREATE TABLE consumed_receipts (
    receipt_id   BYTEA PRIMARY KEY,
    consumed_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
