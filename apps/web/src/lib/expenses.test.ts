import { describe, expect, it } from "vitest";
import type {
  CategoryRow,
  ImportPreviewRowApi,
  TransactionMonthApi,
} from "../api/types";
import {
  adjacentMonthInList,
  buildConfirmDecisions,
  categoriesForKind,
  defaultSelectedMonth,
  deltaToneClass,
  initialDraftForRow,
  monthLabelEs,
  monthShortLabelEs,
  parseMonth,
  rowMatchesFilter,
  signOf,
  summarizeDecisions,
  type ImportRowDraft,
} from "./expenses";

function cat(id: string, scope: CategoryRow["scope"], name: string): CategoryRow {
  return { id, scope, name, sort_index: 0 };
}

function previewRow(
  overrides: Partial<ImportPreviewRowApi> & { index: number },
): ImportPreviewRowApi {
  return {
    op_date: "2026-06-15",
    concept: "TEST",
    amount: "-10.0000",
    currency: "EUR",
    status: "new",
    suggested_kind: "expense",
    suggested_transfer: false,
    currency_warning: false,
    ...overrides,
  };
}

describe("parseMonth", () => {
  it("parses YYYY-MM", () => {
    expect(parseMonth("2026-06")).toEqual({ y: 2026, m: 6 });
  });
  it("rejects garbage", () => {
    expect(parseMonth("2026-13")).toBeNull();
    expect(parseMonth("bad")).toBeNull();
    expect(parseMonth("2026-6")).toBeNull();
  });
});

describe("monthLabelEs / monthShortLabelEs", () => {
  it("labels a month in Spanish with the year", () => {
    expect(monthLabelEs("2026-06").toLowerCase()).toContain("junio");
    expect(monthLabelEs("2026-06")).toContain("2026");
  });
  it("returns the raw string when unparseable", () => {
    expect(monthLabelEs("nope")).toBe("nope");
  });
  it("short label is a non-empty prefix", () => {
    expect(monthShortLabelEs("2026-06").length).toBeGreaterThan(0);
  });
});

describe("defaultSelectedMonth", () => {
  const months: TransactionMonthApi[] = [
    { month: "2026-07", is_complete: false, txn_count: 3 },
    { month: "2026-06", is_complete: true, txn_count: 40 },
    { month: "2026-05", is_complete: true, txn_count: 38 },
  ];
  it("picks the most recent COMPLETE month", () => {
    expect(defaultSelectedMonth(months)).toBe("2026-06");
  });
  it("falls back to the most recent when none complete", () => {
    expect(
      defaultSelectedMonth([
        { month: "2026-07", is_complete: false, txn_count: 3 },
      ]),
    ).toBe("2026-07");
  });
  it("null when empty", () => {
    expect(defaultSelectedMonth([])).toBeNull();
  });
});

describe("adjacentMonthInList", () => {
  const list = ["2026-07", "2026-06", "2026-05"]; // DESC
  it("older moves to a larger index (further in the past)", () => {
    expect(adjacentMonthInList(list, "2026-06", "older")).toBe("2026-05");
  });
  it("newer moves to a smaller index", () => {
    expect(adjacentMonthInList(list, "2026-06", "newer")).toBe("2026-07");
  });
  it("null at the extremes", () => {
    expect(adjacentMonthInList(list, "2026-05", "older")).toBeNull();
    expect(adjacentMonthInList(list, "2026-07", "newer")).toBeNull();
  });
  it("null when current not in list", () => {
    expect(adjacentMonthInList(list, "2020-01", "older")).toBeNull();
  });
});

describe("categoriesForKind", () => {
  const income = [cat("i1", "income", "Nómina")];
  const expense = [cat("e1", "expense", "Súper")];
  it("savings => no categories", () => {
    expect(categoriesForKind("savings", income, expense)).toEqual([]);
  });
  it("income => income categories", () => {
    expect(categoriesForKind("income", income, expense)).toBe(income);
  });
  it("expense => expense categories", () => {
    expect(categoriesForKind("expense", income, expense)).toBe(expense);
  });
});

describe("initialDraftForRow", () => {
  it("new plain row is included and carries the suggestion", () => {
    const d = initialDraftForRow(
      previewRow({
        index: 0,
        suggested_kind: "expense",
        suggested_category_id: "c1",
      }),
    );
    expect(d.include).toBe(true);
    expect(d.kind).toBe("expense");
    expect(d.categoryId).toBe("c1");
    expect(d.force).toBe(false);
  });
  it("duplicate row is excluded", () => {
    expect(
      initialDraftForRow(previewRow({ index: 1, status: "already_imported" }))
        .include,
    ).toBe(false);
  });
  it("suggested transfer is excluded", () => {
    expect(
      initialDraftForRow(previewRow({ index: 2, suggested_transfer: true }))
        .include,
    ).toBe(false);
  });
  it("currency warning is excluded", () => {
    expect(
      initialDraftForRow(previewRow({ index: 3, currency_warning: true }))
        .include,
    ).toBe(false);
  });
  it("savings suggestion never carries a category", () => {
    const d = initialDraftForRow(
      previewRow({
        index: 4,
        suggested_kind: "savings",
        suggested_category_id: "c9",
      }),
    );
    expect(d.categoryId).toBe("");
  });
});

