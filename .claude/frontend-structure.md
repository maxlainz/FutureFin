# Frontend Structure (`apps/web/src/`)

Post-refactor (May 2026). Before: one `App.tsx` of 10.384 LOC owning everything. After: composition root + per-concern modules.

```
src/
├── App.tsx                       # composition root: auth gate + global state + route → view dispatch (~3.000 LOC)
├── App.css                       # global styles
├── main.tsx                      # ReactDOM.createRoot entry
│
├── api/
│   ├── client.ts                 # fetch wrappers: apiGet/Post/Patch/Delete + defaultFetchInit + errorMessageFromResponse
│   ├── client.test.ts            # mocks `globalThis.fetch`, asserts credentials/Content-Type/204
│   └── types.ts                  # all *Api / *Response / *Row types (mirror of Rust handler structs)
│
├── lib/                          # pure helpers, no React imports
│   ├── format.ts                 # money/percent/decimal formatting (es-ES locale), parseDisplayDecimal, METRIC_DASH
│   ├── format.test.ts            # 29 tests
│   ├── dates.ts                  # civil-calendar arithmetic (parallel to crates/engine), TZ-aware "today", interval counts
│   ├── dates.test.ts             # 26 tests
│   ├── ledger.ts                 # shared by views: ledgerViewQs, groupRowsByCategoryOrdered, asset/liability portfolio helpers,
│   │                             #   PAYMENT_FREQ_LABEL, formatProjectionMilestoneCompactLabel, budgetCategoryMap,
│   │                             #   sortBudgetEntriesMacStyle, formatAxisMoney, LedgerPersonScope, LiabilityPaymentFreq
│   ├── fire.ts                   # client-side FIRE math for the live form preview (mirror of handlers/projection.rs):
│   │                             #   defaultFireSettingsApi, normalizeInstallationFireSettings, taxOnGrossCapitalAnnual,
│   │                             #   grossUpNetAnnualFire, computeFireAnnualNeedNetEur, findFirstMonthNetWorthAtLeastInflated
│   ├── projection-chart.ts       # chart helpers: tick builders, SVG layout, niceYTicks, axis age/dates mode,
│   │                             #   PROJECTION_FOCUS_STORAGE_KEY, ASSET_LINE_COLORS, complementaryProjectionTickLabel,
│   │                             #   projectionHoverTitle, formatYearsEsFromMonths, formatProjectionChartHorizonLine
│   └── navigation.ts             # tab ↔ URL map: TABS, TAB_PATH, SETTINGS_SUBTAB_*, tabFromPathname, settingsSubTabPath
│
├── components/                   # generic UI primitives (no domain knowledge)
│   ├── Modal.tsx                 # Modal + ModalFormError + InlineHint
│   ├── MetricCard.tsx
│   ├── icons.tsx                 # PlusIcon, RowEditIcon, RowTrashIcon, GearIcon
│   └── charts/
│       ├── summary.tsx           # SummaryDonutChart + SummaryBreakdownBlock (palettes inline)
│       └── PlanningDirectionChart.tsx
│
├── views/                        # one file per tab — receives props from App.tsx, owns local UI state
│   ├── SummaryView.tsx           # 271 LOC
│   ├── AssetsView.tsx            # 552
│   ├── LiabilitiesView.tsx       # 512
│   ├── BudgetView.tsx            # 752  (exports BudgetScopeToggle type)
│   ├── UpcomingView.tsx          # 424  (Planning)
│   ├── RetirementView.tsx        # 559
│   ├── ProjectionView.tsx        # 246  (wraps ProjectionNetWorthChart)
│   ├── ProjectionNetWorthChart.tsx  # 976 — large SVG chart, drag/zoom/hover
│   ├── SettingsView.tsx          # 1.019 — sub-tabs (Acceso / Calendario / Proyección / Jubilación / Categorías / Datos)
│   └── AllocationRulesPanel.tsx  # 246 — used embedded inside BudgetView modal
│
└── auth/
    └── BootstrapInstallationPanel.tsx  # first-user setup form (currency + IANA tz)
```

## Import conventions

- **`api/`** depends only on `api/` and the DOM `fetch`. No React.
- **`lib/`** is pure: no React, no fetch. May import from other `lib/*` and from `api/types`.
- **`components/`** may import from `lib/` and `api/types`. They are dumb presentational widgets.
- **`views/`** may import from anything below (`lib/`, `api/`, `components/`, other views). They own form/UI state via `useState` and receive data + mutation callbacks from `App.tsx`.
- **`App.tsx`** owns the long-lived state (installation, user, ledgerPersonScope, lists, busy flags) and the API mutation handlers. Dispatch to a view is a `<XxxView {...props} />` call.

## Where to add new code

| New thing | Goes in |
|----|----|
| New API type returned by the backend | `api/types.ts` (export it) |
| New fetch endpoint wrapper | `api/client.ts` if reusable, otherwise inline in `App.tsx` next to existing handlers |
| New pure formatter / parser | `lib/format.ts` (with a Vitest in `lib/format.test.ts`) |
| New shared chart/SVG widget | `components/charts/` |
| New full tab/page | `views/NewView.tsx` + add to `TABS` / `TAB_PATH` in `lib/navigation.ts` + render branch in `App.tsx` |
| New Settings sub-tab | add to `SETTINGS_SUBTAB_SLUG`/`_LABEL` in `lib/navigation.ts` + render branch inside `SettingsView` |
| New auth/setup flow | `auth/` |

## Why this layout

- **`App.tsx` shrinks** to coordination only. Easy to reason about routing + global state.
- **Pure helpers in `lib/`** are testable in `node` (no DOM, no jsdom). Vitest runs them in ~30 ms.
- **Views are self-contained**: each one can be opened and understood without scrolling 10K lines.
- **Tests live next to code**: `format.test.ts` sits beside `format.ts`. The pattern scales — add helpers + tests together.
- **No circular deps**: `views/` import `lib/`, `lib/` doesn't import `views/`. Linter would catch it.

## What is NOT extracted (intentional)

- **API mutation handlers** (`submitAssetForm`, `deleteLiabilityRow`, etc.) stay in `App.tsx`. They close over `setAssets`, `setLiabilities`, etc. Moving them out requires a state library (Redux / Zustand / TanStack Query) — out of scope.
- **Auth gate flow** (login/register/pending screens) is inline in `App.tsx`. `BootstrapInstallationPanel` is extracted but the login/register form is small enough that splitting it adds ceremony.
- **FIRE client-side math** (`lib/fire.ts`) duplicates the Rust engine's tax/gross-up logic. Intentional: it powers the **live preview** of the FIRE settings form (user types `swr_pct`, sees the target update without a round-trip). If you change tax brackets server-side, mirror the change here.

## Frontend tests

See [`tests.md`](tests.md). Setup: Vitest + `node` environment (no jsdom needed for the current test set). All tests are in `*.test.ts` files colocated with the module they test.
