"use strict";

const authorityRotation = require("../src/chain/authorityRotation");

describe("authorityRotation proposer validation", () => {
  const accounts = ["natoshisakamoto", "shindevlin", "josh"];
  const prevHash = "a".repeat(64);

  test("exports validateProposer used by P2P BLOCK_PROPOSAL handling", () => {
    expect(typeof authorityRotation.validateProposer).toBe("function");
  });

  test("accepts the deterministic designated proposer", () => {
    const designated = authorityRotation.selectProposer(42, prevHash, accounts);
    const result = authorityRotation.validateProposer(
      designated,
      42,
      prevHash,
      accounts,
      Date.now()
    );

    expect(result.valid).toBe(true);
    expect(result.designated).toBe(designated);
    expect(result.fallback).toBe(false);
  });

  test("rejects non-designated proposer before fallback window", () => {
    const designated = authorityRotation.selectProposer(43, prevHash, accounts);
    const other = accounts.find(a => a !== designated);
    const result = authorityRotation.validateProposer(
      other,
      43,
      prevHash,
      accounts,
      Date.now()
    );

    expect(result.valid).toBe(false);
    expect(result.designated).toBe(designated);
  });

  test("accepts eligible fallback proposer after missed authority window", () => {
    const designated = authorityRotation.selectProposer(44, prevHash, accounts);
    const other = accounts.find(a => a !== designated);
    const epochStart = Date.now() - 30000;
    const result = authorityRotation.validateProposer(
      other,
      44,
      prevHash,
      accounts,
      epochStart
    );

    expect(result.valid).toBe(true);
    expect(result.designated).toBe(designated);
    expect(result.fallback).toBe(true);
  });

  test("self-heals when local eligible registry is empty", () => {
    const result = authorityRotation.validateProposer(
      "natoshisakamoto",
      45,
      prevHash,
      [],
      Date.now()
    );

    expect(result.valid).toBe(true);
    expect(result.designated).toBe("natoshisakamoto");
    expect(result.fallback).toBe(true);
  });
});
