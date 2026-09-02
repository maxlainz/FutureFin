import { describe, expect, it } from "vitest";
import type {
  CategoryRow,
  ImportConfirmResponseApi,
  ImportPreviewRowApi,
  SummaryTotalsApi,
  TransactionApi,
  TransactionKindApi,
  TransactionMonthApi,
} from "../api/types";
import {
  KIND_LABEL_ES,
  PENDING_ASSIGNMENTS_CAP,
  SAVINGS_GROUP_LABEL,
  adjacentMonthInList,
  adoptSuggestion,
  avgBasisDetail,
  avgUnavailableDetail,
  avgWindowLabel,
  buildConfirmDecisions,
  buildPendingAssignments,
  capitalizeSource,
  categoriesForKind,
  compareTransactions,
  defaultSelectedMonth,
  deltaToneClass,
  draftsEqual,
  groupTransactionsByCategory,
  initialDraftForRow,
  isPositiveAmountString,
  isReconciled,
  isRefundRow,
  kpiBudgetTrend,
  mergePendingAssignments,
  mergeRepreview,
  monthLabelEs,
  monthShortLabelEs,
  naturalSortDir,
  normalizeSearchText,
  parseMonth,
  pendingAssignmentForDraft,
  rowMatchesFilter,
  savingsBreakdown,
  savingsRateFromTotals,
  signOf,
  significanceThreshold,
  significantDeltaTone,
  sortTransactionGroups,
  sortTransactions,
  summarizeDecisions,
  summarizeImportBatch,
  transactionMatchesQuery,
  trendArrow,
  type ImportRowDraft,
} from "./expenses";

function cat(
  id: string,
  scope: CategoryRow["scope"],
  name: string,
  isFallback = false,
): CategoryRow {
  return { id, scope, name, sort_index: 0, is_fallback: isFallback };
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
  it("la procedencia de la categoría viaja desde el preview (4.15.0)", () => {
    expect(
      initialDraftForRow(
        previewRow({
          index: 0,
          suggested_category_id: "c1",
          suggested_category_source: "rule",
        }),
      ).categorySource,
    ).toBe("rule");
    expect(
      initialDraftForRow(
        previewRow({
          index: 1,
          suggested_category_id: "otros-gastos",
          suggested_category_source: "fallback",
        }),
      ).categorySource,
    ).toBe("fallback");
  });
  it("sin categoría sugerida no hay procedencia que declarar", () => {
    expect(initialDraftForRow(previewRow({ index: 2 })).categorySource).toBeNull();
    expect(
      initialDraftForRow(
        previewRow({
          index: 3,
          suggested_kind: "savings",
          suggested_category_id: "c9",
          suggested_category_source: "rule",
        }),
      ).categorySource,
    ).toBeNull();
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
        categorySource: "user",
        linkedAssetId: "",
        linkedLiabilityId: "l1",
        force: false,
      },
      {
        include: true,
        kind: "expense",
        categoryId: "",
        categorySource: null,
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
        categorySource: "user",
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
        categorySource: null,
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
      // Solo las DOS que se van a crear: el duplicado omitido y la excluida no llegan a existir,
      // así que el servidor tampoco las valida.
      missingCategory: 2,
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
      // Las tres filas del helper llegan sin categoría sugerida; solo las dos INCLUIDAS cuentan.
      missingCategory: 2,
    });
  });

  it("missingCategory usa el MISMO predicado que el servidor: solo lo que se va a crear", () => {
    const rows = [
      previewRow({ index: 0 }),
      previewRow({ index: 1 }),
      previewRow({ index: 2 }),
      previewRow({ index: 3 }),
      previewRow({ index: 4, status: "already_imported" }),
    ];
    const drafts: ImportRowDraft[] = [
      draft({ include: true, kind: "expense", categoryId: "c1" }), // clasificada
      draft({ include: true, kind: "income", categoryId: "" }), // ← cuenta
      draft({ include: false, kind: "expense", categoryId: "" }), // excluida: no crea nada
      draft({ include: true, kind: "savings", categoryId: "" }), // savings no la admite
      draft({ include: true, kind: "expense", categoryId: "" }), // duplicado omitido: tampoco
    ];
    expect(summarizeDecisions(rows, drafts).missingCategory).toBe(1);
  });

  it("con el preview pre-rellenando la categoría por defecto, missingCategory es 0", () => {
    const rows = [
      previewRow({
        index: 0,
        suggested_category_id: "otros-gastos",
        suggested_category_source: "fallback",
      }),
      previewRow({
        index: 1,
        suggested_kind: "income",
        suggested_category_id: "otros-ingresos",
        suggested_category_source: "fallback",
      }),
    ];
    expect(
      summarizeDecisions(rows, rows.map(initialDraftForRow)).missingCategory,
    ).toBe(0);
  });
});

