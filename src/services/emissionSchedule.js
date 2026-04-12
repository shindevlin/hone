"use strict";

/**
 * BTCPC Emission Schedule
 *
 * Doubling-halving interval model (v3.0: 30-second epochs):
 *   Period 1: 1 month  (86400 epochs), allotment 2,100,000 BTCPC, reward ~24.306/epoch
 *   Period 2: 2 months (172800 epochs), allotment 2,381,400 BTCPC, reward ~13.785/epoch
 *   Period 3: 4 months (345600 epochs), allotment 2,700,508 BTCPC, reward ~7.819/epoch
 *   ...each period: duration doubles, allotment *= 1.134, reward = allotment / epochs_in_period
 *
 * Total supply: 42,000,000 BTCPC
 * Same annual emission and halving schedule as the 5-min epoch model.
 * 10× more epochs per year → reward per epoch is 1/10 of the original.
 */

const EPOCHS_PER_MONTH = 86400; // 30 days * 24 hours * 120 epochs/hour (30-second epochs)
const GROWTH_RATIO = 1.134;
const GENESIS_ALLOTMENT = 2100000;
const TOTAL_SUPPLY = 42000000;
const NUM_PERIODS = 11;

/**
 * Pre-compute the period table for fast lookup.
 * Each entry: { period, start_epoch, end_epoch, duration_months, allotment, reward_per_epoch }
 */
function buildPeriodTable() {
  const periods = [];
  let cumulative_epochs = 0;
  let allotment = GENESIS_ALLOTMENT;
  let duration_months = 1;
  let cumulative_supply = 0;

  for (let i = 1; i <= NUM_PERIODS; i++) {
    let period_allotment;
    let period_epochs;

    if (i === NUM_PERIODS) {
      // Final period is truncated to cap at TOTAL_SUPPLY
      period_allotment = TOTAL_SUPPLY - cumulative_supply;
      // Duration adjusted: ~345 months per whitepaper
      duration_months = 345;
      period_epochs = duration_months * EPOCHS_PER_MONTH;
    } else {
      period_allotment = allotment;
      period_epochs = duration_months * EPOCHS_PER_MONTH;
    }

    const reward_per_epoch = period_allotment / period_epochs;

    periods.push({
      period: i,
      start_epoch: cumulative_epochs,
      end_epoch: cumulative_epochs + period_epochs - 1,
      duration_months,
      epochs: period_epochs,
      allotment: period_allotment,
      reward_per_epoch
    });

    cumulative_epochs += period_epochs;
    cumulative_supply += period_allotment;

    // Next period: allotment grows by GROWTH_RATIO, duration doubles
    allotment = allotment * GROWTH_RATIO;
    duration_months = duration_months * 2;
  }

  return periods;
}

const PERIOD_TABLE = buildPeriodTable();

/**
 * Get which period a given epoch falls in.
 * Returns the period object or null if past all periods.
 */
function getCurrentPeriod(epochNumber) {
  for (const period of PERIOD_TABLE) {
    if (epochNumber >= period.start_epoch && epochNumber <= period.end_epoch) {
      return period;
    }
  }
  return null;
}

/**
 * Get the block reward for a specific epoch number.
 * Returns 0 if the epoch is past the final period (all tokens mined).
 */
function getBlockReward(epochNumber) {
  const period = getCurrentPeriod(epochNumber);
  if (!period) return 0;
  return parseFloat(period.reward_per_epoch.toFixed(10));
}

/**
 * Return the number of epochs per month (8640).
 */
function getEpochsPerMonth() {
  return EPOCHS_PER_MONTH;
}

/**
 * Return the full period table for inspection.
 */
function getPeriodTable() {
  return PERIOD_TABLE;
}

module.exports = {
  getBlockReward,
  getCurrentPeriod,
  getEpochsPerMonth,
  getPeriodTable,
  TOTAL_SUPPLY,
  GROWTH_RATIO,
  GENESIS_ALLOTMENT,
  EPOCHS_PER_MONTH
};
