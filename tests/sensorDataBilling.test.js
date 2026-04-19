"use strict";

jest.mock("../src/services/ledger", () => ({
  getCurrentEpoch: jest.fn().mockResolvedValue(88),
  recordEscrowLock: jest.fn(),
  recordEscrowRelease: jest.fn(),
  recordEscrowRefund: jest.fn(),
  recordTransfer: jest.fn(),
}));

jest.mock("../src/services/escrow", () => ({
  lockFunds: jest.fn().mockResolvedValue({
    request_id: "req-1",
    payer: "buyer",
    amount: 10,
    status: "locked",
  }),
}));

jest.mock("../src/chain/stateStore", () => ({
  getChainHeight: jest.fn().mockReturnValue(5700),
}));

const ledger = require("../src/services/ledger");
const escrow = require("../src/services/escrow");
const stateStore = require("../src/chain/stateStore");
const billing = require("../src/services/sensorDataBilling");

describe("sensor data billing", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    ledger.getCurrentEpoch.mockResolvedValue(88);
    stateStore.getChainHeight.mockReturnValue(5700);
  });

  test("quote math reflects sensor type, age, resolution, and volume", () => {
    const quote = billing.quoteSensorQuery(
      {
        type: "gps",
        resolution: "aggregate",
        age_hours: 8,
        reading_count: 250,
        enterprise: false,
      },
      {
        account: "buyer",
      }
    );

    expect(quote.quote_id).toMatch(/^sdq_/);
    expect(quote.estimated_readings).toBe(250);
    expect(quote.price_per_reading).toBeCloseTo(
      billing.DEFAULT_RATE_CARD.sensor_types.gps *
        billing.DEFAULT_RATE_CARD.age_multipliers[2].multiplier *
        billing.DEFAULT_RATE_CARD.resolution_multipliers.aggregate *
        billing.DEFAULT_RATE_CARD.volume_discounts[2].multiplier,
      10
    );
    expect(quote.max_cost).toBeCloseTo(quote.price_per_reading * 250, 10);
  });

  test("calculatePayouts splits sensor pool proportionally across owners and keeps protocol split balanced", () => {
    const payouts = billing.calculatePayouts(
      [
        { owner: "alice" },
        { owner: "alice" },
        { owner: "bob" },
      ],
      1.0
    );

    expect(payouts.sensor_pool).toBeCloseTo(0.7, 10);
    expect(payouts.protocol_pool).toBeCloseTo(0.2, 10);
    expect(payouts.recycle_pool).toBeCloseTo(0.1, 10);
    expect(payouts.owner_payouts).toHaveLength(2);

    const byOwner = Object.fromEntries(
      payouts.owner_payouts.map((item) => [item.owner, item.amount])
    );
    expect(byOwner.alice).toBeCloseTo(0.4666666667, 10);
    expect(byOwner.bob).toBeCloseTo(0.2333333333, 10);
    expect(payouts.protocol_payouts).toEqual([
      expect.objectContaining({ account: "shindevlin", amount: 0.1 }),
      expect.objectContaining({ account: "natoshisakamoto", amount: 0.1 }),
    ]);
    expect(payouts.total_payout).toBeCloseTo(1.0, 10);
  });

  test("calculatePayouts routes rounding remainder into recycle", () => {
    const payouts = billing.calculatePayouts(
      [{ owner: "alice" }],
      0.03
    );

    expect(payouts.owner_payouts[0].amount).toBeCloseTo(0.021, 10);
    expect(payouts.protocol_payouts[0].amount).toBeCloseTo(0.003, 10);
    expect(payouts.protocol_payouts[1].amount).toBeCloseTo(0.003, 10);
    expect(payouts.recycle_pool).toBeCloseTo(0.003, 10);
    expect(payouts.total_payout).toBeCloseTo(0.03, 10);
  });

  test("settleSensorDataPayment locks quote max, releases proportional payouts, and refunds overquote", async () => {
    const result = await billing.settleSensorDataPayment(
      {
        payer: "buyer",
        query_id: "req-1",
        quote: {
          quote_id: "sdq_test",
          max_cost: 10,
          price_per_reading: 2,
        },
        actual_fee: 6,
        readings: [
          { owner: "alice" },
          { owner: "alice" },
          { owner: "bob" },
        ],
      },
      {
        escrow,
        ledger,
        stateStore,
        epoch: 88,
      }
    );

    expect(escrow.lockFunds).toHaveBeenCalledWith("req-1", "buyer", 10);
    expect(ledger.recordEscrowRelease).toHaveBeenCalledWith("alice", "req-1", 2.8, 88, "Sensor data payout");
    expect(ledger.recordEscrowRelease).toHaveBeenCalledWith("bob", "req-1", 1.4, 88, "Sensor data payout");
    expect(ledger.recordEscrowRelease).toHaveBeenCalledWith("shindevlin", "req-1", 0.6, 88, "Sensor data protocol share");
    expect(ledger.recordEscrowRelease).toHaveBeenCalledWith("natoshisakamoto", "req-1", 0.6, 88, "Sensor data protocol share");
    expect(ledger.recordEscrowRelease).toHaveBeenCalledWith("btcpc_recycle", "req-1", 0.6, 88, "Sensor data recycle share");
    expect(ledger.recordEscrowRefund).toHaveBeenCalledWith("buyer", "req-1", 4, 88);
    expect(result.total_fee).toBe(6);
    expect(result.refund).toBe(4);
    expect(result.block_height).toBe(5700);
  });

  test("blurCoordinates reduces lat/lon to 2 decimal places", () => {
    const reading = {
      sensor_id: "natoshisakamoto/gnss-base",
      metadata: { latitude: 13.692940, longitude: -89.218191, altitude: 672 },
    };
    const blurred = billing.blurCoordinates(reading);
    expect(blurred.metadata.latitude).toBe(13.69);
    expect(blurred.metadata.longitude).toBe(-89.22);
    expect(blurred.metadata.altitude).toBe(672);
  });

  test("blurCoordinates is a no-op when metadata has no coordinates", () => {
    const reading = { sensor_id: "a/b", metadata: { battery_pct: 87 } };
    const result = billing.blurCoordinates(reading);
    expect(result).toBe(reading);
  });

  test("blurCoordinates is a no-op for null/non-object readings", () => {
    expect(billing.blurCoordinates(null)).toBeNull();
    expect(billing.blurCoordinates("str")).toBe("str");
  });

  test("executePaidSensorQuery blurs coordinates when allow_precise_location is false", async () => {
    const mockStateStore = {
      getChainHeight: jest.fn().mockReturnValue(200),
      getSensor: jest.fn().mockReturnValue({
        sensor_id: "natoshisakamoto/gnss-base",
        owner: "natoshisakamoto",
        type: "gps",
        status: "active",
        allow_precise_location: false,
      }),
      getAllSensors: jest.fn().mockReturnValue([]),
      getSensorReadings: jest.fn().mockReturnValue([
        { epoch: 150, value: 13.692940, metadata: { latitude: 13.692940, longitude: -89.218191 }, submitted_at: Date.now() },
      ]),
      getBalance: jest.fn().mockReturnValue(999),
    };
    const mockLedger = {
      getCurrentEpoch: jest.fn().mockResolvedValue(200),
      recordEscrowRelease: jest.fn(),
      recordEscrowRefund: jest.fn(),
    };
    const result = await billing.executePaidSensorQuery(
      { sensor_id: "natoshisakamoto/gnss-base", from_epoch: 100, to_epoch: 200 },
      { account: "buyer", stateStore: mockStateStore, ledger: mockLedger, use_escrow: false }
    );
    const r = result.readings[0];
    expect(r.metadata.latitude).toBe(13.69);
    expect(r.metadata.longitude).toBe(-89.22);
  });

  test("executePaidSensorQuery returns precise coordinates when allow_precise_location is true", async () => {
    const mockStateStore = {
      getChainHeight: jest.fn().mockReturnValue(200),
      getSensor: jest.fn().mockReturnValue({
        sensor_id: "natoshisakamoto/gnss-base",
        owner: "natoshisakamoto",
        type: "gps",
        status: "active",
        allow_precise_location: true,
      }),
      getAllSensors: jest.fn().mockReturnValue([]),
      getSensorReadings: jest.fn().mockReturnValue([
        { epoch: 150, value: 13.692940, metadata: { latitude: 13.692940, longitude: -89.218191 }, submitted_at: Date.now() },
      ]),
      getBalance: jest.fn().mockReturnValue(999),
    };
    const mockLedger = {
      getCurrentEpoch: jest.fn().mockResolvedValue(200),
      recordEscrowRelease: jest.fn(),
      recordEscrowRefund: jest.fn(),
    };
    const result = await billing.executePaidSensorQuery(
      { sensor_id: "natoshisakamoto/gnss-base", from_epoch: 100, to_epoch: 200 },
      { account: "buyer", stateStore: mockStateStore, ledger: mockLedger, use_escrow: false }
    );
    const r = result.readings[0];
    expect(r.metadata.latitude).toBeCloseTo(13.692940, 5);
    expect(r.metadata.longitude).toBeCloseTo(-89.218191, 5);
  });

  test("settleSensorDataPayment with zero readings issues full refund and charges nothing", async () => {
    const result = await billing.settleSensorDataPayment(
      {
        payer: "buyer",
        query_id: "req-empty",
        quote: {
          quote_id: "sdq_empty",
          max_cost: 5,
          price_per_reading: 0.5,
        },
        actual_fee: 5,
        readings: [],
      },
      {
        escrow,
        ledger,
        stateStore,
        epoch: 88,
      }
    );

    expect(escrow.lockFunds).toHaveBeenCalledWith("req-empty", "buyer", 5);
    expect(ledger.recordEscrowRelease).not.toHaveBeenCalled();
    expect(ledger.recordEscrowRefund).toHaveBeenCalledWith("buyer", "req-empty", 5, 88);
    expect(result.total_fee).toBe(0);
    expect(result.charged).toBe(0);
    expect(result.refunded).toBe(true);
    expect(result.refund).toBe(5);
    expect(result.transfers).toEqual([]);
  });

  test("settleSensorDataPayment with non-empty readings settles normally", async () => {
    const result = await billing.settleSensorDataPayment(
      {
        payer: "buyer",
        query_id: "req-normal",
        quote: {
          quote_id: "sdq_normal",
          max_cost: 4,
          price_per_reading: 2,
        },
        actual_fee: 4,
        readings: [
          { owner: "carol" },
          { owner: "carol" },
        ],
      },
      {
        escrow,
        ledger,
        stateStore,
        epoch: 88,
      }
    );

    expect(escrow.lockFunds).toHaveBeenCalledWith("req-normal", "buyer", 4);
    expect(ledger.recordEscrowRelease).toHaveBeenCalledWith("carol", "req-normal", 2.8, 88, "Sensor data payout");
    expect(result.total_fee).toBe(4);
    expect(result.refund).toBe(0);
    expect(result.refunded).toBeUndefined();
  });
});
