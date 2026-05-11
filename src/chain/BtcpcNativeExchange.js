"use strict";

/**
 * BtcpcNativeExchange — FIFO Exchange logic for BTCPC Native Chain.
 * Handles user BTCPC deposits, hybrid pricing (bonding curve -> oracle),
 * and FIFO fulfillment with revenue splitting.
 */

const FEE_RECIPIENT = 'shindevlin';
const FEE_PERCENT = 0.3; // 0.3% fee

const BONDING_CURVE_INITIAL_PRICE = 0.10; // $0.10 USD starting price
const BONDING_CURVE_INCREMENT = 0.01;    // $0.01 increase per 10k tokens sold
const BONDING_CURVE_LIMIT = 500000;      // Switch to oracle after 500k tokens

/**
 * Calculate the current BTCPC/USD price based on total sold volume.
 * @param {number} totalSold - Total BTCPC sold through the exchange protocol.
 * @param {object} oracleFeeds - Reference to the oracle feeds service.
 * @returns {number} Price in USD per 1 BTCPC.
 */
function calculatePrice(totalSold, oracleFeeds) {
    if (totalSold < BONDING_CURVE_LIMIT) {
        // Linear bonding curve: price = base + (blocks_of_10k * increment)
        const increments = Math.floor(totalSold / 10000);
        return BONDING_CURVE_INITIAL_PRICE + (increments * BONDING_CURVE_INCREMENT);
    } else {
        // AMM-aligned price via oracle
        if (oracleFeeds && typeof oracleFeeds.getFeedValue === 'function') {
            const oraclePrice = oracleFeeds.getFeedValue('btcpc/usd');
            if (oraclePrice && oraclePrice > 0) return oraclePrice;
        }
        // Fallback to the end of the bonding curve if oracle is unavailable
        const finalBondingPrice = BONDING_CURVE_INITIAL_PRICE + (Math.floor(BONDING_CURVE_LIMIT / 10000) * BONDING_CURVE_INCREMENT);
        return finalBondingPrice;
    }
}

/**
 * Execute a purchase from the FIFO queue.
 * This is called by stateStore.applyEntry during block processing.
 * 
 * @param {object} params - { buyer, token, amount, entry, state }
 * @param {object} state - References to in-memory maps and functions from stateStore
 */
function applyExchangeBuy(params, state) {
    const { buyer, token, amount } = params;
    const { 
        exchangeQueue, 
        exchangeStats,
        oracleFeeds,
        _round,
        _debit,
        _credit
    } = state;

    if (exchangeQueue.length === 0) return; // Nothing to buy

    // Buyer pays in stablecoins
    // (In stateStore, stablecoin tokens are handled by _debit and _credit as well)
    if (!_debit(buyer, token, amount)) return;

    const currentPrice = calculatePrice(exchangeStats.total_volume, oracleFeeds);
    const btcpcToBuy = _round(amount / currentPrice);
    
    let remainingToFulfill = btcpcToBuy;
    let totalFees = 0;

    // FIFO Fulfillment
    while (remainingToFulfill > 0 && exchangeQueue.length > 0) {
        const deposit = exchangeQueue[0];
        const take = Math.min(remainingToFulfill, deposit.amount);
        
        const usdValue = take * currentPrice;
        const fee = _round(usdValue * (FEE_PERCENT / 100));
        const sellerShare = _round(usdValue - fee);

        // Credit seller with stablecoins
        _credit(deposit.seller, token, sellerShare);

        totalFees = _round(totalFees + fee);
        remainingToFulfill = _round(remainingToFulfill - take);
        deposit.amount = _round(deposit.amount - take);

        if (deposit.amount <= 0) {
            exchangeQueue.shift(); // Remove fully fulfilled deposit
        }
    }

    const actualBought = _round(btcpcToBuy - remainingToFulfill);
    
    // Credit buyer with BTCPC
    _credit(buyer, "BTCPC", actualBought);
    
    // Debit BTCPC from exchange system account (where it was locked on deposit)
    _debit("btcpc_exchange", "BTCPC", actualBought);

    // Credit fee recipient with stablecoins
    _credit(FEE_RECIPIENT, token, totalFees);

    // Update stats
    exchangeStats.total_volume = _round(exchangeStats.total_volume + actualBought);
    exchangeStats.total_fees = _round(exchangeStats.total_fees + totalFees);
}

module.exports = {
    calculatePrice,
    applyExchangeBuy
};
