"use strict";

const { getReservedNamesForShin } = require("../src/services/reservedNames");

describe("reservedNames", () => {
  test("includes curated names and all one, two, and three letter combinations", () => {
    const names = getReservedNamesForShin();
    expect(names).toContain("bob");
    expect(names).toContain("a");
    expect(names).toContain("zz");
    expect(names).toContain("zzz");
    expect(names).toHaveLength(new Set(names).size);
    expect(names.length).toBeGreaterThanOrEqual(26 + 26 * 26 + 26 * 26 * 26);
  });
});
