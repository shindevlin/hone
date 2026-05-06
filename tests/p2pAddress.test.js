"use strict";

const os = require("os");

const {
  getAdvertisedP2PAddress,
  isConnectableP2PAddress,
  normalizeP2PAddress,
} = require("../src/p2p/address");

describe("p2p address normalization", () => {
  afterEach(() => {
    jest.restoreAllMocks();
    delete process.env.BTCPC_PUBLIC_ADDRESS;
    delete process.env.P2P_ADVERTISE_IP;
    delete process.env.P2P_PORT;
  });

  test("rejects localhost and loopback peer addresses", () => {
    expect(normalizeP2PAddress("ws://localhost:6942")).toBeNull();
    expect(isConnectableP2PAddress("ws://localhost:6942")).toBe(false);
    expect(isConnectableP2PAddress("ws://127.0.0.1:6942")).toBe(false);
    expect(isConnectableP2PAddress("ws://[::1]:6942")).toBe(false);
    expect(isConnectableP2PAddress("ws://[::ffff:127.0.0.1]:6942")).toBe(false);
  });

  test("normalizes bare hosts to the configured P2P port", () => {
    process.env.P2P_PORT = "6942";
    expect(normalizeP2PAddress("node.example")).toBe("ws://node.example:6942");
    expect(normalizeP2PAddress("192.0.2.10")).toBe("ws://192.0.2.10:6942");
  });

  test("keeps explicit ws/wss URLs connectable", () => {
    expect(normalizeP2PAddress("wss://node.example:9443/ws")).toBe("wss://node.example:9443/ws");
  });

  test("prefers BTCPC_PUBLIC_ADDRESS when it is valid", () => {
    process.env.BTCPC_PUBLIC_ADDRESS = "wss://node.example:9443/ws";
    expect(getAdvertisedP2PAddress()).toBe("wss://node.example:9443/ws");
  });

  test("falls back to the first non-internal interface when no env address is set", () => {
    jest.spyOn(os, "networkInterfaces").mockReturnValue({
      lo: [{ address: "127.0.0.1", family: "IPv4", internal: true }],
      eth0: [{ address: "198.51.100.12", family: "IPv4", internal: false }],
    });
    process.env.P2P_PORT = "6942";
    expect(getAdvertisedP2PAddress()).toBe("ws://198.51.100.12:6942");
  });
});
