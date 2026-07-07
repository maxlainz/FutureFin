import { describe, expect, it } from "vitest";
import { MOBILE_MAX_WIDTH, isMobileWidth } from "./responsive";

describe("isMobileWidth", () => {
  it("treats widths at or below the breakpoint as mobile", () => {
    expect(isMobileWidth(320)).toBe(true);
    expect(isMobileWidth(390)).toBe(true);
    expect(isMobileWidth(639)).toBe(true);
    expect(isMobileWidth(MOBILE_MAX_WIDTH)).toBe(true);
  });
  it("treats widths above the breakpoint as desktop", () => {
    expect(isMobileWidth(MOBILE_MAX_WIDTH + 1)).toBe(false);
    expect(isMobileWidth(720)).toBe(false);
    expect(isMobileWidth(1280)).toBe(false);
  });
  it("breakpoint is the canonical bp:mobile 640", () => {
    expect(MOBILE_MAX_WIDTH).toBe(640);
  });
});
