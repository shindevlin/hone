/*
 * hone_scheduler.c — adaptive sensor rotation scheduler (see .h)
 *
 * Pure logic, no hardware. Unit-testable off device.
 *
 * Shin Devlin — honemesh.network
 */

#include "hone_scheduler.h"

/* Defaults. */
#define HONE_HEARTBEAT_EVERY   30   /* heartbeat ~once every 30 cycles       */
#define HONE_EPOCH_CAP         20   /* event sensors plateau at 20/epoch     */
#define HONE_MAX_BACKOFF       16   /* skip at most 16 cycles when barren     */
#define HONE_BASE_DELAY_MS     500  /* min inter-cycle delay at full battery  */

static HoneSensorClass class_of(HoneSensorKind k) {
    switch(k) {
    case HoneSensorSubGhz:    return HoneClassContinuous;
    case HoneSensorNfc:       return HoneClassEvent;
    case HoneSensorRfid:      return HoneClassEvent;
    case HoneSensorIButton:   return HoneClassEvent;
    case HoneSensorHeartbeat: return HoneClassHousekeeping;
    default:                   return HoneClassContinuous;
    }
}

void hone_sched_init(HoneScheduler* s) {
    if(!s) return;
    for(size_t i = 0; i < HoneSensorCount; i++) {
        s->sensors[i].class          = class_of((HoneSensorKind)i);
        s->sensors[i].backoff        = 0;
        s->sensors[i].skip_remaining = 0;
        s->sensors[i].epoch_yield    = 0;
        s->sensors[i].total_yield    = 0;
        s->sensors[i].attempts       = 0;
    }
    s->cycle           = 0;
    s->heartbeat_every = HONE_HEARTBEAT_EVERY;
    s->epoch_cap       = HONE_EPOCH_CAP;
    s->max_backoff     = HONE_MAX_BACKOFF;
    s->battery_pct     = 100;
}

void hone_sched_new_epoch(HoneScheduler* s) {
    if(!s) return;
    for(size_t i = 0; i < HoneSensorCount; i++) {
        s->sensors[i].epoch_yield = 0;
        /* An event sensor that was deprioritised only because it hit its cap
         * should get a fresh chance next epoch — clear its backoff. */
        if(s->sensors[i].class == HoneClassEvent) {
            s->sensors[i].backoff        = 0;
            s->sensors[i].skip_remaining = 0;
        }
    }
}

void hone_sched_set_battery(HoneScheduler* s, uint8_t pct) {
    if(!s) return;
    s->battery_pct = pct > 100 ? 100 : pct;
}

bool hone_sched_at_cap(const HoneScheduler* s, HoneSensorKind kind) {
    if(!s || kind >= HoneSensorCount) return false;
    const HoneSensorState* st = &s->sensors[kind];
    return st->class == HoneClassEvent && st->epoch_yield >= s->epoch_cap;
}

/* Is this sensor eligible to run this cycle? */
static bool eligible(const HoneScheduler* s, HoneSensorKind k) {
    const HoneSensorState* st = &s->sensors[k];

    if(k == HoneSensorHeartbeat) {
        /* Housekeeping runs on a fixed slow cadence, ignores yield. */
        return (s->cycle % s->heartbeat_every) == 0;
    }
    if(hone_sched_at_cap(s, k)) return false; /* economically pointless now */
    return st->skip_remaining == 0;
}

HoneSensorKind hone_sched_next(HoneScheduler* s) {
    if(!s) return HoneSensorHeartbeat;

    s->cycle++;

    /* Decrement skip counters at the top of each cycle. */
    for(size_t i = 0; i < HoneSensorCount; i++) {
        if(s->sensors[i].skip_remaining > 0) s->sensors[i].skip_remaining--;
    }

    /* Heartbeat has priority on its cadence tick (cheap, always useful). */
    if(eligible(s, HoneSensorHeartbeat)) {
        return HoneSensorHeartbeat;
    }

    /* Round-robin starting point rotates with the cycle so no sensor starves.
     * Pick the first eligible sensor in rotation order from the start point. */
    HoneSensorKind start = (HoneSensorKind)(s->cycle % HoneSensorCount);
    for(size_t off = 0; off < HoneSensorCount; off++) {
        HoneSensorKind k = (HoneSensorKind)((start + off) % HoneSensorCount);
        if(k == HoneSensorHeartbeat) continue; /* handled above */
        if(eligible(s, k)) return k;
    }

    /* Everything is backed off or capped. Return the non-heartbeat sensor
     * with the fewest skip cycles remaining so the loop still makes progress
     * (prefer one that isn't hard-capped this epoch). */
    HoneSensorKind best = HoneSensorSubGhz;
    uint32_t best_skip = 0xFFFFFFFFu;
    for(size_t i = 0; i < HoneSensorCount; i++) {
        HoneSensorKind k = (HoneSensorKind)i;
        if(k == HoneSensorHeartbeat) continue;
        if(hone_sched_at_cap(s, k)) continue;
        if(s->sensors[i].skip_remaining < best_skip) {
            best_skip = s->sensors[i].skip_remaining;
            best = k;
        }
    }
    return best;
}

void hone_sched_report(HoneScheduler* s, HoneSensorKind kind, bool found) {
    if(!s || kind >= HoneSensorCount) return;
    HoneSensorState* st = &s->sensors[kind];
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

uint32_t hone_sched_cycle_delay_ms(const HoneScheduler* s) {
    if(!s) return HONE_BASE_DELAY_MS;
    /* Scale delay inversely with battery: full battery = base delay,
     * low battery stretches it out to sip power. At 100% -> 1x, at 20% -> ~3x,
     * floored so we never busy-spin. */
    uint32_t pct = s->battery_pct == 0 ? 1 : s->battery_pct;
    /* multiplier = 1 + (100 - pct)/40, integer-scaled by 100 to avoid floats */
    uint32_t mult_x100 = 100 + ((100 - pct) * 100) / 40;
    return (HONE_BASE_DELAY_MS * mult_x100) / 100;
}