describe("summarizeImportBatch", () => {
  const confirmRes = (
    over: Partial<ImportConfirmResponseApi>,
  ): ImportConfirmResponseApi => ({
    import_id: "11111111-1111-1111-1111-111111111111",
    imported: 0,
    skipped_already_imported: 0,
    discarded: 0,
    rules_learned: 0,
    reconciled_pairs: 0,
    ...over,
  });

  it("suma campo a campo las respuestas de todos los confirms de la tanda", () => {
    const batch = summarizeImportBatch([
      confirmRes({ imported: 12, skipped_already_imported: 2, reconciled_pairs: 1 }),
      confirmRes({ imported: 30, discarded: 4, rules_learned: 3 }),
      confirmRes({ imported: 0, import_id: undefined, skipped_already_imported: 7 }),
    ]);
    expect(batch).toEqual({
      files: 3,
      imported: 42,
      skipped_already_imported: 9,
      discarded: 4,
      rules_learned: 3,
      reconciled_pairs: 1,
    });
  });

  it("tanda vacía (cancelar sin confirmar nada) → todo a cero", () => {
    expect(summarizeImportBatch([])).toEqual({
      files: 0,
      imported: 0,
      skipped_already_imported: 0,
      discarded: 0,
      rules_learned: 0,
      reconciled_pairs: 0,
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
    expect(avgWindowLabel("ytd")).toBe("año en curso");
    expect(avgWindowLabel("all")).toBe("total");
  });
  it("falls back to the id for unknown windows", () => {
    expect(avgWindowLabel("weird")).toBe("weird");
  });
});

describe("avgBasisDetail", () => {
  it("names the contiguous range the average came from", () => {
    expect(
      avgBasisDetail({
        months: 3,
        first_month: "2026-04",
        last_month: "2026-06",
        has_gaps: false,
      }),
    ).toBe("3 meses · abr–jun 2026");
  });

  it("keeps both years when the range crosses one", () => {
    expect(
      avgBasisDetail({
        months: 3,
        first_month: "2025-11",
        last_month: "2026-01",
        has_gaps: false,
      }),
    ).toBe("3 meses · nov 2025–ene 2026");
  });

  it("collapses a single month instead of repeating it", () => {
    expect(
      avgBasisDetail({
        months: 1,
        first_month: "2026-04",
        last_month: "2026-04",
        has_gaps: false,
      }),
    ).toBe("1 mes · abr 2026");
  });

  it("refuses to fake a contiguous range when there are gaps", () => {
    // abr y jun (may fuera) NO son «abr–jun»: se dice cuántos meses y hasta cuándo.
    expect(
      avgBasisDetail({
        months: 2,
        first_month: "2026-04",
        last_month: "2026-06",
        has_gaps: true,
      }),
    ).toBe("2 meses con datos · hasta jun 2026");
  });

  it("has nothing to say without a basis", () => {
    expect(avgBasisDetail(undefined)).toBeUndefined();
  });
});

describe("avgUnavailableDetail", () => {
  it("distinguishes no data from only-recurring data", () => {
    expect(avgUnavailableDetail("empty_window")).toBe("sin datos en la ventana");
    expect(avgUnavailableDetail("only_recurring_months")).toBe(
      "sin meses con movimientos reales",
    );
  });
  it("says nothing when there IS an average", () => {
    expect(avgUnavailableDetail(undefined)).toBeUndefined();
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
    expect(byKey["savings"].label).toBe(SAVINGS_GROUP_LABEL);
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
    net_avg: "0",
    refunds_actual: "0",
    refunds_avg: "0",
    ...overrides,
  };
}

function draft(overrides: Partial<ImportRowDraft>): ImportRowDraft {
  return {
    include: true,
    kind: "expense",
    categoryId: "",
    categorySource: null,
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

// ---------------------------------------------------------------------------
// Devoluciones (gasto con importe positivo)
// ---------------------------------------------------------------------------

describe("isPositiveAmountString", () => {
  it("reconoce positivos tal como los sirve la API", () => {
    expect(isPositiveAmountString("12.5000")).toBe(true);
    expect(isPositiveAmountString("0.0100")).toBe(true);
    expect(isPositiveAmountString("+7")).toBe(true);
    expect(isPositiveAmountString(" 3 ")).toBe(true);
  });
  it("acepta la coma decimal (lo que se teclea en el modal de edición)", () => {
    expect(isPositiveAmountString("12,5")).toBe(true);
    expect(isPositiveAmountString("-12,5")).toBe(false);
  });
  it("el cero NO es positivo, con los decimales que traiga", () => {
    expect(isPositiveAmountString("0")).toBe(false);
    expect(isPositiveAmountString("0.0000")).toBe(false);
    expect(isPositiveAmountString("-0.0000")).toBe(false);
  });
  it("negativos: menos ASCII y menos tipográfico", () => {
    expect(isPositiveAmountString("-1")).toBe(false);
    expect(isPositiveAmountString("−1")).toBe(false);
  });
  it("ante lo que no entiende, calla (no afirma que sea devolución)", () => {
    expect(isPositiveAmountString("")).toBe(false);
    expect(isPositiveAmountString(null)).toBe(false);
    expect(isPositiveAmountString(undefined)).toBe(false);
    expect(isPositiveAmountString("1e3")).toBe(false);
    expect(isPositiveAmountString("abc")).toBe(false);
  });
});

describe("isRefundRow", () => {
  it("solo el gasto positivo es devolución", () => {
    expect(isRefundRow("expense", "12.0000")).toBe(true);
    expect(isRefundRow("expense", "-12.0000")).toBe(false);
    expect(isRefundRow("income", "12.0000")).toBe(false);
    expect(isRefundRow("savings", "12.0000")).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// Automatch en vivo del preview
// ---------------------------------------------------------------------------

describe("draftsEqual", () => {
  it("compara los seis campos editables", () => {
    expect(draftsEqual(draft({}), draft({}))).toBe(true);
    expect(draftsEqual(draft({}), draft({ include: false }))).toBe(false);
    expect(draftsEqual(draft({}), draft({ categoryId: "c1" }))).toBe(false);
    expect(draftsEqual(draft({}), draft({ kind: "income" }))).toBe(false);
    expect(draftsEqual(draft({}), draft({ force: true }))).toBe(false);
    expect(draftsEqual(draft({}), draft({ linkedAssetId: "a1" }))).toBe(false);
    expect(draftsEqual(draft({}), draft({ linkedLiabilityId: "l1" }))).toBe(false);
  });
});

describe("pendingAssignmentForDraft (gate)", () => {
  it("con categoría, sí", () => {
    expect(pendingAssignmentForDraft("CAFE 365", draft({ categoryId: "c1" }))).toEqual({
      concept: "CAFE 365",
      kind: "expense",
      category_id: "c1",
    });
  });
  it("savings sí, aunque no lleve categoría (no puede llevarla)", () => {
    expect(pendingAssignmentForDraft("TRASPASO", draft({ kind: "savings" }))).toEqual({
      concept: "TRASPASO",
      kind: "savings",
      category_id: null,
    });
  });
  it("expense/income sin categoría NO enseñan nada al servidor", () => {
    expect(pendingAssignmentForDraft("X", draft({ categoryId: "" }))).toBeNull();
    expect(
      pendingAssignmentForDraft("X", draft({ kind: "income", categoryId: "" })),
    ).toBeNull();
  });
  it("la categoría POR DEFECTO tampoco: nadie la decidió, no debe propagarse (4.15.0)", () => {
    expect(
      pendingAssignmentForDraft(
        "COMPRA RARA",
        draft({ categoryId: "otros-gastos", categorySource: "fallback" }),
      ),
    ).toBeNull();
  });
  it("la MISMA categoría elegida a mano sí se propaga: elegirla es una decisión", () => {
    expect(
      pendingAssignmentForDraft(
        "COMPRA RARA",
        draft({ categoryId: "otros-gastos", categorySource: "user" }),
      ),
    ).toEqual({
      concept: "COMPRA RARA",
      kind: "expense",
      category_id: "otros-gastos",
    });
  });
  it("una categoría que puso una REGLA sigue propagándose", () => {
    expect(
      pendingAssignmentForDraft("CAFE 365", draft({ categoryId: "c1", categorySource: "rule" })),
    ).not.toBeNull();
  });
});

describe("draftsEqual (procedencia fuera de la comparación)", () => {
  it("dos drafts que solo difieren en categorySource son el MISMO draft", () => {
    expect(
      draftsEqual(
        draft({ categoryId: "c1", categorySource: "fallback" }),
        draft({ categoryId: "c1", categorySource: "user" }),
      ),
    ).toBe(true);
  });
  it("pero un cambio real de categoría sí se ve", () => {
    expect(
      draftsEqual(draft({ categoryId: "c1" }), draft({ categoryId: "c2" })),
    ).toBe(false);
  });
});

describe("mergePendingAssignments", () => {
  it("acumula solo lo que pasa el gate", () => {
    const out = mergePendingAssignments(new Map(), [
      { concept: "CAFE 365", draft: draft({ categoryId: "c1" }) },
      { concept: "SIN CAT", draft: draft({}) },
    ]);
    expect([...out.keys()]).toEqual(["CAFE 365"]);
  });

  it("devuelve el MISMO Map si nada pasó el gate (identidad = no relanzar el preview)", () => {
    const prev = new Map();
    expect(mergePendingAssignments(prev, [{ concept: "X", draft: draft({}) }])).toBe(
      prev,
    );
  });

  it("no muta el Map anterior", () => {
    const prev = new Map();
    mergePendingAssignments(prev, [
      { concept: "CAFE 365", draft: draft({ categoryId: "c1" }) },
    ]);
    expect(prev.size).toBe(0);
  });

  it("última escritura gana Y pasa al final (recencia)", () => {
    let m = mergePendingAssignments(new Map(), [
      { concept: "A", draft: draft({ categoryId: "c1" }) },
      { concept: "B", draft: draft({ categoryId: "c2" }) },
    ]);
    m = mergePendingAssignments(m, [
      { concept: "A", draft: draft({ categoryId: "c9" }) },
    ]);
    expect([...m.keys()]).toEqual(["B", "A"]);
    expect(m.get("A")?.category_id).toBe("c9");
  });

  it("ignora conceptos vacíos (fila fuera de rango)", () => {
    const prev = new Map();
    expect(
      mergePendingAssignments(prev, [
        { concept: "", draft: draft({ categoryId: "c1" }) },
      ]),
    ).toBe(prev);
  });
});

describe("buildPendingAssignments", () => {
  it("cap: se queda con las MÁS RECIENTES (la cola del orden de inserción)", () => {
    let m = new Map();
    for (let i = 0; i < 5; i += 1) {
      m = mergePendingAssignments(m, [
        { concept: `C${i}`, draft: draft({ categoryId: "c1" }) },
      ]);
    }
    expect(buildPendingAssignments(m, 2).map((a) => a.concept)).toEqual(["C3", "C4"]);
  });
  it("por debajo del cap devuelve todo, ya deduplicado por concepto", () => {
    let m = mergePendingAssignments(new Map(), [
      { concept: "A", draft: draft({ categoryId: "c1" }) },
    ]);
    m = mergePendingAssignments(m, [
      { concept: "A", draft: draft({ categoryId: "c2" }) },
    ]);
    expect(buildPendingAssignments(m)).toEqual([
      { concept: "A", kind: "expense", category_id: "c2" },
    ]);
  });
  it("el cap por defecto es el que acepta el backend", () => {
    expect(PENDING_ASSIGNMENTS_CAP).toBe(200);
  });
});

describe("adoptSuggestion", () => {
  it("adopta kind y categoría recalculados", () => {
    const d = draft({});
    const out = adoptSuggestion(
      d,
      previewRow({ index: 0, suggested_kind: "expense", suggested_category_id: "c1" }),
    );
    expect(out.categoryId).toBe("c1");
    expect(out.include).toBe(d.include);
  });
  it("savings no arrastra categoría", () => {
    const out = adoptSuggestion(
      draft({ categoryId: "c1" }),
      previewRow({ index: 0, suggested_kind: "savings", suggested_category_id: "c1" }),
    );
    expect(out.kind).toBe("savings");
    expect(out.categoryId).toBe("");
  });
  it("misma referencia cuando no cambia nada", () => {
    const d = draft({ categoryId: "c1" });
    expect(
      adoptSuggestion(
        d,
        previewRow({ index: 0, suggested_kind: "expense", suggested_category_id: "c1" }),
      ),
    ).toBe(d);
  });
  it("adopta también la procedencia (4.15.0)", () => {
    const out = adoptSuggestion(
      draft({}),
      previewRow({
        index: 0,
        suggested_category_id: "otros-gastos",
        suggested_category_source: "fallback",
      }),
    );
    expect(out.categoryId).toBe("otros-gastos");
    expect(out.categorySource).toBe("fallback");
  });
  it("misma categoría con procedencia nueva: se refresca sin tocar la asignación", () => {
    const d = draft({ categoryId: "c1", categorySource: "fallback" });
    const out = adoptSuggestion(
      d,
      previewRow({
        index: 0,
        suggested_category_id: "c1",
        suggested_category_source: "rule",
      }),
    );
    expect(out).not.toBe(d);
    expect(out.categoryId).toBe("c1");
    expect(out.categorySource).toBe("rule");
  });
});

describe("mergeRepreview", () => {
  const rows = [
    previewRow({ index: 0, suggested_kind: "expense", suggested_category_id: "c9" }),
    previewRow({ index: 1, suggested_kind: "expense", suggested_category_id: "c9" }),
    previewRow({ index: 2, suggested_kind: "expense", suggested_category_id: "c9" }),
  ];
  const previous = [
    previewRow({ index: 0 }),
    previewRow({ index: 1 }),
    previewRow({ index: 2 }),
  ];

  it("las filas TOCADAS no se pisan nunca; el resto adopta", () => {
    const drafts = [
      draft({ categoryId: "mia" }),
      draft({ categoryId: "" }),
      draft({ categoryId: "" }),
    ];
    const out = mergeRepreview(drafts, [true, false, false], rows, previous);
    expect(out).not.toBeNull();
    expect(out?.drafts[0].categoryId).toBe("mia");
    expect(out?.drafts[0]).toBe(drafts[0]);
    expect(out?.drafts[1].categoryId).toBe("c9");
    expect(out?.drafts[2].categoryId).toBe("c9");
    expect(out?.changed).toBe(2);
  });

  it("cuenta 0 y conserva el array cuando no se mueve ninguna fila", () => {
    const drafts = [
      draft({ categoryId: "c9" }),
      draft({ categoryId: "c9" }),
      draft({ categoryId: "c9" }),
    ];
    const out = mergeRepreview(drafts, [false, false, false], rows, previous);
    expect(out?.changed).toBe(0);
    expect(out?.drafts).toBe(drafts);
  });

  it("un refresco de PROCEDENCIA no cuenta como fila movida (el aviso no se infla)", () => {
    const soloProcedencia = [
      previewRow({
        index: 0,
        suggested_category_id: "c9",
        suggested_category_source: "rule",
      }),
      previewRow({
        index: 1,
        suggested_category_id: "c9",
        suggested_category_source: "rule",
      }),
      previewRow({
        index: 2,
        suggested_category_id: "c9",
        suggested_category_source: "rule",
      }),
    ];
    const drafts = [
      draft({ categoryId: "c9", categorySource: "fallback" }),
      draft({ categoryId: "c9", categorySource: "fallback" }),
      draft({ categoryId: "c9", categorySource: "fallback" }),
    ];
    const out = mergeRepreview(drafts, [false, false, false], soloProcedencia, previous);
    expect(out?.changed).toBe(0);
    // …pero el array SÍ se reemplaza: la procedencia nueva tiene que llegar al draft.
    expect(out?.drafts).not.toBe(drafts);
    expect(out?.drafts.every((d) => d.categorySource === "rule")).toBe(true);
  });

  it("todas tocadas → no se mueve nada", () => {
    const drafts = [draft({}), draft({}), draft({})];
    const out = mergeRepreview(drafts, [true, true, true], rows, previous);
    expect(out?.changed).toBe(0);
    expect(out?.drafts).toBe(drafts);
  });

  it("null si el recuento de filas no cuadra", () => {
    expect(
      mergeRepreview([draft({})], [false], rows, previous),
    ).toBeNull();
  });

  it("null si los índices de las filas no cuadran (nunca adoptar descolocado)", () => {
    const shifted = [
      previewRow({ index: 0 }),
      previewRow({ index: 7 }),
      previewRow({ index: 2 }),
    ];
    expect(
      mergeRepreview(
        [draft({}), draft({}), draft({})],
        [false, false, false],
        shifted,
        previous,
      ),
    ).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// Ahorro de Movimientos (4.15.0): ingresos − gastos, y su desglose
// ---------------------------------------------------------------------------

describe("savingsBreakdown", () => {
  it("mes normal: lo que no se invirtió se quedó en cuenta", () => {
    expect(
      savingsBreakdown(totals({ net_avg: "1000", savings_avg: "800" })),
    ).toEqual({ ahorro: 1000, invertido: 800, enCuenta: 200 });
  });

  it("invertir más de lo ahorrado deja «en cuenta» NEGATIVO: salió de reservas", () => {
    const out = savingsBreakdown(totals({ net_avg: "200", savings_avg: "500" }));
    expect(out).toEqual({ ahorro: 200, invertido: 500, enCuenta: -300 });
  });

  it("ahorro negativo (se gastó más de lo que entró) es un valor, no un error", () => {
    expect(savingsBreakdown(totals({ net_avg: "-450", savings_avg: "0" }))).toEqual({
      ahorro: -450,
      invertido: 0,
      enCuenta: -450,
    });
  });

  it("sin promedio no hay desglose: null, nunca ceros de relleno", () => {
    expect(savingsBreakdown(totals({ net_avg: null, savings_avg: null }))).toBeNull();
    expect(savingsBreakdown(totals({ net_avg: "1000", savings_avg: null }))).toBeNull();
    expect(savingsBreakdown(totals({ net_avg: null, savings_avg: "800" }))).toBeNull();
    expect(savingsBreakdown(null)).toBeNull();
  });

  it("un cero de verdad SÍ se desglosa (cero ≠ ausencia)", () => {
    expect(savingsBreakdown(totals({ net_avg: "0", savings_avg: "0" }))).toEqual({
      ahorro: 0,
      invertido: 0,
      enCuenta: 0,
    });
  });
});

describe("savingsRateFromTotals", () => {
  it("es el ahorro sobre el ingreso, en porcentaje", () => {
    expect(
      savingsRateFromTotals(totals({ net_avg: "500", income_avg: "2000" })),
    ).toBeCloseTo(25);
  });

  it("puede ser negativa: gastar más de lo que entra no es «0 %»", () => {
    expect(
      savingsRateFromTotals(totals({ net_avg: "-200", income_avg: "1000" })),
    ).toBeCloseTo(-20);
  });

  it("ingreso 0 o negativo → null: no hay denominador que signifique nada", () => {
    expect(
      savingsRateFromTotals(totals({ net_avg: "500", income_avg: "0" })),
    ).toBeNull();
    expect(
      savingsRateFromTotals(totals({ net_avg: "500", income_avg: "-10" })),
    ).toBeNull();
  });

  it("sin promedio, null (y no un 0 % inventado)", () => {
    expect(
      savingsRateFromTotals(totals({ net_avg: null, income_avg: null })),
    ).toBeNull();
    expect(
      savingsRateFromTotals(totals({ net_avg: "500", income_avg: null })),
    ).toBeNull();
    expect(savingsRateFromTotals(undefined)).toBeNull();
  });
});

describe("rótulo de la clase savings (4.15.0)", () => {
  it("se llama «Inversión», no «Ahorro»: el ahorro es ingresos − gastos", () => {
    expect(KIND_LABEL_ES.savings).toBe("Inversión");
    expect(SAVINGS_GROUP_LABEL).toBe("Inversión");
  });
  it("la tabla agrupada y el selector de tipo dicen lo mismo", () => {
    expect(SAVINGS_GROUP_LABEL).toBe(KIND_LABEL_ES.savings);
  });
});
