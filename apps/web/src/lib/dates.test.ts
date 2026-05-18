import { describe, expect, it } from "vitest";
import {
  addMonthsCivil,
  addOneMonthUtc,
  ageCompletedYearsCivil,
  civilDaysInMonth,
  formatProjectionAxisYear,
  formatProjectionHoverMonthYear,
  parseYmdComponents,
  parseYmdUtc,
  paymentIntervalCountUtc,
  todayYmdInTimeZone,
  utcTodayYmd,
} from "./dates";

describe("utcTodayYmd + todayYmdInTimeZone", () => {
  it("utcTodayYmd matches YYYY-MM-DD format", () => {
    expect(utcTodayYmd()).toMatch(/^\d{4}-\d{2}-\d{2}$/);
  });
  it("falls back to UTC when TZ is invalid", () => {
    expect(todayYmdInTimeZone("Not/AZone")).toMatch(/^\d{4}-\d{2}-\d{2}$/);
  });
  it("returns a date for a real TZ", () => {
    expect(todayYmdInTimeZone("Europe/Madrid")).toMatch(/^\d{4}-\d{2}-\d{2}$/);
  });
});

describe("parseYmdUtc + addOneMonthUtc", () => {
  it("parses YYYY-MM-DD as UTC", () => {
    const d = parseYmdUtc("2026-03-15");
    expect(d.getUTCFullYear()).toBe(2026);
    expect(d.getUTCMonth()).toBe(2);
    expect(d.getUTCDate()).toBe(15);
  });
  it("addOneMonthUtc clamps day when target month is shorter", () => {
    const next = addOneMonthUtc(parseYmdUtc("2026-01-31"));
    // Feb 2026 → 28 days
    expect(next.getUTCFullYear()).toBe(2026);
    expect(next.getUTCMonth()).toBe(1);
    expect(next.getUTCDate()).toBe(28);
  });
  it("addOneMonthUtc rolls year over December → January", () => {
    const next = addOneMonthUtc(parseYmdUtc("2026-12-31"));
    expect(next.getUTCFullYear()).toBe(2027);
    expect(next.getUTCMonth()).toBe(0);
    expect(next.getUTCDate()).toBe(31);
  });
});

describe("paymentIntervalCountUtc", () => {
  it("monthly: full year from Jan to Dec = 12 intervals", () => {
    expect(paymentIntervalCountUtc("monthly", "2026-01-15", "2026-12-15")).toBe(12);
  });
  it("weekly: 8 days = ceil(8/7) = 2 intervals", () => {
    expect(paymentIntervalCountUtc("weekly", "2026-01-01", "2026-01-08")).toBe(2);
  });
  it("returns null when end < start", () => {
    expect(paymentIntervalCountUtc("monthly", "2026-03-01", "2026-01-01")).toBeNull();
  });
  it("returns null when input invalid", () => {
    expect(paymentIntervalCountUtc("monthly", "bad", "2026-12-15")).toBeNull();
  });
  it("monthly cap stops infinite loop at 1200", () => {
    expect(paymentIntervalCountUtc("monthly", "1900-01-01", "9999-12-31")).toBeNull();
  });
});

describe("parseYmdComponents", () => {
  it("parses valid date", () => {
    expect(parseYmdComponents("2026-05-17")).toEqual({ y: 2026, m: 5, d: 17 });
  });
  it("rejects null/empty/malformed", () => {
    expect(parseYmdComponents(null)).toBeNull();
    expect(parseYmdComponents(undefined)).toBeNull();
    expect(parseYmdComponents("")).toBeNull();
    expect(parseYmdComponents("not-a-date")).toBeNull();
    expect(parseYmdComponents("2026-13-01")).toBeNull();
    expect(parseYmdComponents("2026-05-32")).toBeNull();
  });
});

describe("civilDaysInMonth", () => {
  it("returns 28 for Feb non-leap", () => {
    expect(civilDaysInMonth(2026, 2)).toBe(28);
  });
  it("returns 29 for Feb leap", () => {
    expect(civilDaysInMonth(2024, 2)).toBe(29);
  });
  it("returns 30 for April, 31 for March", () => {
    expect(civilDaysInMonth(2026, 4)).toBe(30);
    expect(civilDaysInMonth(2026, 3)).toBe(31);
  });
});

describe("addMonthsCivil", () => {
  it("clamps day when target month is shorter (Jan 31 + 1 mes = Feb 28)", () => {
    expect(addMonthsCivil(2026, 1, 31, 1)).toEqual({ y: 2026, m: 2, d: 28 });
  });
  it("rolls year over (Dec + 1 = next Jan)", () => {
    expect(addMonthsCivil(2026, 12, 15, 1)).toEqual({ y: 2027, m: 1, d: 15 });
  });
  it("handles large deltas", () => {
    expect(addMonthsCivil(2026, 1, 15, 24)).toEqual({ y: 2028, m: 1, d: 15 });
  });
});

describe("ageCompletedYearsCivil", () => {
  it("after birthday this year → full years", () => {
    expect(
      ageCompletedYearsCivil({ y: 2026, m: 5, d: 17 }, { y: 1990, m: 1, d: 1 }),
    ).toBe(36);
  });
  it("before birthday this year → one less", () => {
    expect(
      ageCompletedYearsCivil({ y: 2026, m: 5, d: 17 }, { y: 1990, m: 6, d: 1 }),
    ).toBe(35);
  });
  it("birthday today → just turned", () => {
    expect(
      ageCompletedYearsCivil({ y: 2026, m: 5, d: 17 }, { y: 1990, m: 5, d: 17 }),
    ).toBe(36);
  });
  it("day before birthday → still not", () => {
    expect(
      ageCompletedYearsCivil({ y: 2026, m: 5, d: 16 }, { y: 1990, m: 5, d: 17 }),
    ).toBe(35);
  });
  it("birth date in the future → 0", () => {
    expect(
      ageCompletedYearsCivil({ y: 2020, m: 5, d: 17 }, { y: 2026, m: 1, d: 1 }),
    ).toBe(0);
  });
});

describe("formatProjectionAxisYear + formatProjectionHoverMonthYear", () => {
  it("axis year is the YYYY portion", () => {
    expect(formatProjectionAxisYear({ y: 2026, m: 5, d: 17 })).toBe("2026");
  });
  it("hover month/year is zero-padded MM/YYYY", () => {
    expect(formatProjectionHoverMonthYear({ y: 2026, m: 5, d: 17 })).toBe("05/2026");
    expect(formatProjectionHoverMonthYear({ y: 2027, m: 12, d: 1 })).toBe("12/2027");
  });
});
