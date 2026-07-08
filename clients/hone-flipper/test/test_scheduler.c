/*
 * test_scheduler.c — off-device unit tests for the adaptive scheduler.
 *
 * The scheduler is pure logic, so it compiles and runs with a plain host C
 * compiler — no Flipper SDK. Build & run:
 *   cc -I.. test_scheduler.c ../btcpc_scheduler.c -o /tmp/sched && /tmp/sched
 *
 * Shin Devlin — btcpc.network
 */
#include "../btcpc_scheduler.h"
#include <stdio.h>
#include <string.h>

static int failures = 0;
#define CHECK(cond, msg) do { \
    if(!(cond)) { printf("FAIL: %s\n", msg); failures++; } \
    else        { printf("ok:   %s\n", msg); } \
} while(0)

/* Count how many times each sensor is chosen over N cycles, always reporting
 * `found` for the chosen sensor. */
static void run_all_found(BtcpcScheduler* s, int n, int counts[BtcpcSensorCount]) {
    memset(counts, 0, sizeof(int) * BtcpcSensorCount);
    for(int i = 0; i < n; i++) {
        BtcpcSensorKind k = btcpc_sched_next(s);
        counts[k]++;
        btcpc_sched_report(s, k, true);
    }
}

int main(void) {
    /* 1. Barren sensor gets backed off; productive sensor keeps getting slots. */
    {
        BtcpcScheduler s;
        btcpc_sched_init(&s);
        int subghz_hits = 0, nfc_hits = 0;
        for(int i = 0; i < 200; i++) {
            BtcpcSensorKind k = btcpc_sched_next(&s);
            if(k == BtcpcSensorSubGhz) { subghz_hits++; btcpc_sched_report(&s, k, true);  }
            else if(k == BtcpcSensorNfc) { nfc_hits++;   btcpc_sched_report(&s, k, false); }
            else { btcpc_sched_report(&s, k, false); }
        }
        /* Sub-GHz always finds -> gets far more slots than barren NFC. */
        CHECK(subghz_hits > nfc_hits * 3,
              "productive sub-GHz out-scheduled barren NFC by >3x");
        CHECK(s.sensors[BtcpcSensorNfc].backoff >= 8,
              "barren NFC backed off toward the ceiling");
        CHECK(s.sensors[BtcpcSensorSubGhz].backoff == 0,
              "productive sub-GHz never backed off");
    }

    /* 2. Event sensor hits its per-epoch cap, then is skipped until new epoch. */
    {
        BtcpcScheduler s;
        btcpc_sched_init(&s);
        /* Force NFC to always find so it climbs to the cap. */
        for(int i = 0; i < 500; i++) {
            BtcpcSensorKind k = btcpc_sched_next(&s);
            btcpc_sched_report(&s, k, true);
        }
        CHECK(btcpc_sched_at_cap(&s, BtcpcSensorNfc),
              "NFC reached its per-epoch economic cap");
        CHECK(s.sensors[BtcpcSensorNfc].epoch_yield >= s.epoch_cap,
              "NFC epoch_yield >= cap");
        /* Once capped, further next() calls must not pick NFC. */
        int picked_nfc_after_cap = 0;
        for(int i = 0; i < 50; i++) {
            BtcpcSensorKind k = btcpc_sched_next(&s);
            if(k == BtcpcSensorNfc) picked_nfc_after_cap++;
            btcpc_sched_report(&s, k, true);
        }
        CHECK(picked_nfc_after_cap == 0,
              "capped NFC not scheduled again this epoch");

        /* New epoch resets it — NFC becomes schedulable again. */
        btcpc_sched_new_epoch(&s);
        CHECK(!btcpc_sched_at_cap(&s, BtcpcSensorNfc),
              "new epoch clears NFC cap");
        int picked_nfc_new_epoch = 0;
        for(int i = 0; i < 20; i++) {
            BtcpcSensorKind k = btcpc_sched_next(&s);
            if(k == BtcpcSensorNfc) picked_nfc_new_epoch++;
            btcpc_sched_report(&s, k, true);
        }
        CHECK(picked_nfc_new_epoch > 0, "NFC scheduled again after new epoch");
    }

    /* 3. Heartbeat runs on its fixed cadence regardless of yield. */
    {
        BtcpcScheduler s;
        btcpc_sched_init(&s);
        s.heartbeat_every = 10;
        int hb = 0;
        for(int i = 0; i < 100; i++) {
            BtcpcSensorKind k = btcpc_sched_next(&s);
            if(k == BtcpcSensorHeartbeat) hb++;
            btcpc_sched_report(&s, k, false); /* nothing ever "found" */
        }
        /* ~100/10 heartbeats expected. */
        CHECK(hb >= 8 && hb <= 12, "heartbeat fired ~once per cadence window");
    }

    /* 4. Low battery stretches the cycle delay; full battery is the base. */
    {
        BtcpcScheduler s;
        btcpc_sched_init(&s);
        btcpc_sched_set_battery(&s, 100);
        uint32_t full = btcpc_sched_cycle_delay_ms(&s);
        btcpc_sched_set_battery(&s, 20);
        uint32_t low = btcpc_sched_cycle_delay_ms(&s);
        CHECK(low > full, "low battery increases inter-cycle delay");
        CHECK(full >= 500, "full-battery delay is at least the base");
    }

    /* 5. Every sensor eventually runs even when all are barren (no starvation,
     *    no infinite skip). */
    {
        BtcpcScheduler s;
        btcpc_sched_init(&s);
        int counts[BtcpcSensorCount];
        memset(counts, 0, sizeof(counts));
        for(int i = 0; i < 2000; i++) {
            BtcpcSensorKind k = btcpc_sched_next(&s);
            counts[k]++;
            btcpc_sched_report(&s, k, false);
        }
        int ran_all = 1;
        for(int i = 0; i < BtcpcSensorCount; i++) if(counts[i] == 0) ran_all = 0;
        CHECK(ran_all, "no sensor starves even under total barrenness");
    }

    if(failures == 0) { printf("\nALL SCHEDULER TESTS PASSED\n"); return 0; }
    printf("\n%d FAILURE(S)\n", failures);
    return 1;
}
