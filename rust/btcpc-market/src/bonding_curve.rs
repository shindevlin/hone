/// Linear bonding curve: price_per_slot(n) = BASE + SLOPE * n (USD)
/// Exact port of src/services/stakeBondingCurve.js

const DEFAULT_BASE_USD: f64 = 1.00;
const DEFAULT_SLOPE_USD: f64 = 0.05;
pub const DEFAULT_STAKE_PER_SLOT_BTCPC: f64 = 1.0;

fn round4(v: f64) -> f64 {
    (v * 10000.0).round() / 10000.0
}

/// USD cost to add `units` slots on top of `current_capacity`.
pub fn cost_for_capacity(current_capacity: u32, units: u32) -> f64 {
    if units == 0 {
        return 0.0;
    }
    let start = current_capacity as f64;
    let end = (current_capacity + units) as f64;
    let slice_sum = ((end - 1.0) * end - (start - 1.0) * start) / 2.0;
    round4(DEFAULT_BASE_USD * units as f64 + DEFAULT_SLOPE_USD * slice_sum)
}

/// How many slots a USD payment buys at current_capacity.
pub fn capacity_for_payment(current_capacity: u32, payment_usd: f64) -> u32 {
    if payment_usd <= 0.0 {
        return 0;
    }
    let mut lo: u32 = 0;
    let mut hi: u32 = 1;
    while cost_for_capacity(current_capacity, hi) <= payment_usd {
        lo = hi;
        hi = hi.saturating_mul(2);
        if hi > 10_000_000 {
            break;
        }
    }
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if cost_for_capacity(current_capacity, mid) <= payment_usd {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// BTCPC stake locked for `capacity` slots.
pub fn stake_for_capacity(capacity: u32) -> f64 {
    capacity as f64 * DEFAULT_STAKE_PER_SLOT_BTCPC
}
