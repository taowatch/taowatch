//! Shared code for the TaoWatch api and indexer: configuration, database access
//! and the Subtensor chain client.

pub mod chain;
pub mod config;
pub mod db;

/// 1 TAO = 1e9 RAO.
pub const RAO_PER_TAO: u64 = 1_000_000_000;

/// Convert an on-chain RAO amount into an exact decimal TAO value.
pub fn rao_to_tao(rao: u128) -> rust_decimal::Decimal {
    rust_decimal::Decimal::from_i128_with_scale(rao as i128, 9)
}
