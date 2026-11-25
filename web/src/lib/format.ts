// Number/date formatting. Conventions: balances 3 decimals, claimable and deltas 4, τ suffix,
// ▲/▼ plus sign for changes (color is never the only signal).

export const num = (v: string | number | null | undefined): number | null =>
  v === null || v === undefined || v === '' ? null : Number(v);

export function fmt(v: string | number | null | undefined, d = 3): string {
  const n = num(v);
  return n === null || Number.isNaN(n) ? '—' : n.toLocaleString('en-US', { minimumFractionDigits: d, maximumFractionDigits: d });
}

export const tao = (v: string | number | null | undefined, d = 3) => (num(v) === null ? '—' : `${fmt(v, d)} τ`);

/** "▲ +1.2345 τ" / "▼ −0.5000 τ" and a tone class. */
export function signed(v: string | number | null | undefined, d = 4, unit = ' τ'): { text: string; tone: 'up' | 'down' | 'dim' } {
  const n = num(v);
  if (n === null || Number.isNaN(n)) return { text: '—', tone: 'dim' };
  if (n === 0) return { text: `±0${unit}`, tone: 'dim' };
  const abs = Math.abs(n).toLocaleString('en-US', { minimumFractionDigits: d, maximumFractionDigits: d });
  return n > 0 ? { text: `▲ +${abs}${unit}`, tone: 'up' } : { text: `▼ −${abs}${unit}`, tone: 'down' };
}

export const pct = (v: number | null) => (v === null || !Number.isFinite(v) ? '—' : `${v >= 0 ? '+' : '−'}${Math.abs(v).toFixed(1)}%`);

export const short = (a: string) => (a.length > 14 ? `${a.slice(0, 6)}…${a.slice(-4)}` : a);

export const block = (b: number | null | undefined) => (b === null || b === undefined ? '—' : `#${b.toLocaleString('en-US')}`);

/** "2026-11-18 11:34 UTC" */
export const utc = (iso: string | null | undefined) => (iso ? `${iso.slice(0, 16).replace('T', ' ')} UTC` : '—');

export function ago(iso: string | null | undefined, now = Date.now()): string {
  if (!iso) return '—';
  const s = Math.max(0, (now - Date.parse(iso)) / 1000);
  if (s < 90) return 'just now';
  if (s < 5400) return `${Math.round(s / 60)} min ago`;
  if (s < 129600) return `${Math.round(s / 3600)} h ago`;
  return `${Math.round(s / 86400)} d ago`;
}

const BLOCKS = '▁▂▃▄▅▆▇█';
export function sparkline(values: number[]): string {
  if (values.length < 2) return '';
  const mn = Math.min(...values);
  const mx = Math.max(...values);
  return values.map((v) => BLOCKS[Math.min(7, Math.floor(((v - mn) / (mx - mn || 1)) * 8))]).join('');
}

export function bar(pctValue: number, width = 10): string {
  const n = Math.max(pctValue > 0 ? 1 : 0, Math.round((pctValue / 100) * width));
  return '█'.repeat(Math.min(width, n)) + '░'.repeat(Math.max(0, width - n));
}

/** Client-side SS58 sanity check (the server verifies the checksum). */
export function checkAddress(a: string): { ok: boolean; msg: string } {
  const s = a.trim();
  if (!s) return { ok: false, msg: 'Paste a coldkey address starting with 5 (48 characters).' };
  if (/\s/.test(a.trim())) return { ok: false, msg: '✗ Remove spaces from the address.' };
  if (!/^[1-9A-HJ-NP-Za-km-z]+$/.test(s)) return { ok: false, msg: '✗ Contains characters that are not base58 (0, O, I and l are not allowed).' };
  if (s[0] !== '5') return { ok: false, msg: '✗ Bittensor addresses use SS58 prefix 42 and start with "5".' };
  if (s.length !== 48) return { ok: false, msg: `✗ Expected 48 characters, got ${s.length}.` };
  return { ok: true, msg: '✓ Looks valid. The server verifies the checksum.' };
}

export const RANGES = ['30d', '90d', '1y', 'all'] as const;
export type Range = (typeof RANGES)[number];
