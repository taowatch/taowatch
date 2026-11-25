// Thin typed fetch helpers. Amounts arrive as decimal strings (rust_decimal) — never floats on the wire.

export type ChainStatus = { live_block: number | null; seen_at: string | null; lag_blocks: number | null };

export async function getJson<T>(path: string): Promise<T> {
  const res = await fetch(path, { credentials: 'same-origin' });
  if (!res.ok) throw new Error(`${res.status} ${await res.text()}`);
  return (await res.json()) as T;
}

export type NetState = 'live' | 'lag' | 'offline' | 'syncing';

/** Map chain status to the status-bar state: LAG after 5 blocks (~60 s), OFFLINE after 5 minutes. */
export function netState(s: ChainStatus | undefined, now = Date.now()): NetState {
  if (!s?.seen_at) return 'offline';
  const age = (now - Date.parse(s.seen_at)) / 1000;
  if (age > 300) return 'offline';
  if (age > 60 || (s.lag_blocks ?? 0) > 5) return 'lag';
  return 'live';
}
