import { describe, expect, it } from "vitest";
import type {
  CategoryRow,
  ImportPreviewRowApi,
  SummaryTotalsApi,
  TransactionApi,
  TransactionKindApi,
  TransactionMonthApi,
} from "../api/types";
import {
  adjacentMonthInList,
  avgWindowLabel,
  buildConfirmDecisions,
  capitalizeSource,
  categoriesForKind,
  compareTransactions,
  defaultSelectedMonth,
  deltaToneClass,
  groupTransactionsByCategory,
  initialDraftForRow,
  isReconciled,
  kpiBudgetTrend,
  monthLabelEs,
  monthShortLabelEs,
  naturalSortDir,
  normalizeSearchText,
  parseMonth,
  rowMatchesFilter,
  signOf,
  significanceThreshold,
  significantDeltaTone,
  sortTransactionGroups,
  sortTransactions,
  summarizeDecisions,
  transactionMatchesQuery,
  trendArrow,
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
  it("suggested transfer is INCLUDED (3.5.0): la exclusión del gasto es cosa de la conciliación", () => {
    expect(
      initialDraftForRow(previewRow({ index: 2, suggested_transfer: true }))
        .include,
    ).toBe(true);
  });
  it("a duplicate that is also a suggested transfer stays excluded (dup manda)", () => {
    expect(
      initialDraftForRow(
        previewRow({
          index: 2,
          status: "already_imported",
          suggested_transfer: true,
        }),
      ).include,
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
  it("con los drafts POR DEFECTO, una transferencia sugerida cuenta como importada", () => {
    const rows = [
      previewRow({ index: 0 }),
      previewRow({ index: 1, suggested_transfer: true }),
      previewRow({ index: 2, currency_warning: true }),
    ];
    expect(summarizeDecisions(rows, rows.map(initialDraftForRow))).toEqual({
      toImport: 2,
      toSkip: 0,
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
    expect(rowMatchesFilter(previewRow({ index: 1 }), d, "transfers")).toBe(false);
  });
  it("el filtro transfers sigue funcionando con el draft por defecto (ahora incluido)", () => {
    const row = previewRow({ index: 0, suggested_transfer: true });
    const def = initialDraftForRow(row);
    expect(def.include).toBe(true);
    expect(rowMatchesFilter(row, def, "transfers")).toBe(true);
    expect(rowMatchesFilter(row, def, "new")).toBe(true);
    expect(rowMatchesFilter(row, def, "duplicates")).toBe(false);
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

describe("significanceThreshold", () => {
  it("is 1% of income_actual when there is real income", () => {
    expect(significanceThreshold(totals({ income_actual: "2000" }))).toBeCloseTo(
      20,
    );
  });
  it("falls back to 1% of income_budget when actual income is 0", () => {
    expect(
      significanceThreshold(
        totals({ income_actual: "0", income_budget: "1500" }),
      ),
    ).toBeCloseTo(15);
  });
  it("falls back to income_budget when actual income is negative", () => {
    expect(
      significanceThreshold(
        totals({ income_actual: "-10", income_budget: "1500" }),
      ),
    ).toBeCloseTo(15);
  });
  it("is 0 when both income actual and budget are 0", () => {
    expect(
      significanceThreshold(totals({ income_actual: "0", income_budget: "0" })),
    ).toBe(0);
  });
});

describe("trendArrow", () => {
  it("empty slot (null) when there is no average — no data is not 'no change'", () => {
    expect(trendArrow(500, "expense", 10, false)).toEqual({
      direction: null,
      tone: "",
    });
    expect(trendArrow(5, "income", 10, false)).toEqual({
      direction: null,
      tone: "",
    });
  });
  it("flat (=, neutral tone) when there IS an average but |Δ| is within the threshold (inclusive)", () => {
    expect(trendArrow(10, "expense", 10, true)).toEqual({
      direction: "flat",
      tone: "",
    });
    expect(trendArrow(-10, "income", 10, true)).toEqual({
      direction: "flat",
      tone: "",
    });
    expect(trendArrow(0, "expense", 10, true)).toEqual({
      direction: "flat",
      tone: "",
    });
  });
  it("expense up over average is unfavorable (red)", () => {
    expect(trendArrow(50, "expense", 10, true)).toEqual({
      direction: "up",
      tone: "num-neg",
    });
  });
  it("expense down under average is favorable (green)", () => {
    expect(trendArrow(-50, "expense", 10, true)).toEqual({
      direction: "down",
      tone: "num-pos",
    });
  });
  it("income up over average is favorable (green)", () => {
    expect(trendArrow(50, "income", 10, true)).toEqual({
      direction: "up",
      tone: "num-pos",
    });
  });
  it("income down under average is unfavorable (red)", () => {
    expect(trendArrow(-50, "income", 10, true)).toEqual({
      direction: "down",
      tone: "num-neg",
    });
  });
});

describe("significantDeltaTone", () => {
  it("is neutral below or at the threshold", () => {
    expect(significantDeltaTone(10, "expense", 10)).toBe("");
    expect(significantDeltaTone(-10, "income", 10)).toBe("");
  });
  it("is the delta tone above the threshold", () => {
    expect(significantDeltaTone(50, "expense", 10)).toBe("num-neg");
    expect(significantDeltaTone(-50, "expense", 10)).toBe("num-pos");
    expect(significantDeltaTone(50, "income", 10)).toBe("num-pos");
  });
});

describe("kpiBudgetTrend", () => {
  it("returns null when there is no average (no data ≠ no change)", () => {
    expect(kpiBudgetTrend(800, 1000, "expense", 10, false)).toBeNull();
  });
  it("returns null when there is no budget to compare against (budget <= 0)", () => {
    expect(kpiBudgetTrend(800, 0, "expense", 10, true)).toBeNull();
    expect(kpiBudgetTrend(800, -50, "expense", 10, true)).toBeNull();
  });
  it("expense averaging below budget is favorable (down, green)", () => {
    expect(kpiBudgetTrend(800, 1000, "expense", 10, true)).toEqual({
      delta: -200,
      direction: "down",
      tone: "num-pos",
    });
  });
  it("expense averaging above budget is unfavorable (up, red)", () => {
    expect(kpiBudgetTrend(1200, 1000, "expense", 10, true)).toEqual({
      delta: 200,
      direction: "up",
      tone: "num-neg",
    });
  });
  it("income averaging above budget is favorable (up, green)", () => {
    expect(kpiBudgetTrend(2200, 2000, "income", 10, true)).toEqual({
      delta: 200,
      direction: "up",
      tone: "num-pos",
    });
  });
  it("income averaging below budget is unfavorable (down, red)", () => {
    expect(kpiBudgetTrend(1800, 2000, "income", 10, true)).toEqual({
      delta: -200,
      direction: "down",
      tone: "num-neg",
    });
  });
  it("within the threshold is flat with a neutral tone", () => {
    expect(kpiBudgetTrend(1005, 1000, "expense", 10, true)).toEqual({
      delta: 5,
      direction: "flat",
      tone: "",
    });
  });
});

describe("avgWindowLabel", () => {
  it("maps known windows", () => {
    expect(avgWindowLabel("3")).toBe("3m");
    expect(avgWindowLabel("ytd")).toBe("YTD");
    expect(avgWindowLabel("all")).toBe("total");
  });
  it("falls back to the id for unknown windows", () => {
    expect(avgWindowLabel("weird")).toBe("weird");
  });
});

describe("capitalizeSource", () => {
  it("maps the known bank presets", () => {
    expect(capitalizeSource("myinvestor")).toBe("MyInvestor");
    expect(capitalizeSource("N26")).toBe("N26");
  });
  it("uppercases the first letter of a generic source", () => {
    expect(capitalizeSource("manual")).toBe("Manual");
  });
  it("empty stays empty", () => {
    expect(capitalizeSource("")).toBe("");
    expect(capitalizeSource("   ")).toBe("");
  });
});

describe("normalizeSearchText", () => {
  it("lowercases and strips diacritics", () => {
    expect(normalizeSearchText("Café CON Leche")).toBe("cafe con leche");
    expect(normalizeSearchText("Nómina")).toBe("nomina");
    expect(normalizeSearchText("ÑOÑO")).toBe("nono");
  });
  it("is idempotent on already-plain text", () => {
    expect(normalizeSearchText("mercadona 42")).toBe("mercadona 42");
  });
});

describe("transactionMatchesQuery", () => {
  it("empty (or blank) query matches everything", () => {
    expect(transactionMatchesQuery("Alquiler", "Vivienda", "")).toBe(true);
    expect(transactionMatchesQuery("Alquiler", null, "   ")).toBe(true);
  });
  it("matches on concept, case- and accent-insensitive", () => {
    expect(transactionMatchesQuery("Café Central", null, "cafe")).toBe(true);
    expect(transactionMatchesQuery("Café Central", null, "CAFÉ")).toBe(true);
  });
  it("matches on category name too", () => {
    expect(transactionMatchesQuery("MERCADONA", "Alimentación", "aliment")).toBe(true);
  });
  it("returns false when neither concept nor category match", () => {
    expect(transactionMatchesQuery("Mercadona", "Alimentación", "gasolina")).toBe(false);
  });
  it("null category is fine", () => {
    expect(transactionMatchesQuery("Retirada cajero", null, "cajero")).toBe(true);
  });
});

describe("naturalSortDir", () => {
  it("date and amount start descending, concept ascending", () => {
    expect(naturalSortDir("date")).toBe("desc");
    expect(naturalSortDir("amount")).toBe("desc");
    expect(naturalSortDir("concept")).toBe("asc");
  });
});

describe("compareTransactions / sortTransactions", () => {
  const rows: TransactionApi[] = [
    txn({ id: "a", op_date: "2026-06-01", concept: "Zeta", amount: "-10.0000" }),
    txn({ id: "b", op_date: "2026-06-10", concept: "alfa", amount: "200.0000" }),
    txn({ id: "c", op_date: "2026-06-05", concept: "Mike", amount: "-500.0000" }),
  ];
  it("date desc is the natural order (most recent first)", () => {
    expect(sortTransactions(rows, "date", "desc").map((r) => r.id)).toEqual([
      "b",
      "c",
      "a",
    ]);
  });
  it("date asc reverses it", () => {
    expect(sortTransactions(rows, "date", "asc").map((r) => r.id)).toEqual([
      "a",
      "c",
      "b",
    ]);
  });
  it("concept asc is alphabetical, accent/case-insensitive", () => {
    expect(sortTransactions(rows, "concept", "asc").map((r) => r.id)).toEqual([
      "b", // alfa
      "c", // Mike
      "a", // Zeta
    ]);
  });
  it("amount sorts by MAGNITUDE, not signed value", () => {
    // |200| < |−500|, and |−10| is the smallest.
    expect(sortTransactions(rows, "amount", "desc").map((r) => r.id)).toEqual([
      "c", // 500
      "b", // 200
      "a", // 10
    ]);
    expect(sortTransactions(rows, "amount", "asc").map((r) => r.id)).toEqual([
      "a",
      "b",
      "c",
    ]);
  });
  it("stable tiebreak: op_date desc then id, independent of dir", () => {
    const tie: TransactionApi[] = [
      txn({ id: "y", op_date: "2026-06-02", concept: "x", amount: "100" }),
      txn({ id: "x", op_date: "2026-06-02", concept: "x", amount: "100" }),
      txn({ id: "z", op_date: "2026-06-09", concept: "x", amount: "100" }),
    ];
    // Same concept + same magnitude → tiebreak by op_date desc (z first), then id asc.
    expect(sortTransactions(tie, "amount", "desc").map((r) => r.id)).toEqual([
      "z",
      "x",
      "y",
    ]);
    expect(sortTransactions(tie, "amount", "asc").map((r) => r.id)).toEqual([
      "z",
      "x",
      "y",
    ]);
  });
  it("does not mutate the input array", () => {
    const input = [...rows];
    sortTransactions(input, "amount", "asc");
    expect(input.map((r) => r.id)).toEqual(["a", "b", "c"]);
  });
  it("compareTransactions is the primitive behind the sort", () => {
    expect(compareTransactions(rows[1], rows[2], "amount", "desc")).toBeGreaterThan(0);
  });
});

describe("isReconciled", () => {
  it("la contrapartida presente es la fuente de verdad", () => {
    expect(
      isReconciled(txn({ id: "1", transfer_counterpart_id: "abc" })),
    ).toBe(true);
  });
  it("sin contrapartida no está conciliada", () => {
    expect(isReconciled(txn({ id: "1" }))).toBe(false);
  });
});

describe("groupTransactionsByCategory", () => {
  const names = new Map<string, string>([
    ["c1", "Alimentación"],
    ["c2", "Ocio"],
  ]);
  it("groups by category, savings and uncategorized into their own buckets", () => {
    const rows: TransactionApi[] = [
      txn({ id: "1", category_id: "c1", amount: "-40.0000" }),
      txn({ id: "2", category_id: "c1", amount: "-10.0000" }),
      txn({ id: "3", kind: "savings", amount: "300.0000" }),
      txn({ id: "4", amount: "-25.0000" }), // expense, no category
      txn({ id: "5", category_id: "c2", amount: "-15.0000" }),
    ];
    const groups = groupTransactionsByCategory(rows, names);
    const byKey = Object.fromEntries(groups.map((g) => [g.key, g]));
    expect(byKey["c1"].label).toBe("Alimentación");
    expect(byKey["c1"].rows).toHaveLength(2);
    expect(byKey["c1"].subtotal).toBeCloseTo(-50);
    expect(byKey["savings"].label).toBe("Ahorro / Inversión");
    expect(byKey["savings"].subtotal).toBeCloseTo(300);
    expect(byKey["uncategorized-expense"].label).toBe("Sin categoría");
    expect(byKey["uncategorized-expense"].subtotal).toBeCloseTo(-25);
  });
  it("groups carry their kind (inherited from their rows)", () => {
    const rows: TransactionApi[] = [
      txn({ id: "1", kind: "income", category_id: "c1" }),
      txn({ id: "2", kind: "savings" }),
      txn({ id: "3", kind: "expense", category_id: "c2" }),
    ];
    const byKey = Object.fromEntries(
      groupTransactionsByCategory(rows, names).map((g) => [g.key, g]),
    );
    expect(byKey["c1"].kind).toBe("income");
    expect(byKey["savings"].kind).toBe("savings");
    expect(byKey["c2"].kind).toBe("expense");
  });
  it("uncategorized splits BY KIND: income without category ≠ expense without category", () => {
    const rows: TransactionApi[] = [
      txn({ id: "1", kind: "income", amount: "1000.0000" }),
      txn({ id: "2", kind: "expense", amount: "-25.0000" }),
    ];
    const groups = groupTransactionsByCategory(rows, names);
    expect(groups).toHaveLength(2);
    const byKey = Object.fromEntries(groups.map((g) => [g.key, g]));
    expect(byKey["uncategorized-income"].kind).toBe("income");
    expect(byKey["uncategorized-income"].label).toBe("Sin categoría");
    expect(byKey["uncategorized-expense"].kind).toBe("expense");
    expect(byKey["uncategorized-expense"].label).toBe("Sin categoría");
  });
  it("subtotal is SIGNED (positives and negatives net out)", () => {
    const rows: TransactionApi[] = [
      txn({ id: "1", category_id: "c1", amount: "100.0000" }),
      txn({ id: "2", category_id: "c1", amount: "-30.0000" }),
    ];
    expect(groupTransactionsByCategory(rows, names)[0].subtotal).toBeCloseTo(70);
  });
  it("las conciliadas siguen en rows pero NO suman al subtotal", () => {
    const rows: TransactionApi[] = [
      txn({ id: "1", category_id: "c1", amount: "-40.0000" }),
      txn({
        id: "2",
        category_id: "c1",
        amount: "-1000.0000",
        transfer_counterpart_id: "cp",
        transfer_counterpart_concept: "TRASPASO",
        transfer_counterpart_op_date: "2026-06-16",
      }),
    ];
    const g = groupTransactionsByCategory(rows, names)[0];
    expect(g.rows.map((r) => r.id)).toEqual(["1", "2"]);
    expect(g.subtotal).toBeCloseTo(-40);
  });
  it("un grupo entero de conciliadas queda con subtotal 0 (y sus filas visibles)", () => {
    const rows: TransactionApi[] = [
      txn({ id: "1", category_id: "c2", amount: "-500.0000", transfer_counterpart_id: "x" }),
      txn({ id: "2", category_id: "c2", amount: "500.0000", transfer_counterpart_id: "y" }),
    ];
    const g = groupTransactionsByCategory(rows, names)[0];
    expect(g.rows).toHaveLength(2);
    expect(g.subtotal).toBe(0);
  });
  it("falls back to the row's cached category_name when the id is unknown", () => {
    const rows: TransactionApi[] = [
      txn({ id: "1", category_id: "gone", category_name: "Vieja" }),
    ];
    expect(groupTransactionsByCategory(rows, names)[0].label).toBe("Vieja");
  });
  it("savings with a stray category_id still lands in the savings bucket", () => {
    const rows: TransactionApi[] = [
      txn({ id: "1", kind: "savings", category_id: "c1", amount: "50" }),
    ];
    const groups = groupTransactionsByCategory(rows, names);
    expect(groups).toHaveLength(1);
    expect(groups[0].key).toBe("savings");
  });
});

describe("sortTransactionGroups", () => {
  function grp(
    over: Partial<ReturnType<typeof groupTransactionsByCategory>[number]>,
  ) {
    return {
      key: "k",
      label: "L",
      kind: "expense" as TransactionKindApi,
      rows: [],
      subtotal: 0,
      ...over,
    };
  }
  it("kind sections come first: income → savings → expense", () => {
    const groups = [
      grp({ key: "e", kind: "expense", subtotal: -9000 }),
      grp({ key: "s", kind: "savings", subtotal: 10 }),
      grp({ key: "i", kind: "income", subtotal: 100 }),
    ];
    // The expense group is by far the largest, but income still leads.
    expect(sortTransactionGroups(groups).map((g) => g.key)).toEqual(["i", "s", "e"]);
  });
  it("within a section, groups always order by |subtotal| descending", () => {
    const groups = [
      grp({ key: "a", subtotal: 100 }),
      grp({ key: "b", subtotal: -500 }),
      grp({ key: "c", subtotal: -30 }),
    ];
    expect(sortTransactionGroups(groups).map((g) => g.key)).toEqual(["b", "a", "c"]);
  });
  it("kind priority + |subtotal| desc combined", () => {
    const groups = [
      grp({ key: "e1", kind: "expense", subtotal: -30 }),
      grp({ key: "i1", kind: "income", subtotal: 100 }),
      grp({ key: "e2", kind: "expense", subtotal: -500 }),
      grp({ key: "i2", kind: "income", subtotal: 2000 }),
    ];
    expect(sortTransactionGroups(groups).map((g) => g.key)).toEqual([
      "i2",
      "i1",
      "e2",
      "e1",
    ]);
  });
  it("ties break alphabetically by label (accent/case-insensitive)", () => {
    const groups = [
      grp({ key: "a", label: "Ómnibus", subtotal: -50 }),
      grp({ key: "b", label: "alquiler", subtotal: 50 }),
      grp({ key: "c", label: "Zapatos", subtotal: -50 }),
    ];
    expect(sortTransactionGroups(groups).map((g) => g.key)).toEqual(["b", "a", "c"]);
  });
  it("changing sortKey/sortDir does NOT reorder groups but DOES reorder their rows", () => {
    const rows: TransactionApi[] = [
      txn({ id: "1", category_id: "c1", concept: "Beta", op_date: "2026-06-01", amount: "-40.0000" }),
      txn({ id: "2", category_id: "c1", concept: "Alfa", op_date: "2026-06-20", amount: "-10.0000" }),
      txn({ id: "3", kind: "income", concept: "Nómina", op_date: "2026-06-25", amount: "2000.0000" }),
      txn({ id: "4", category_id: "c2", concept: "Cine", op_date: "2026-06-10", amount: "-300.0000" }),
    ];
    const names = new Map([
      ["c1", "Alimentación"],
      ["c2", "Ocio"],
    ]);
    const groups = sortTransactionGroups(groupTransactionsByCategory(rows, names));
    // Fixed group order: income section first, then expense groups by |subtotal| desc.
    const expectedGroupKeys = ["uncategorized-income", "c2", "c1"];
    expect(groups.map((g) => g.key)).toEqual(expectedGroupKeys);
    for (const [key, dir] of [
      ["date", "asc"],
      ["date", "desc"],
      ["concept", "asc"],
      ["amount", "desc"],
    ] as const) {
      const resorted = sortTransactionGroups(
        groupTransactionsByCategory(rows, names),
      ).map((g) => ({ ...g, rows: sortTransactions(g.rows, key, dir) }));
      expect(resorted.map((g) => g.key)).toEqual(expectedGroupKeys);
    }
    // …but the rows inside c1 do follow the active key.
    const c1 = (key: "date" | "concept" | "amount", dir: "asc" | "desc") =>
      sortTransactions(groups.find((g) => g.key === "c1")!.rows, key, dir).map(
        (r) => r.id,
      );
    expect(c1("date", "desc")).toEqual(["2", "1"]);
    expect(c1("date", "asc")).toEqual(["1", "2"]);
    expect(c1("concept", "asc")).toEqual(["2", "1"]); // Alfa, Beta
    expect(c1("amount", "desc")).toEqual(["1", "2"]); // |−40| > |−10|
  });
});

function totals(overrides: Partial<SummaryTotalsApi>): SummaryTotalsApi {
  return {
    expense_actual: "0",
    expense_budget: "0",
    expense_avg: "0",
    income_actual: "0",
    income_budget: "0",
    income_avg: "0",
    savings_actual: "0",
    savings_avg: "0",
    net_actual: "0",
    ...overrides,
  };
}

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

function txn(
  overrides: Partial<TransactionApi> & { id: string },
): TransactionApi {
  return {
    source: "manual",
    op_date: "2026-06-15",
    concept: "TEST",
    amount: "-10.0000",
    currency: "EUR",
    kind: "expense" as TransactionKindApi,
    created_at: "2026-06-15T00:00:00Z",
    updated_at: "2026-06-15T00:00:00Z",
    ...overrides,
  };
}
