"use strict";

const dns = require("dns").promises;
const { isPrivateIpLiteral, isPublicHttpUrl } = require("../src/services/urlSafety");

describe("urlSafety", () => {
  afterEach(() => {
    jest.restoreAllMocks();
  });

  test("blocks loopback and private IP literals", () => {
    expect(isPrivateIpLiteral("localhost")).toBe(true);
    expect(isPrivateIpLiteral("127.0.0.1")).toBe(true);
    expect(isPrivateIpLiteral("2130706433")).toBe(true);
    expect(isPrivateIpLiteral("0x7f000001")).toBe(true);
    expect(isPrivateIpLiteral("10.0.0.1")).toBe(true);
    expect(isPrivateIpLiteral("fd00::1")).toBe(true);
  });

  test("allows public hosts when DNS resolves to public addresses", async () => {
    jest.spyOn(dns, "lookup").mockResolvedValue([
      { address: "93.184.216.34", family: 4 },
    ]);
    await expect(isPublicHttpUrl("https://example.com/path")).resolves.toBe(true);
  });

  test("rejects hosts that resolve to private addresses", async () => {
    jest.spyOn(dns, "lookup").mockResolvedValue([
      { address: "127.0.0.1", family: 4 },
    ]);
    await expect(isPublicHttpUrl("https://internal.example")).resolves.toBe(false);
  });

  test("rejects loopback URLs without DNS lookup", async () => {
    const lookup = jest.spyOn(dns, "lookup");
    await expect(isPublicHttpUrl("http://127.0.0.1:8080")).resolves.toBe(false);
    expect(lookup).not.toHaveBeenCalled();
  });

  test("blocks IPv6 loopback literal in URLs", async () => {
    const lookup = jest.spyOn(dns, "lookup");
    await expect(isPublicHttpUrl("http://[::1]/admin")).resolves.toBe(false);
    expect(lookup).not.toHaveBeenCalled();
  });

  test("blocks IPv4-mapped IPv6 loopback (::ffff:127.0.0.1)", async () => {
    const lookup = jest.spyOn(dns, "lookup");
    await expect(isPublicHttpUrl("http://[::ffff:127.0.0.1]/secret")).resolves.toBe(false);
    expect(lookup).not.toHaveBeenCalled();
  });

  test("blocks IPv4-mapped IPv6 private range (::ffff:10.0.0.1)", async () => {
    const lookup = jest.spyOn(dns, "lookup");
    await expect(isPublicHttpUrl("http://[::ffff:10.0.0.1]/internal")).resolves.toBe(false);
    expect(lookup).not.toHaveBeenCalled();
  });

  test("blocks link-local IPv6 (fe80::1)", () => {
    expect(isPrivateIpLiteral("fe80::1")).toBe(true);
  });

  test("blocks ULA IPv6 (fd00::1)", () => {
    expect(isPrivateIpLiteral("fd00::1")).toBe(true);
  });
});
