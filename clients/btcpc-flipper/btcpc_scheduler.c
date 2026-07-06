/*
 * btcpc_scheduler.c — adaptive sensor rotation scheduler (see .h)
 *
 * Pure logic, no hardware. Unit-testable off device.
 *
 * Shin Devlin — btcpc.network
 */

#include "btcpc_scheduler.h"

/* Defaults. */
#define BTCPC_HEARTBEAT_EVERY   30   /* heartbeat ~once every 30 cycles       */
#define BTCPC_EPOCH_CAP         20   /* event sensors plateau at 20/epoch     */
#define BTCPC_MAX_BACKOFF       16   /* skip at most 16 cycles when barren     */
#define BTCPC_BASE_DELAY_MS     500  /* min inter-cycle delay at full battery  */

static BtcpcSensorClass class_of(BtcpcSensorKind k) {
    switch(k) {
    case BtcpcSensorSubGhz:    return BtcpcClassContinuous;
    case BtcpcSensorNfc:       return BtcpcClassEvent;
    case BtcpcSensorRfid:      return BtcpcClassEvent;
    case BtcpcSensorIButton:   return BtcpcClassEvent;
    case BtcpcSensorHeartbeat: return BtcpcClassHousekeeping;
    default:                   return BtcpcClassContinuous;
    }
}

void btcpc_sched_init(BtcpcScheduler* s) {
    if(!s) return;
    for(size_t i = 0; i < BtcpcSensorCount; i++) {
        s->sensors[i].class          = class_of((BtcpcSensorKind)i);
        s->sensors[i].backoff        = 0;
        s->sensors[i].skip_remaining = 0;
        s->sensors[i].epoch_yield    = 0;
        s->sensors[i].total_yield    = 0;
        s->sensors[i].attempts       = 0;
    }
    s->cycle           = 0;
    s->heartbeat_every = BTCPC_HEARTBEAT_EVERY;
    s->epoch_cap       = BTCPC_EPOCH_CAP;
    s->max_backoff     = BTCPC_MAX_BACKOFF;
    s->battery_pct     = 100;
}

void btcpc_sched_new_epoch(BtcpcScheduler* s) {
    if(!s) return;
    for(size_t i = 0; i < BtcpcSensorCount; i++) {
        s->sensors[i].epoch_yield = 0;
        /* An event sensor that was deprioritised only because it hit its cap
         * should get a fresh chance next epoch — clear its backoff. */
        if(s->sensors[i].class == BtcpcClassEvent) {
            s->sensors[i].backoff        = 0;
            s->sensors[i].skip_remaining = 0;
        }
    }
}

void btcpc_sched_set_battery(BtcpcScheduler* s, uint8_t pct) {
    if(!s) return;
    s->battery_pct = pct > 100 ? 100 : pct;
}

bool btcpc_sched_at_cap(const BtcpcScheduler* s, BtcpcSensorKind kind) {
    if(!s || kind >= BtcpcSensorCount) return false;
    const BtcpcSensorState* st = &s->sensors[kind];
    return st->class == BtcpcClassEvent && st->epoch_yield >= s->epoch_cap;
}

/* Is this sensor eligible to run this cycle? */
static bool eligible(const BtcpcScheduler* s, BtcpcSensorKind k) {
    const BtcpcSensorState* st = &s->sensors[k];

    if(k == BtcpcSensorHeartbeat) {
        /* Housekeeping runs on a fixed slow cadence, ignores yield. */
        return (s->cycle % s->heartbeat_every) == 0;
    }
    if(btcpc_sched_at_cap(s, k)) return false; /* economically pointless now */
    return st->skip_remaining == 0;
}

BtcpcSensorKind btcpc_sched_next(BtcpcScheduler* s) {
    if(!s) return BtcpcSensorHeartbeat;

    s->cycle++;

    /* Decrement skip counters at the top of each cycle. */
    for(size_t i = 0; i < BtcpcSensorCount; i++) {
        if(s->sensors[i].skip_remaining > 0) s->sensors[i].skip_remaining--;
    }

    /* Heartbeat has priority on its cadence tick (cheap, always useful). */
    if(eligible(s, BtcpcSensorHeartbeat)) {
        return BtcpcSensorHeartbeat;
    }

    /* Round-robin starting point rotates with the cycle so no sensor starves.
     * Pick the first eligible sensor in rotation order from the start point. */
    BtcpcSensorKind start = (BtcpcSensorKind)(s->cycle % BtcpcSensorCount);
    for(size_t off = 0; off < BtcpcSensorCount; off++) {
        BtcpcSensorKind k = (BtcpcSensorKind)((start + off) % BtcpcSensorCount);
        if(k == BtcpcSensorHeartbeat) continue; /* handled above */
        if(eligible(s, k)) return k;
    }

    /* Everything is backed off or capped. Return the non-heartbeat sensor
     * with the fewest skip cycles remaining so the loop still makes progress
     * (prefer one that isn't hard-capped this epoch). */
    BtcpcSensorKind best = BtcpcSensorSubGhz;
    uint32_t best_skip = 0xFFFFFFFFu;
    for(size_t i = 0; i < BtcpcSensorCount; i++) {
        BtcpcSensorKind k = (BtcpcSensorKind)i;
        if(k == BtcpcSensorHeartbeat) continue;
        if(btcpc_sched_at_cap(s, k)) continue;
        if(s->sensors[i].skip_remaining < best_skip) {
            best_skip = s->sensors[i].skip_remaining;
            best = k;
        }
    }
    return best;
}

void btcpc_sched_report(BtcpcScheduler* s, BtcpcSensorKind kind, bool found) {
    if(!s || kind >= BtcpcSensorCount) return;
    BtcpcSensorState* st = &s->sensors[kind];
    st->attempts++;

    if(found) {
        st->epoch_yield++;
        st->total_yield++;
        /* Productive: reset backoff, run it again soon. */
        st->backoff        = 0;
        st->skip_remaining = 0;
    } else {
        /* Barren: exponential backoff up to the ceiling. */
        if(st->backoff == 0) {
            st->backoff = 1;
        } else if(st->backoff < s->max_backoff) {
            st->backoff *= 2;
            if(st->backoff > s->max_backoff) st->backoff = s->max_backoff;
        }
        st->skip_remaining = st->backoff;
    }
}

uint32_t btcpc_sched_cycle_delay_ms(const BtcpcScheduler* s) {
    if(!s) return BTCPC_BASE_DELAY_MS;
    /* Scale delay inversely with battery: full battery = base delay,
     * low battery stretches it out to sip power. At 100% -> 1x, at 20% -> ~3x,
     * floored so we never busy-spin. */
    uint32_t pct = s->battery_pct == 0 ? 1 : s->battery_pct;
    /* multiplier = 1 + (100 - pct)/40, integer-scaled by 100 to avoid floats */
    uint32_t mult_x100 = 100 + ((100 - pct) * 100) / 40;
    return (BTCPC_BASE_DELAY_MS * mult_x100) / 100;
}
