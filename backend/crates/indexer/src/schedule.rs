//! Pure scheduling and derivation logic (no I/O), unit-tested below.

/// Blocks per day at ~12 s per block.
pub const BLOCKS_PER_DAY: u32 = 7_200;

/// History depth for a newly added wallet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// Daily for 120 days, then weekly back to `max_days`.
    Recommended {
        max_days: u32,
    },
    /// Daily for the last 30 days.
    Short,
    None,
}

impl Plan {
    pub fn parse(s: &str, max_days: u32) -> Option<Self> {
        match s {
            "rec" => Some(Plan::Recommended { max_days }),
            "short" => Some(Plan::Short),
            "none" => Some(Plan::None),
            _ => None,
        }
    }
}

/// "Days back from head" to sample, newest first (recent history shows up first, and the
/// walk back can stop at dTAO), deduplicated. Day 0 = head.
pub fn plan_days(plan: Plan) -> Vec<u32> {
    let mut days: Vec<u32> = match plan {
        Plan::None => vec![],
        Plan::Short => (0..=30).collect(),
        Plan::Recommended { max_days } => {
            let daily = 120.min(max_days);
            let mut d: Vec<u32> = (0..=daily).collect();
            d.extend((daily + 7..=max_days).step_by(7));
            d
        }
    };
    days.sort_unstable();
    days.dedup();
    days
}

/// Block number for "N days back" from `head`, or `None` before genesis.
pub fn block_for_day(head: u32, days_back: u32) -> Option<u32> {
    head.checked_sub(days_back.checked_mul(BLOCKS_PER_DAY)?)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotKind {
    Hourly,
    Daily,
}

impl SnapshotKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SnapshotKind::Hourly => "hourly",
            SnapshotKind::Daily => "daily",
        }
    }
}

const HOUR_MS: u64 = 3_600_000;
const DAY_MS: u64 = 86_400_000;

/// Which periodic snapshot (if any) is due when moving from the last stored
/// snapshot time `last_ms` to the current block time `now_ms` (both UTC ms).
/// A day boundary wins over an hour boundary. `None` last time ⇒ take one now.
pub fn due_snapshot(last_ms: Option<u64>, now_ms: u64) -> Option<SnapshotKind> {
    let Some(last) = last_ms else { return Some(SnapshotKind::Hourly) };
    if now_ms <= last {
        return None;
    }
    if now_ms / DAY_MS != last / DAY_MS {
        Some(SnapshotKind::Daily)
    } else if now_ms / HOUR_MS != last / HOUR_MS {
        Some(SnapshotKind::Hourly)
    } else {
        None
    }
}

/// One historical point of a validator's cumulative claimed amount (RAO).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CumPoint {
    pub block: u32,
    pub ts_ms: u64,
    pub claimed_cum: u64,
}

/// The basket bookkeeping value changed somewhere in `(after_block, at_block]`.
/// This is only a *candidate*: stake changes move it too, so a claim must be
/// confirmed with the block's events (which also carry the real TAO amount).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChangeWindow {
    pub after_block: u32,
    pub at_block: u32,
    pub ts_ms: u64,
    pub amount_rao: u64,
    pub window_days: f64,
}

/// Windows between consecutive snapshots where `BasketClaimed` changed.
/// Points must be sorted by block.
pub fn change_windows(points: &[CumPoint]) -> Vec<ChangeWindow> {
    points
        .windows(2)
        .filter_map(|w| {
            let (a, b) = (w[0], w[1]);
            (b.claimed_cum != a.claimed_cum).then(|| ChangeWindow {
                after_block: a.block,
                at_block: b.block,
                ts_ms: b.ts_ms,
                amount_rao: b.claimed_cum.abs_diff(a.claimed_cum),
                window_days: (b.block - a.block) as f64 / BLOCKS_PER_DAY as f64,
            })
        })
        .collect()
}

/// Archive errors worth retrying (rate limit, timeouts, dropped connections).
pub fn is_transient(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    ["-32004", "rate limit", "timeout", "timed out", "connection", "restart", "background task closed", "transport"]
        .iter()
        .any(|k| e.contains(k))
}

/// Exponential backoff in seconds, capped at 60 s.
/// A request the node could not finish in time (slow historical runtime call): retrying
/// a few times is fine, forever is not.
pub fn is_timeout(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    e.contains("timeout") || e.contains("timed out")
}

pub fn backoff_secs(attempt: u32) -> u64 {
    (2u64 << attempt.min(5)).min(60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_recommended_is_daily_then_weekly_newest_first() {
        let d = plan_days(Plan::Recommended { max_days: 660 });
        assert_eq!(d.first(), Some(&0));
        assert_eq!(d.last(), Some(&659)); // 127, 134, … , 659 weekly
        assert!(d.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(d.iter().filter(|&&x| x <= 120).count(), 121);
        assert_eq!(d.iter().filter(|&&x| x > 120).count(), 77);
    }

    #[test]
    fn plan_short_and_none() {
        assert_eq!(plan_days(Plan::Short).len(), 31);
        assert!(plan_days(Plan::None).is_empty());
        assert_eq!(plan_days(Plan::Recommended { max_days: 10 }).len(), 11);
    }

    #[test]
    fn block_for_day_stops_at_genesis() {
        assert_eq!(block_for_day(9_581_423, 0), Some(9_581_423));
        assert_eq!(block_for_day(9_581_423, 1), Some(9_574_223));
        assert_eq!(block_for_day(1000, 1), None);
    }

    #[test]
    fn snapshot_due_on_hour_and_day_boundaries() {
        let t = 1_795_581_144_000u64; // 2026-11-25 ~04:32 UTC
        assert_eq!(due_snapshot(None, t), Some(SnapshotKind::Hourly));
        assert_eq!(due_snapshot(Some(t), t + 60_000), None);
        let next_hour = (t / HOUR_MS + 1) * HOUR_MS;
        assert_eq!(due_snapshot(Some(t), next_hour), Some(SnapshotKind::Hourly));
        let next_day = (t / DAY_MS + 1) * DAY_MS;
        assert_eq!(due_snapshot(Some(next_day - 1), next_day), Some(SnapshotKind::Daily));
        assert_eq!(due_snapshot(Some(t), t - 1), None);
    }

    #[test]
    fn change_windows_from_bookkeeping_changes() {
        let p = |block, cum| CumPoint { block, ts_ms: block as u64, claimed_cum: cum };
        // Real values from finney: the bookkeeping jumped 40.43 τ although the claim paid 14.87 τ.
        let pts = [p(9_516_000, 45_938_184_580), p(9_523_200, 45_938_184_580), p(9_537_600, 86_365_550_758)];
        let c = change_windows(&pts);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].amount_rao, 40_427_366_178);
        assert_eq!(c[0].after_block, 9_523_200);
        assert!((c[0].window_days - 2.0).abs() < 1e-9);
        assert!(change_windows(&pts[..1]).is_empty());
    }

    #[test]
    fn transient_errors() {
        assert!(is_transient("RPC error: User error: Historical work rate limit exceeded (-32004)"));
        assert!(is_transient("Connection timeout exceeded: 10s"));
        assert!(!is_transient("Runtime API method `x` not found in trait"));
        assert_eq!(backoff_secs(0), 2);
        assert_eq!(backoff_secs(10), 60);
    }
}
