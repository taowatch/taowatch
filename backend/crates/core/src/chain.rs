//! Subtensor chain client.
//!
//! Verified against finney mainnet (November 2026):
//! - subxt 0.51 defaults to the `chainHead_v1` RPCs, which Subtensor nodes reject
//!   with "Method not found" → we always use [`LegacyBackend`].
//! - Stake comes from `StakeInfoRuntimeApi` (storage `AlphaV2` holds pool *shares*).
//! - Claimable comes from `BetaBasketRuntimeApi` (root claim "beta basket" model).
//!   Older runtimes lack it entirely; its first version lacks `claim_previews`.
//! - Runtimes before dTAO have a different `StakeInfo` layout → [`SnapshotError::PreDtao`].

use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, Result};
use subxt::backend::LegacyBackend;
use subxt::dynamic;
pub use subxt::utils::AccountId32;
use subxt::{OnlineClient, SubstrateConfig};

type Acc = [u8; 32];

#[derive(Debug, scale_decode::DecodeAsType)]
struct AccountInfo {
    nonce: u32,
    data: AccountData,
}
#[derive(Debug, scale_decode::DecodeAsType)]
struct AccountData {
    free: u128,
}
#[derive(Debug, scale_decode::DecodeAsType)]
struct RawStakeInfo {
    hotkey: Acc,
    netuid: u16,
    stake: u64,
}
#[derive(Debug, scale_decode::DecodeAsType)]
struct RawPreview {
    hotkey: Acc,
    accrued_tao: u64,
    redeemable_tao: u64,
}

/// One stake position (coldkey → hotkey on a subnet).
#[derive(Debug, Clone)]
pub struct Position {
    pub hotkey: AccountId32,
    pub netuid: u16,
    /// Amount in RAO of the subnet's token (TAO on root, alpha elsewhere).
    pub amount_rao: u64,
    /// Pool price as (SubnetTAO, SubnetAlphaIn); `None` on root (price 1).
    pub pool: Option<(u64, u64)>,
}

impl Position {
    /// Value in RAO of TAO, using the pool price.
    pub fn value_rao(&self) -> u128 {
        match self.pool {
            None => self.amount_rao as u128,
            Some((_, 0)) => 0,
            Some((tao, alpha)) => self.amount_rao as u128 * tao as u128 / alpha as u128,
        }
    }
}

/// Root basket state for one validator.
#[derive(Debug, Clone)]
pub struct BasketPreview {
    pub hotkey: AccountId32,
    pub accrued_rao: u64,
    pub redeemable_rao: u64,
}

/// Everything TaoWatch reads for one coldkey at one block.
#[derive(Debug, Clone)]
pub struct WalletSnapshot {
    pub block: u32,
    pub timestamp_ms: u64,
    pub free_rao: u128,
    pub positions: Vec<Position>,
    /// `None` before the basket runtime existed.
    pub claimable_rao: Option<u64>,
    /// Empty if the runtime's basket API has no previews yet.
    pub previews: Vec<BasketPreview>,
    /// `BasketClaimed(hotkey, coldkey)` per hotkey — basket *bookkeeping*, not TAO received
    /// (it also moves on stake changes). Only for locating claims; amounts come from events.
    /// Signed on chain: a stake move can leave it slightly negative (seen: -2).
    pub claimed_rao: Vec<(AccountId32, i64)>,
}

