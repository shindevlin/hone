"use strict";

const verifier = require("../src/inference/verifier");

describe("inference verifier scaling", () => {
  test("returns no panel when no nodes opt into verification", () => {
    const policy = verifier.getVerifierPolicy(25, 0);
    expect(policy.count).toBe(0);
    expect(policy.phase).toBe("no-verifiers");
  });

  test("scales verifier panel size with network adoption but caps at opt-in supply", () => {
    expect(verifier.getVerifierPolicy(1, 1).count).toBe(1);
    expect(verifier.getVerifierPolicy(10, 10).count).toBe(3);
    expect(verifier.getVerifierPolicy(50, 2).count).toBe(2);
    expect(verifier.getVerifierPolicy(200, 20).count).toBe(7);
  });

  test("job coverage is deterministic for the same job and block", () => {
    const a = verifier.shouldVerifyJob("job-1", "abc", 0.25);
    const b = verifier.shouldVerifyJob("job-1", "abc", 0.25);
    expect(a).toBe(b);
    expect(verifier.shouldVerifyJob("job-1", "abc", 0)).toBe(false);
    expect(verifier.shouldVerifyJob("job-1", "abc", 1)).toBe(true);
  });
});
