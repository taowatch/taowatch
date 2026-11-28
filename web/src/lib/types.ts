// Response shapes of the API (backend/crates/api/src/data.rs, auth.rs, alerts.rs).
// Decimal amounts are strings.

export type Dec = string;

export type WalletRow = {
  address: string;
  label: string;
  sort: number;
  history_status: 'pending' | 'running' | 'done' | 'skipped' | 'error';
  history_progress: number;
  block: number | null;
  ts: string | null;
  free: Dec | null;
  stake: Dec | null;
  claimable: Dec | null;
  redeemable: Dec | null;
  claimed_total: Dec | null;
  change_24h: Dec | null;
  last_claim_block: number | null;
  last_claim_at: string | null;
  last_claim_amount: Dec | null;
};

export type PositionRow = {
  hotkey: string;
  label: string | null;
  netuid: number;
  amount: Dec;
  value_tao: Dec;
  price: Dec | null;
  accrued: Dec | null;
  redeemable: Dec | null;
  claimed_total: Dec | null;
  last_claim_block: number | null;
  last_claim_at: string | null;
};

export type Backfill = { status: 'queued' | 'running' | 'done' | 'error'; plan: string; done: number; total: number; last_error: string | null };

export type WalletDetail = { wallet: WalletRow; positions: PositionRow[]; backfill: Backfill | null };

export type DayPoint = {
  day: string;
  block: number;
  free: Dec;
  stake: Dec;
  claimable: Dec | null;
  redeemable: Dec | null;
  total: Dec;
  claimed: Dec;
  real_delta: Dec | null;
  change: Dec | null;
  partial: boolean;
};

export type Summary = {
  wallets: number;
  block: number | null;
  free: Dec;
  stake: Dec;
  claimable: Dec | null;
  redeemable: Dec | null;
  total: Dec;
  claimed_total: Dec;
  change_24h: Dec | null;
  claimable_change_24h: Dec | null;
  last_claim: { address: string; hotkey: string; label: string | null; block: number | null; ts: string; amount: Dec } | null;
};

export type ClaimRow = { address: string; hotkey: string; label: string | null; block: number | null; ts: string; amount: Dec; exact: boolean };

export type ClaimableResponse = {
  days: DayPoint[];
  validators: { hotkey: string; label: string | null; points: [string, Dec][] }[];
};

export type EventRow = {
  block: number;
  idx: number;
  ts: string;
  address: string;
  kind: string;
  hotkey: string | null;
  label: string | null;
  netuid: number | null;
  from_hotkey: string | null;
  from_label: string | null;
  from_netuid: number | null;
  amount: Dec | null;
  source: 'event' | 'snapshot_diff';
};

export type Rule = {
  id: string;
  wallet_address: string | null;
  metric: 'claimable_real_delta' | 'claimable_level' | 'days_since_claim' | 'total_change_24h' | 'stake_removed' | 'source_offline';
  op: 'below' | 'above';
  threshold: Dec;
  unit: 'pct' | 'tao' | 'days' | 'blocks';
  channels: string[];
  enabled: boolean;
  created_at: string;
};

export type PasskeyInfo = { id: string; name: string; created_at: string; last_used_at: string | null };
export type Invite = { code: string; used_by: string | null; expires_at: string; revoked_at: string | null };
export type UserInfo = { username: string; role: string; wallets: number; created_at: string };
