-- TaoWatch initial schema. Amounts are TAO with 9 decimals.
CREATE EXTENSION IF NOT EXISTS timescaledb;

-- ── auth ────────────────────────────────────────────────────────────────
CREATE TABLE users (
    id           uuid PRIMARY KEY,
    username     text NOT NULL UNIQUE,
    display_name text,
    role         text NOT NULL CHECK (role IN ('admin', 'user')),
    created_at   timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE passkeys (
    id           bytea PRIMARY KEY,
    user_id      uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    credential   jsonb NOT NULL,
    name         text NOT NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    last_used_at timestamptz
);
CREATE INDEX passkeys_user ON passkeys(user_id);

CREATE TABLE recovery_codes (
    user_id   uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash text NOT NULL,
    used_at   timestamptz,
    PRIMARY KEY (user_id, code_hash)
);

CREATE TABLE invites (
    code       text PRIMARY KEY,
    created_by uuid REFERENCES users(id) ON DELETE SET NULL,
    used_by    uuid REFERENCES users(id) ON DELETE SET NULL,
    expires_at timestamptz NOT NULL,
    revoked_at timestamptz
);

-- ── wallets ─────────────────────────────────────────────────────────────
CREATE TABLE wallets (
    address          text PRIMARY KEY,
    added_at         timestamptz NOT NULL DEFAULT now(),
    history_status   text NOT NULL DEFAULT 'pending' CHECK (history_status IN ('pending', 'running', 'done', 'skipped', 'error')),
    history_progress int NOT NULL DEFAULT 0
);

CREATE TABLE user_wallets (
    user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    address    text NOT NULL REFERENCES wallets(address) ON DELETE CASCADE,
    label      text NOT NULL,
    sort       int NOT NULL DEFAULT 0,
    deleted_at timestamptz,               -- soft delete for the 30 s undo
    PRIMARY KEY (user_id, address)
);

CREATE TABLE validators (
    hotkey text PRIMARY KEY,
    label  text
);

-- ── current state ───────────────────────────────────────────────────────
CREATE TABLE wallet_latest (
    address       text PRIMARY KEY REFERENCES wallets(address) ON DELETE CASCADE,
    block         bigint NOT NULL,
    ts            timestamptz NOT NULL,
    free          numeric(30,9) NOT NULL,
    stake         numeric(30,9) NOT NULL,
    claimable     numeric(30,9),
    redeemable    numeric(30,9),
    claimed_total numeric(30,9) NOT NULL DEFAULT 0,
    updated_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE positions_latest (
    address          text NOT NULL REFERENCES wallets(address) ON DELETE CASCADE,
    hotkey           text NOT NULL,
    netuid           int NOT NULL,
    amount           numeric(30,9) NOT NULL,
    pool_tao         numeric(30,9),
    pool_alpha       numeric(30,9),
    value_tao        numeric(30,9) NOT NULL,
    accrued          numeric(30,9),
    redeemable       numeric(30,9),
    claimed_total    numeric(30,9),
    last_claim_block bigint,
    last_claim_at    timestamptz,
    PRIMARY KEY (address, hotkey, netuid)
);

-- ── history (hypertables) ───────────────────────────────────────────────
CREATE TABLE snapshots (
    ts         timestamptz NOT NULL,
    address    text NOT NULL,
    block      bigint NOT NULL,
    kind       text NOT NULL CHECK (kind IN ('hourly', 'daily', 'backfill')),
    free       numeric(30,9) NOT NULL,
    stake      numeric(30,9) NOT NULL,
    claimable  numeric(30,9),
    redeemable numeric(30,9)
);
SELECT create_hypertable('snapshots', by_range('ts', INTERVAL '30 days'));
CREATE UNIQUE INDEX snapshots_addr_block ON snapshots(address, block, ts);

CREATE TABLE position_snapshots (
    ts          timestamptz NOT NULL,
    address     text NOT NULL,
    block       bigint NOT NULL,
    hotkey      text NOT NULL,
    netuid      int NOT NULL,
    amount      numeric(30,9) NOT NULL,
    value_tao   numeric(30,9) NOT NULL,
    accrued     numeric(30,9),
    redeemable  numeric(30,9),
    basket_cum numeric(30,9)
);
SELECT create_hypertable('position_snapshots', by_range('ts', INTERVAL '30 days'));
CREATE INDEX position_snapshots_addr ON position_snapshots(address, ts DESC);

CREATE TABLE events (
    block   bigint NOT NULL,
    idx     int NOT NULL,
    ts      timestamptz NOT NULL,
    address text NOT NULL,
    kind    text NOT NULL,              -- claim | stake_added | stake_removed | transfer_in | transfer_out | system
    hotkey  text,
    netuid  int,
    amount  numeric(30,9),
    source  text NOT NULL CHECK (source IN ('event', 'snapshot_diff')),
    PRIMARY KEY (block, idx, address)
);
CREATE INDEX events_addr_ts ON events(address, ts DESC);

CREATE TABLE claims (
    id          bigserial PRIMARY KEY,
    block       bigint,                  -- exact block when known
    ts          timestamptz NOT NULL,
    address     text NOT NULL,
    hotkey      text NOT NULL,
    amount      numeric(30,9) NOT NULL,
    exact       bool NOT NULL,
    window_days numeric(6,1)             -- when detected between two snapshots
);
CREATE INDEX claims_addr_ts ON claims(address, ts DESC);

-- ── alerts & notifications ──────────────────────────────────────────────
CREATE TABLE alert_rules (
    id             uuid PRIMARY KEY,
    user_id        uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    wallet_address text REFERENCES wallets(address) ON DELETE CASCADE,
    metric         text NOT NULL,        -- claimable_real_delta | claimable_level | days_since_claim | total_change_24h | stake_removed | source_offline
    op             text NOT NULL CHECK (op IN ('below', 'above')),
    threshold      numeric NOT NULL,
    unit           text NOT NULL CHECK (unit IN ('pct', 'tao', 'days', 'blocks')),
    channels       text[] NOT NULL DEFAULT '{web_push}',
    cooldown       interval NOT NULL DEFAULT '1 day',
    enabled        bool NOT NULL DEFAULT true,
    created_at     timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE alert_fires (
    rule_id      uuid NOT NULL REFERENCES alert_rules(id) ON DELETE CASCADE,
    fired_at     timestamptz NOT NULL,
    value        numeric,
    delivered_to text[] NOT NULL DEFAULT '{}',
    PRIMARY KEY (rule_id, fired_at)
);

CREATE TABLE push_subscriptions (
    endpoint   text PRIMARY KEY,
    user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    keys       jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE telegram_links (
    user_id   uuid PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    chat_id   bigint NOT NULL,
    linked_at timestamptz NOT NULL DEFAULT now()
);

-- ── indexer bookkeeping ─────────────────────────────────────────────────
CREATE TABLE backfill_jobs (
    address    text PRIMARY KEY REFERENCES wallets(address) ON DELETE CASCADE,
    status     text NOT NULL DEFAULT 'queued' CHECK (status IN ('queued', 'running', 'done', 'error')),
    plan       text NOT NULL DEFAULT 'rec' CHECK (plan IN ('rec', 'short', 'none')),
    total      int NOT NULL DEFAULT 0,
    done       int NOT NULL DEFAULT 0,
    last_error text,
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE chain_status (
    id         int PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    live_block bigint,
    live_ts    timestamptz,
    seen_at    timestamptz,
    archive_ok bool,
    lag_blocks int
);
INSERT INTO chain_status (id) VALUES (1);
