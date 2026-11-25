// Thin typed fetch helpers. Amounts arrive as decimal strings (rust_decimal) — never floats on the wire.

export class ApiError extends Error {
  constructor(public status: number, message: string) {
    super(message);
  }
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(path, {
    method,
    credentials: 'same-origin',
    headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!res.ok) {
    let msg = `${res.status} ${res.statusText}`;
    try {
      const j = (await res.json()) as { error?: string };
      if (j.error) msg = j.error;
    } catch {
      /* not JSON */
    }
    throw new ApiError(res.status, msg);
  }
  if (res.status === 204) return undefined as T;
  return (await res.json()) as T;
}

export const getJson = <T,>(path: string) => request<T>('GET', path);
export const postJson = <T,>(path: string, body?: unknown) => request<T>('POST', path, body ?? {});

export type Me = { id: string; username: string; display_name: string | null; role: 'admin' | 'user' };

export type ChainStatus = {
  live_block: number | null;
  live_ts: string | null;
  seen_at: string | null;
  lag_blocks: number | null;
  archive_ok: boolean | null;
};

export type NetState = 'live' | 'lag' | 'offline' | 'syncing';

/** Map chain status to the status-bar state: LAG after 5 blocks (~60 s), OFFLINE after 5 minutes. */
export function netState(s: ChainStatus | undefined, now = Date.now()): NetState {
  if (!s?.seen_at) return 'offline';
  const age = (now - Date.parse(s.seen_at)) / 1000;
  if (age > 300) return 'offline';
  if (age > 60 || (s.lag_blocks ?? 0) > 5) return 'lag';
  return 'live';
}
