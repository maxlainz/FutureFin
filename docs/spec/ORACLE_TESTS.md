# Oráculos de regresión numérica — mapeo desde tests Swift

La nueva implementación debe reproducir los **mismos resultados** que `FutureFinCore.SummaryService` y las métricas orquestadas en `AppState` donde aplique. Ejecutar los tests en el repo Swift de referencia antes de declarar paridad.

**Ubicación referencia:** `tests/coreTests/SummaryServiceTests.swift`, `tests/desktopTests/AppStateMetricsTests.swift`.

## SummaryServiceTests (`SummaryService`)

| Test | Qué valida |
|------|------------|
| `snapshotReturnsHouseholdTotals` | Totales hogar, net worth, ratio deuda/activos, breakdown por categoría / etiqueta pasivos |
| `snapshotCanFilterByPerson` | Filtro `personID` en snapshot |
| `debtRatioIsZeroWhenNoAssets` | Ratio cero sin activos |
| `summaryIncludesAssetBreakdownByCategory` | Breakdown por categoría y `liabilityBreakdownByLabel` |
| `financialHealthMetricsCalculatesSavingsRunwayAndUpcomingCoverage` | Ingresos/gastos/ahorro/tasa, runway, upcoming y coverage |
| `financialHealthMetricsHandlesZeroDenominators` | Savings rate, runway, coverage en ceros |
| `financialHealthMetricsRunwayUsesOnlyLiquidAssets` | Runway solo `isLiquid` |
| `projectionSeriesAppliesDatedUpcomingAtDueDate` | Upcoming con fecha en serie |
| `projectionSeriesSpreadsUndatedUpcomingAcrossNinetyDays` | Reparto 90 días sin fecha |
| `projectionSeriesRoutesUpcomingPositiveIntoRemainingBudgetContributions` | Boost ingresos → % remanente |
| `projectionSeriesDoesNotRouteUpcomingPositiveIntoFixedContributions` | Fijos no reciben boost |
| `projectionSeriesHandlesNegativeMonthlySavings` | Ahorro negativo |
| `projectionSeriesCanAdjustForInflation` | Modo inflación |
| `projectionSeriesAppliesFixedAndRemainingBudgetContributions` | Cola fijos + % |
| `projectionSeriesIncreasesRemainingBudgetContributionsAfterDebtEnds` | Fin de deuda libera flujo |
| `fireMilestoneReachesBeforePensionWhenPrePensionTargetIsHit` | Estado FIRE antes pensión |
| `fireMilestoneCanReachAfterPensionStartsWhenPrePensionTargetMisses` | FIRE después pensión |
| `fireMilestoneReportsNotReachedWhenProjectionCannotHitTarget` | No alcanzado |
| `fireMilestoneIncreasesTargetWhenCapitalWithdrawalTaxesIncrease` | Impuestos CG suben objetivo |
| `fireMilestoneTaxesOnlyTheGainPortionOfWithdrawals` | Solo parte ganancia gravada |
| `expenseDrainReducesLiquidAssetBeforeIlliquid` | Orden drenaje líquido primero |
| `expenseDrainOverflowsFromLiquidToIlliquid` | Desborde a ilíquido |
| `expenseDrainOrdersLiquidAssetsByAscendingReturnRate` | Orden por tasa dentro de líquidos |
| `expenseDrainAlsoReducesContributedCapitalSeries` | Serie capital aportado |
| `expenseGapWidensOverTimeWhenAssetsHavePositiveGrowth` | Impacto temporal gasto |
| `earlierExpenseCreatesLargerLongTermCompoundingImpactThanLaterExpense` | Timing gasto |
| `expenseImpactStaysConstantWhenAssetGrowthIsZero` | Sin crecimiento |
| `projectionMatchesAnalyticalCompoundCurveAfterSingleExpenseDrain` | Curva analítica |
| `undrainedExpenseResidualReducesNetWorthDirectly` | Residual no drenado |

## AppStateMetricsTests (`FutureFinDesktop`)

| Test | Qué valida |
|------|------------|
| `retirementSettingsPersistAcrossAppStateRecreation` | Persistencia FIRE JSON por hogar |
| `monthlySavingsMatchesVisibleBudgetNetIncludingDerivedPaymentPlanEntries` | Budget visible incluye derivadas; métricas alineadas |
| `projectionHorizonUsesEditableTargetAge` | Horizonte años vs edad objetivo |
| `upcomingNetForMilestoneBaselineUsesDatedAndUndatedNinetyDayLogic` | Baseline 90 días + undated completo |
| `fireMilestoneUsesPrimaryMemberAgeAndUpdatesReactively` | Edad referencia primaria |
| `fireMilestoneRespondsToPensionSettings` | Ajuste pensiones |
| `fireMilestoneSupportsManualRetirementExpenseAndTaxBrackets` | Modo manual + tramos |
| `fireMilestoneRespectsSelectedPersonScope` | Alcance persona seleccionada |

## Uso recomendado en CI del nuevo stack

1. Extraer fixtures mínimos (UUIDs fijos, cantidades decimales) de cada test y portarlos a tests unitarios en el lenguaje elegido **o**
2. Ejecutar `swift test` en pipeline contra el paquete Swift como **golden master** temporal **o**
3. Generar JSON de puntos de `projectNetWorthSeries` / `fireMilestone` desde Swift y comparar en tests cruzados.

La opción 3 escala mejor cuando el motor deja de ser Swift.