impl WalletSnapshot {
    pub fn stake_rao(&self) -> u128 {
        self.positions.iter().map(Position::value_rao).sum()
    }
    pub fn redeemable_rao(&self) -> Option<u64> {
        (!self.previews.is_empty()).then(|| self.previews.iter().map(|p| p.redeemable_rao).sum())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    /// The block's runtime predates dTAO; there is no comparable stake data.
    #[error("runtime at this block predates dTAO stake info")]
    PreDtao,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// An on-chain event that concerns one tracked coldkey.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletEvent {
    /// Index of the event within its block.
    pub index: u32,
    pub coldkey: AccountId32,
    pub kind: EventKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventKind {
    /// Basket redeemed for one validator; the TAO is staked onto root.
    Claim {
        hotkey: AccountId32,
        tao_rao: u64,
    },
    /// Coldkey-wide root claim total (emitted alongside per-validator `Claim`s).
    RootClaimTotal {
        tao_rao: u64,
    },
    StakeAdded {
        hotkey: AccountId32,
        netuid: u16,
        tao_rao: u64,
    },
    StakeRemoved {
        hotkey: AccountId32,
        netuid: u16,
        tao_rao: u64,
    },
    StakeMoved {
        from_hotkey: AccountId32,
        from_netuid: u16,
        to_hotkey: AccountId32,
        to_netuid: u16,
        tao_rao: u64,
    },
    StakeTransferredOut {
        to_coldkey: AccountId32,
        hotkey: AccountId32,
        netuid: u16,
        tao_rao: u64,
    },
    StakeTransferredIn {
        from_coldkey: AccountId32,
        hotkey: AccountId32,
        netuid: u16,
        tao_rao: u64,
    },
    TransferIn {
        from: AccountId32,
        tao_rao: u64,
    },
    TransferOut {
        to: AccountId32,
        tao_rao: u64,
    },
}

// Event field layouts, taken from the pinned metadata (backend/metadata/finney.scale).
#[derive(scale_decode::DecodeAsType)]
#[allow(dead_code)] // alpha and fee are decoded but not stored yet
struct EvStake(Acc, Acc, u64, u64, u16, u64); // coldkey, hotkey, tao, alpha, netuid, fee
#[derive(scale_decode::DecodeAsType)]
struct EvStakeMoved(Acc, Acc, u16, Acc, u16, u64); // coldkey, from_hotkey, from_netuid, to_hotkey, to_netuid, tao
#[derive(scale_decode::DecodeAsType)]
struct EvStakeTransferred(Acc, Acc, Acc, u16, u16, u64); // from_coldkey, to_coldkey, hotkey, from_netuid, to_netuid, tao
#[derive(scale_decode::DecodeAsType)]
struct EvStakeSwapped(Acc, Acc, u16, u16, u64); // coldkey, hotkey, from_netuid, to_netuid, tao
#[derive(scale_decode::DecodeAsType)]
struct EvRootClaimed {
    coldkey: Acc,
    tao: u64,
}
#[derive(scale_decode::DecodeAsType)]
struct EvBasketClaimed {
    hotkey: Acc,
    coldkey: Acc,
    tao: u64,
}
#[derive(scale_decode::DecodeAsType)]
struct EvTransfer {
    from: Acc,
    to: Acc,
    amount: u64,
}

pub fn parse_address(s: &str) -> Result<AccountId32> {
    AccountId32::from_str(s.trim()).map_err(|e| anyhow::anyhow!("invalid SS58 address: {e:?}"))
}

#[derive(Clone)]
pub struct Chain {
    api: OnlineClient<SubstrateConfig>,
}

impl Chain {
    pub async fn connect(url: &str) -> Result<Self> {
        // ws:// is allowed: a public read-only node (e.g. lite-finney on :9944) may not serve TLS.
        let rpc = subxt_rpcs::RpcClient::from_insecure_url(url).await.with_context(|| format!("connect {url}"))?;
        let backend = LegacyBackend::<SubstrateConfig>::builder().build(rpc);
        let api = OnlineClient::<SubstrateConfig>::from_backend(Arc::new(backend)).await?;
        Ok(Self { api })
    }

    pub fn client(&self) -> &OnlineClient<SubstrateConfig> {
        &self.api
    }

    pub async fn head(&self) -> Result<u32> {
        Ok(self.api.at_current_block().await?.block_number() as u32)
    }

    /// Snapshot at the current best block.
    pub async fn snapshot(&self, coldkey: &AccountId32) -> Result<WalletSnapshot, SnapshotError> {
        let at = self.api.at_current_block().await.map_err(anyhow::Error::from)?;
        let block = at.block_number() as u32;
        snapshot_inner(&at, block, coldkey).await
    }

    /// Snapshot at a historical block (needs an archive node for old blocks).
    pub async fn snapshot_at(&self, block: u32, coldkey: &AccountId32) -> Result<WalletSnapshot, SnapshotError> {
        let at = self.api.at_block(block).await.map_err(anyhow::Error::from)?;
        snapshot_inner(&at, block, coldkey).await
    }

    /// Cumulative claimed RAO for (hotkey, coldkey) at a block; 0 if absent.
    pub async fn basket_claimed_at(&self, block: u32, hotkey: &AccountId32, coldkey: &AccountId32) -> Result<i64> {
        let at = self.api.at_block(block).await?;
        let addr = dynamic::storage::<(Acc, Acc), i64>("SubtensorModule", "BasketClaimed");
        match at.storage().try_fetch(addr, (hotkey.0, coldkey.0)).await? {
            Some(v) => Ok(v.decode()?),
            None => Ok(0),
        }
    }

    /// Block timestamp (ms) and the events of `block` that touch any of `tracked`.
    pub async fn wallet_events(&self, block: u32, tracked: &std::collections::HashSet<Acc>) -> Result<(u64, Vec<WalletEvent>)> {
        let at = self.api.at_block(block).await?;
        let ts: u64 = at.storage().fetch(dynamic::storage::<(), u64>("Timestamp", "Now"), ()).await?.decode()?;
        if tracked.is_empty() {
            return Ok((ts, vec![]));
        }
        let events = at.events().fetch().await?;
        let mut out = Vec::new();
        for ev in events.iter() {
            let ev = ev?;
            let index = ev.index();
            let mut push = |coldkey: Acc, kind: EventKind| {
                if tracked.contains(&coldkey) {
                    out.push(WalletEvent { index, coldkey: AccountId32(coldkey), kind });
                }
            };
            match (ev.pallet_name(), ev.event_name()) {
                ("SubtensorModule", "BasketClaimed") => {
                    let e: EvBasketClaimed = ev.decode_fields_unchecked_as()?;
                    push(e.coldkey, EventKind::Claim { hotkey: AccountId32(e.hotkey), tao_rao: e.tao });
                }
                ("SubtensorModule", "RootClaimed") => {
                    let e: EvRootClaimed = ev.decode_fields_unchecked_as()?;
                    push(e.coldkey, EventKind::RootClaimTotal { tao_rao: e.tao });
                }
                ("SubtensorModule", "StakeAdded") => {
                    let e: EvStake = ev.decode_fields_unchecked_as()?;
                    push(e.0, EventKind::StakeAdded { hotkey: AccountId32(e.1), netuid: e.4, tao_rao: e.2 });
                }
                ("SubtensorModule", "StakeRemoved") => {
                    let e: EvStake = ev.decode_fields_unchecked_as()?;
                    push(e.0, EventKind::StakeRemoved { hotkey: AccountId32(e.1), netuid: e.4, tao_rao: e.2 });
                }
                ("SubtensorModule", "StakeMoved") => {
                    let e: EvStakeMoved = ev.decode_fields_unchecked_as()?;
                    push(
                        e.0,
                        EventKind::StakeMoved {
                            from_hotkey: AccountId32(e.1),
                            from_netuid: e.2,
                            to_hotkey: AccountId32(e.3),
                            to_netuid: e.4,
                            tao_rao: e.5,
                        },
                    );
                }
                ("SubtensorModule", "StakeSwapped") => {
                    let e: EvStakeSwapped = ev.decode_fields_unchecked_as()?;
                    push(
                        e.0,
                        EventKind::StakeMoved {
                            from_hotkey: AccountId32(e.1),
                            from_netuid: e.2,
                            to_hotkey: AccountId32(e.1),
                            to_netuid: e.3,
                            tao_rao: e.4,
                        },
                    );
                }
                ("SubtensorModule", "StakeTransferred") => {
                    let e: EvStakeTransferred = ev.decode_fields_unchecked_as()?;
                    push(
                        e.0,
                        EventKind::StakeTransferredOut {
                            to_coldkey: AccountId32(e.1),
                            hotkey: AccountId32(e.2),
                            netuid: e.3,
                            tao_rao: e.5,
                        },
                    );
                    push(
                        e.1,
                        EventKind::StakeTransferredIn {
                            from_coldkey: AccountId32(e.0),
                            hotkey: AccountId32(e.2),
                            netuid: e.4,
                            tao_rao: e.5,
                        },
                    );
                }
                ("Balances", "Transfer") => {
                    let e: EvTransfer = ev.decode_fields_unchecked_as()?;
                    push(e.from, EventKind::TransferOut { to: AccountId32(e.to), tao_rao: e.amount });
                    push(e.to, EventKind::TransferIn { from: AccountId32(e.from), tao_rao: e.amount });
                }
                _ => {}
            }
        }
        Ok((ts, out))
    }

    /// Account nonce of `coldkey` at `block` (0 for an account that does not exist yet).
    pub async fn nonce_at(&self, block: u32, coldkey: &AccountId32) -> Result<u32> {
        let at = self.api.at_block(block).await?;
        Ok(match at.storage().try_fetch(dynamic::storage::<(Acc,), AccountInfo>("System", "Account"), (coldkey.0,)).await? {
            Some(v) => v.decode()?.nonce,
            None => 0,
        })
    }

    /// Every block in `(from, to]` where `coldkey` signed a transaction (its nonce went up),
    /// oldest first, by bisecting the nonce. Stake, unstake, move and transfers all need the
    /// coldkey's signature, so their blocks are all here (calls made through a proxy are not).
    /// `n_from` / `n_to` are the nonces at `from` / `to` when already known.
    pub async fn signed_blocks(&self, coldkey: &AccountId32, from: u32, n_from: u32, to: u32, n_to: u32) -> Result<Vec<u32>> {
        let mut out = Vec::new();
        let mut stack = vec![(from, n_from, to, n_to)];
        while let Some((lo, n_lo, hi, n_hi)) = stack.pop() {
            if n_lo == n_hi || hi <= lo {
                continue;
            }
            if hi - lo == 1 {
                out.push(hi);
                continue;
            }
            let mid = lo + (hi - lo) / 2;
            let n_mid = self.nonce_at(mid, coldkey).await?;
            stack.push((lo, n_lo, mid, n_mid));
            stack.push((mid, n_mid, hi, n_hi));
        }
        out.sort_unstable();
        Ok(out)
    }

    /// All blocks in `(from, to]` where the `BasketClaimed(hotkey, coldkey)` accounting
    /// value changed, newest first, found by repeated binary search.
    ///
    /// NOTE: this storage is basket bookkeeping ("reward debt"): it also moves when stake
    /// is added, and its jump on a claim is NOT the TAO received. Use it only to locate
    /// candidate blocks, then confirm claims and amounts with [`Chain::wallet_events`].
    pub async fn basket_change_blocks(&self, hotkey: &AccountId32, coldkey: &AccountId32, from: u32, to: u32) -> Result<Vec<u32>> {
        let mut out = Vec::new();
        let base = self.basket_claimed_at(from, hotkey, coldkey).await?;
        let mut hi_end = to;
        while hi_end > from {
            let cur = self.basket_claimed_at(hi_end, hotkey, coldkey).await?;
            if cur == base {
                break;
            }
            let (mut lo, mut hi) = (from, hi_end);
            while hi - lo > 1 {
                let mid = lo + (hi - lo) / 2;
                if self.basket_claimed_at(mid, hotkey, coldkey).await? == cur { hi = mid } else { lo = mid }
            }
            out.push(hi);
            hi_end = hi - 1;
        }
        Ok(out)
    }
}

/// True when a runtime API (or one of its methods) does not exist at this block's runtime.
fn api_missing(msg: &str) -> bool {
    msg.contains("not found in trait")
        || (msg.contains("Runtime API trait") && msg.contains("not found"))
        || msg.contains("Cannot get Runtime API info")
}

/// Wrap any subxt error with a short description of what we were reading.
fn ctx<E>(what: &'static str) -> impl FnOnce(E) -> anyhow::Error
where
    E: std::error::Error + Send + Sync + 'static,
{
    move |e| anyhow::Error::from(e).context(what)
}

#[derive(Debug, scale_decode::DecodeAsType)]
struct RawIdentity {
    name: Vec<u8>,
}

impl Chain {
    /// The top validators: every hotkey registered on the root subnet (netuid 0), which only
    /// admits the largest stakes (64 slots on finney).
    pub async fn top_validators(&self) -> Result<Vec<AccountId32>> {
        let at = self.api.at_current_block().await?;
        let mut out = Vec::new();
        let mut it = at.storage().iter(dynamic::storage::<(u16, u16), Acc>("SubtensorModule", "Keys"), (0u16,)).await?;
        while let Some(kv) = it.next().await {
            out.push(AccountId32(kv?.value().decode()?));
        }
        Ok(out)
    }

    /// On-chain name of a validator hotkey: `IdentitiesV2(Owner(hotkey)).name`, trimmed.
    /// `None` when the owner has set no identity (or an empty name).
    pub async fn validator_name(&self, hotkey: &AccountId32) -> Result<Option<String>> {
        let at = self.api.at_current_block().await?;
        let owner: Acc = match at.storage().try_fetch(dynamic::storage::<(Acc,), Acc>("SubtensorModule", "Owner"), (hotkey.0,)).await? {
            Some(v) => v.decode()?,
            None => return Ok(None),
        };
        let id = at.storage().try_fetch(dynamic::storage::<(Acc,), RawIdentity>("SubtensorModule", "IdentitiesV2"), (owner,)).await?;
        Ok(match id {
            Some(v) => Some(String::from_utf8_lossy(&v.decode()?.name).trim().to_owned()).filter(|n| !n.is_empty()),
            None => None,
        })
    }
}

/// Live and historical snapshots share this one code path.
async fn snapshot_inner(
    at: &subxt::client::OnlineClientAtBlock<SubstrateConfig>,
    block: u32,
    coldkey: &AccountId32,
) -> Result<WalletSnapshot, SnapshotError> {
    let ck = coldkey.0;

    let timestamp_ms: u64 = at
        .storage()
        .fetch(dynamic::storage::<(), u64>("Timestamp", "Now"), ())
        .await
        .map_err(ctx("Timestamp.Now"))?
        .decode()
        .map_err(ctx("decode Timestamp.Now"))?;

    // A never-funded account has no System.Account entry: that is 0, other errors propagate.
    let free_rao = match at
        .storage()
        .try_fetch(dynamic::storage::<(Acc,), AccountInfo>("System", "Account"), (ck,))
        .await
        .map_err(ctx("System.Account"))?
    {
        Some(v) => v.decode().map_err(ctx("decode System.Account"))?.data.free,
        None => 0,
    };

    let raw: Vec<RawStakeInfo> = match at
        .runtime_apis()
        .call(dynamic::runtime_api_call::<_, Vec<RawStakeInfo>>("StakeInfoRuntimeApi", "get_stake_info_for_coldkey", (ck,)))
        .await
    {
        Ok(v) => v,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("decode") || msg.contains("not found") {
                return Err(SnapshotError::PreDtao);
            }
            return Err(anyhow::Error::from(e).context("get_stake_info_for_coldkey").into());
        }
    };

    let mut positions = Vec::new();
    for s in raw.into_iter().filter(|s| s.stake > 0) {
        let pool = if s.netuid == 0 {
            None
        } else {
            let tao: u64 = at
                .storage()
                .fetch(dynamic::storage::<(u16,), u64>("SubtensorModule", "SubnetTAO"), (s.netuid,))
                .await
                .map_err(ctx("SubnetTAO"))?
                .decode()
                .map_err(ctx("decode SubnetTAO"))?;
            let alpha: u64 = at
                .storage()
                .fetch(dynamic::storage::<(u16,), u64>("SubtensorModule", "SubnetAlphaIn"), (s.netuid,))
                .await
                .map_err(ctx("SubnetAlphaIn"))?
                .decode()
                .map_err(ctx("decode SubnetAlphaIn"))?;
            Some((tao, alpha))
        };
        positions.push(Position { hotkey: AccountId32(s.hotkey), netuid: s.netuid, amount_rao: s.stake, pool });
    }

    // Basket API is absent on older runtimes: that is `None`. Any other error (rate limit,
    // timeout) must propagate so the caller retries instead of storing a false "no data".
    let claimable_rao: Option<u64> =
        match at.runtime_apis().call(dynamic::runtime_api_call::<_, u64>("BetaBasketRuntimeApi", "get_root_basket_owed", (ck,))).await {
            Ok(v) => Some(v),
            Err(e) if api_missing(&e.to_string()) => None,
            Err(e) => return Err(anyhow::Error::from(e).context("get_root_basket_owed").into()),
        };

    let mut previews = Vec::new();
    let mut claimed_rao = Vec::new();
    if claimable_rao.is_some() {
        // The first basket runtime has no `get_root_basket_claim_previews`.
        match at
            .runtime_apis()
            .call(dynamic::runtime_api_call::<_, Vec<RawPreview>>("BetaBasketRuntimeApi", "get_root_basket_claim_previews", (ck,)))
            .await
        {
            Ok(v) => {
                previews = v
                    .into_iter()
                    .map(|p| BasketPreview { hotkey: AccountId32(p.hotkey), accrued_rao: p.accrued_tao, redeemable_rao: p.redeemable_tao })
                    .collect();
            }
            Err(e) if api_missing(&e.to_string()) => {}
            Err(e) => return Err(anyhow::Error::from(e).context("get_root_basket_claim_previews").into()),
        }
        let mut hotkeys: Vec<Acc> = positions.iter().filter(|p| p.netuid == 0).map(|p| p.hotkey.0).collect();
        hotkeys.extend(previews.iter().map(|p| p.hotkey.0));
        hotkeys.sort();
        hotkeys.dedup();
        for hk in hotkeys {
            let addr = dynamic::storage::<(Acc, Acc), i64>("SubtensorModule", "BasketClaimed");
            let v = match at.storage().try_fetch(addr, (hk, ck)).await.map_err(ctx("BasketClaimed"))? {
                Some(v) => v.decode().map_err(ctx("decode BasketClaimed"))?,
                None => 0,
            };
            claimed_rao.push((AccountId32(hk), v));
        }
    }

    Ok(WalletSnapshot { block, timestamp_ms, free_rao, positions, claimable_rao, previews, claimed_rao })
}
