/*
 * btcpc_scheduler.h — adaptive sensor rotation scheduler
 *
 * The Flipper cannot run all sensors at once (the sub-GHz, NFC, and 125kHz
 * RFID front-ends contend for the radio/analog hardware and power). This
 * scheduler decides which single sensor to activate next and adapts based on
 * observed yield and the chain's reward economics:
 *
 *   - Sensors that keep returning nothing (empty room, no card present) get
 *     exponentially longer backoff, so we stop wasting battery on them.
 *   - Sensors still producing get priority.
 *   - "event"-class sensors (a distinct discovery: an NFC/RFID card, a Sub-GHz
 *     device) plateau in the chain reward model at ~20 readings per epoch
 *     (sensor_score caps them), so once a sensor has hit its per-epoch cap it
 *     is deprioritized until the epoch rolls over — further scans earn nothing.
 *   - As battery drops, the whole cycle slows down.
 *
 * The scheduler is PURE LOGIC — no hardware calls — so it is unit-testable off
 * device. The scene layer calls btcpc_sched_next() to get the sensor to run,
 * performs the actual capture, then calls btcpc_sched_report() with the yield.
 *
 * Shin Devlin — btcpc.network
 */
#pragma once

#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>

/* The rotatable sensors. HEARTBEAT is always-cheap and runs on a fixed slow
 * cadence regardless of yield (it reports battery/uptime, never "finds"
 * anything). Order here is the base round-robin order. */
typedef enum {
    BtcpcSensorSubGhz = 0, /* continuous: sub-GHz RSSI observation      */
    BtcpcSensorNfc,        /* event: ISO14443 card in field             */
    BtcpcSensorRfid,       /* event: 125kHz card in field               */
    BtcpcSensorIButton,    /* event: 1-Wire iButton contact             */
    BtcpcSensorHeartbeat,  /* housekeeping: battery/uptime              */
    BtcpcSensorCount,
} BtcpcSensorKind;

/* Reward class, mirrors the chain's sensor_score() cases. Drives cap logic. */
typedef enum {
    BtcpcClassContinuous, /* sqrt curve, no hard cap — always some value */
    BtcpcClassEvent,      /* hard cap ~20/epoch, then worthless          */
    BtcpcClassHousekeeping,
} BtcpcSensorClass;

/* Per-sensor adaptive state. */
typedef struct {
    BtcpcSensorClass class;
    uint32_t backoff;        /* current backoff level (cycles to skip)   */
    uint32_t skip_remaining; /* cycles left to skip before next attempt  */
    uint32_t epoch_yield;    /* successful captures this epoch            */
    uint32_t total_yield;    /* successful captures since boot            */
    uint32_t attempts;       /* attempts since boot                      */
} BtcpcSensorState;

typedef struct {
    BtcpcSensorState sensors[BtcpcSensorCount];
    uint32_t cycle;          /* monotonically increasing cycle counter   */
    uint32_t heartbeat_every;/* run heartbeat once per this many cycles  */
    uint32_t epoch_cap;      /* per-epoch cap for event sensors (~20)     */
    uint32_t max_backoff;    /* ceiling on backoff cycles                 */
    uint8_t  battery_pct;    /* last known battery, drives slowdown       */
} BtcpcScheduler;

/* Initialise with sensible defaults. */
void btcpc_sched_init(BtcpcScheduler* s);

/* Roll over per-epoch counters (call when the chain epoch advances so event
 * sensors that hit their cap become worth scanning again). */
void btcpc_sched_new_epoch(BtcpcScheduler* s);

/* Update battery reading (0-100); affects inter-scan pacing, not selection. */
void btcpc_sched_set_battery(BtcpcScheduler* s, uint8_t pct);

/*
 * Pick the next sensor to activate. Advances the internal cycle counter and
 * decrements skip counters. Returns the chosen sensor. Never returns a sensor
 * that is backed off unless every sensor is backed off (then it returns the
 * one closest to ready, so the loop always makes progress).
 */
BtcpcSensorKind btcpc_sched_next(BtcpcScheduler* s);

/*
 * Report the outcome of the capture the scene just performed for `kind`.
 * `found` = true if the sensor produced a real reading (a card, a signal, a
 * live RSSI sample). Adapts backoff and yield counters. `found == false`
 * lengthens backoff; `found == true` resets it.
 */
void btcpc_sched_report(BtcpcScheduler* s, BtcpcSensorKind kind, bool found);

/*
 * Suggested delay in milliseconds before the next cycle, given battery level.
 * Scales up as battery drops so a low Flipper sips power.
 */
uint32_t btcpc_sched_cycle_delay_ms(const BtcpcScheduler* s);

/* True if `kind` has hit its per-epoch economic cap (event sensors only). */
bool btcpc_sched_at_cap(const BtcpcScheduler* s, BtcpcSensorKind kind);