describe("buildConfirmDecisions", () => {
  it("produces one decision per row, parallel by index", () => {
    const rows = [
      previewRow({ index: 0 }),
      previewRow({ index: 1, status: "already_imported" }),
    ];
    const drafts: ImportRowDraft[] = [
      {
        include: true,
        kind: "expense",
        categoryId: "c1",
        linkedAssetId: "",
        linkedLiabilityId: "l1",
        force: false,
      },
      {
        include: true,
        kind: "expense",
        categoryId: "",
        linkedAssetId: "",
        linkedLiabilityId: "",
        force: true,
      },
    ];
    const out = buildConfirmDecisions(rows, drafts);
    expect(out).toHaveLength(2);
    expect(out[0]).toEqual({
      kind: "expense",
      category_id: "c1",
      linked_liability_id: "l1",
    });
    expect(out[1]).toEqual({ kind: "expense", force: true });
  });
  it("excluded row => discard true, no category noise", () => {
    const rows = [previewRow({ index: 0 })];
    const drafts: ImportRowDraft[] = [
      {
        include: false,
        kind: "expense",
        categoryId: "c1",
        linkedAssetId: "",
        linkedLiabilityId: "",
        force: false,
      },
    ];
    expect(buildConfirmDecisions(rows, drafts)[0]).toEqual({
      kind: "expense",
      discard: true,
      category_id: "c1",
    });
  });
  it("savings never emits a category_id", () => {
    const rows = [previewRow({ index: 0 })];
    const drafts: ImportRowDraft[] = [
      {
        include: true,
        kind: "savings",
        categoryId: "c1",
        linkedAssetId: "a1",
        linkedLiabilityId: "",
        force: false,
      },
    ];
    expect(buildConfirmDecisions(rows, drafts)[0]).toEqual({
      kind: "savings",
      linked_asset_id: "a1",
    });
  });
});

describe("summarizeDecisions", () => {
  it("counts import / skip / discard with backend semantics", () => {
    const rows = [
      previewRow({ index: 0 }), // new, included -> import
      previewRow({ index: 1, status: "already_imported" }), // dup, not forced -> skip
      previewRow({ index: 2, status: "already_imported" }), // dup, forced -> import
      previewRow({ index: 3 }), // excluded -> discard
    ];
    const drafts: ImportRowDraft[] = [
      draft({ include: true }),
      draft({ include: true, force: false }),
      draft({ include: true, force: true }),
      draft({ include: false }),
    ];
    expect(summarizeDecisions(rows, drafts)).toEqual({
      toImport: 2,
      toSkip: 1,
      toDiscard: 1,
    });
  });
});

describe("rowMatchesFilter", () => {
  const d = draft({});
  it("all matches everything", () => {
    expect(rowMatchesFilter(previewRow({ index: 0 }), d, "all")).toBe(true);
  });
  it("duplicates filter", () => {
    expect(
      rowMatchesFilter(
        previewRow({ index: 0, status: "already_imported" }),
        d,
        "duplicates",
      ),
    ).toBe(true);
    expect(rowMatchesFilter(previewRow({ index: 0 }), d, "duplicates")).toBe(
      false,
    );
  });
  it("transfers filter", () => {
    expect(
      rowMatchesFilter(
        previewRow({ index: 0, suggested_transfer: true }),
        d,
        "transfers",
      ),
    ).toBe(true);
  });
  it("uncategorized filter ignores savings", () => {
    expect(
      rowMatchesFilter(
        previewRow({ index: 0 }),
        draft({ kind: "expense", categoryId: "" }),
        "uncategorized",
      ),
    ).toBe(true);
    expect(
      rowMatchesFilter(
        previewRow({ index: 0 }),
        draft({ kind: "savings", categoryId: "" }),
        "uncategorized",
      ),
    ).toBe(false);
  });
});

describe("signOf / deltaToneClass", () => {
  it("signOf rounds to euros", () => {
    expect(signOf(0.4)).toBe("zero");
    expect(signOf(1.2)).toBe("pos");
    expect(signOf(-2)).toBe("neg");
  });
  it("expense over budget is unfavorable (red)", () => {
    expect(deltaToneClass(50, "expense")).toBe("num-neg");
    expect(deltaToneClass(-50, "expense")).toBe("num-pos");
  });
  it("income over budget is favorable (green)", () => {
    expect(deltaToneClass(50, "income")).toBe("num-pos");
    expect(deltaToneClass(-50, "income")).toBe("num-neg");
  });
  it("zero delta => no color class", () => {
    expect(deltaToneClass(0, "expense")).toBe("");
    expect(deltaToneClass(0, "income")).toBe("");
  });
});

function draft(overrides: Partial<ImportRowDraft>): ImportRowDraft {
  return {
    include: true,
    kind: "expense",
    categoryId: "",
    linkedAssetId: "",
    linkedLiabilityId: "",
    force: false,
    ...overrides,
  };
}
