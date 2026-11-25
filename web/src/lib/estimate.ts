// Claimable estimate: how fast the root basket has been growing, and where that puts it.
import { num } from './format';
import type { DayPoint } from './types';

const DAY_MS = 86_400_000;

export type Rate = {
  /** Average real change per day (claims added back), τ/day. Can be negative when pool prices fall. */
  perDay: number;
  /** Days the average spans (may be shorter than asked when history is short). */
  days: number;
  since: string;
};

/**
 * Average claimable growth over roughly the last `window` days of `days` (ascending, one point per day).
 * Uses the sum of real Δ (claimable change + TAO claimed in between) so a claim does not look like a drop.
 * Null when there are fewer than two points or a real Δ is missing inside the window.
 */
export function claimRate(days: DayPoint[], window: number): Rate | null {
  const pts = days.filter((d) => d.claimable !== null);
  const last = pts[pts.length - 1];
  if (!last) return null;
  const end = Date.parse(last.day);
  let start = 0;
  for (let i = pts.length - 1; i >= 0; i--) {
    if (Date.parse(pts[i]!.day) <= end - window * DAY_MS) {
      start = i;
      break;
    }
  }
  if (start >= pts.length - 1) return null;
  let sum = 0;
  for (const p of pts.slice(start + 1)) {
    const d = num(p.real_delta);
    if (d === null) return null;
    sum += d;
  }
  const span = (end - Date.parse(pts[start]!.day)) / DAY_MS;
  return span > 0 ? { perDay: sum / span, days: span, since: pts[start]!.day } : null;
}

/** Claimable after `days` more days at `rate` (never below zero). */
export const project = (claimable: string | number | null | undefined, rate: Rate | null, days: number): number | null => {
  const c = num(claimable);
  return c === null || rate === null ? null : Math.max(0, c + rate.perDay * days);
};
