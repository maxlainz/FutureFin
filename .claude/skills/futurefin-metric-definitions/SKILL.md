---
name: futurefin-metric-definitions
description: >
  El catálogo de descripciones de métricas (`apps/web/src/lib/helpTexts.ts`) es el CONTRATO EN
  PROSA de cada cifra que FutureFin enseña: qué mide, con qué base, con qué ventana y en qué modos
  existe. Carga esta skill SIEMPRE que vayas a cambiar la semántica de una métrica o de un KPI:
  cambiar su base o su ventana, renombrarla, añadir una nueva, retirarla, o cambiar de dónde salen
  el ingreso/gasto/ahorro que la alimentan. Triggers: "añadir un KPI", "cambiar la base de", "esta
  cifra ahora sale de", "renombrar la métrica", "qué significa exactamente X", "el texto de ayuda",
  "helpTexts", "HelpPopover", "el popup dice otra cosa que el código", "tasa de ahorro", "promedio
  ponderado", "ventana del promedio". NO la uses para: la mecánica del componente de popover o
  tokens/CSS (.claude/design-system.md), las fórmulas FIRE (futurefin-fire-domain-reference), los
  ejes de configuración (futurefin-config-and-flags), ni las puertas genéricas de merge
  (futurefin-change-control — esa te enruta aquí, esta skill ES la evaluación a la que enruta).
---

# Definiciones de métricas — el catálogo es contrato

## 1. La regla

`apps/web/src/lib/helpTexts.ts` describe en español lo que cada métrica mide. **Código y texto en
desacuerdo son un bug en uno de los dos, nunca una discrepancia tolerable.** No hay una jerarquía
fija sobre cuál gana: a veces el texto describe la intención correcta y el código se desvió, y a
veces al revés. Lo que no está permitido es dejarlos divergir.

Esto existe porque el fallo que originó el catálogo no fue un error de cálculo. En 3.9.0 el Resumen
enseñaba **tres** cifras de ahorro (610,00 / 786,00 / 520,00 €) todas aritméticamente correctas y
mutuamente irreconciliables, con una tasa de ahorro que mezclaba el neto de un modo con el ingreso
de otro. Nadie mintió: simplemente ninguna tarjeta decía cuál era su base.

## 2. Puerta de merge

Todo cambio que toque la semántica de una métrica debe acabar en **exactamente uno** de:

1. **Texto actualizado** — la entrada del catálogo refleja la base nueva.
2. **Entrada añadida/retirada** — con su icono cableado o descableado en la misma vista.
3. **n/a razonado** — el cambio no altera lo que la métrica significa (refactor puro, cambio de
   formato, movimiento de fichero). Dilo en el cuerpo del commit.

Nunca en silencio. Es el mismo mecanismo probado de `futurefin-mcp-parity` §1.

## 3. Qué cuenta como «cambio de semántica»

- La **base** cambia: la cifra pasa a salir del presupuesto en vez de los movimientos, o al revés.
- La **ventana** cambia: otro número de meses, u otra forma de contarlos.
- El **denominador** cambia: meses con datos vs meses de calendario, bruto vs neto.
- Lo que se **excluye** cambia: transferencias conciliadas, meses parciales, un `kind`.
- La métrica pasa a **depender del modo** (`savings_source`) o deja de hacerlo.
- Se **renombra** una métrica visible, o dos métricas distintas comparten rótulo.

## 4. Cómo se escribe una entrada

```ts
"summary.savings": {
  title: "Ahorro mensual",
  body: "Lo que la simulación da por ahorrado cada mes, y la única cifra de ahorro con la que…",
},
```

- **Id**: `<vista>.<métrica>`, en minúsculas y con punto. El punto no es decorativo: el test de
  cobertura lo usa para distinguir un id de cualquier otra cadena del código.
- **Título**: ≤ 40 caracteres, el mismo rótulo que ve el usuario en la tarjeta.
- **Cuerpo**: > 60 caracteres, un par de frases. Español, tuteando, **sin jerga de
  implementación** — nada de «endpoint», «JSONB», «engine», «promedio ponderado».
- **La base siempre explícita.** Si la cifra depende del modo o de una ventana, dilo. Si algo queda
  fuera del cálculo (las transferencias conciliadas, el mes en curso), dilo.
- **Di lo que la métrica NO es** cuando se parece a otra. «Traspasado a ahorro» lleva
  explícitamente «no es ingresos menos gastos» porque durante dos versiones compartió rótulo con
  una cifra que sí lo era, y diferían en 11 puntos.

## 5. El test de cobertura

`apps/web/src/lib/helpTexts.test.ts` comprueba las **dos** direcciones:

- Ningún icono apunta a un texto inexistente (el popover saldría vacío).
- Ningún texto queda huérfano — sin consumidor sigue describiendo una métrica que quizá cambió, y
  nadie se entera. Esta mitad es la que importa a largo plazo.

Si retiras una métrica, **retira su texto**; no lo dejes «por si vuelve». El test lo caza.

## 6. Estado del catálogo (2026-08-25)

**16 entradas** repartidas en cuatro zonas de la app. `grep -c '^  "' apps/web/src/lib/helpTexts.ts` para el recuento;
`grep -n '^  "' …` para la lista. Por si necesitas orientarte sin abrir el fichero:

| Vista | Ids |
|---|---|
| Resumen · salud financiera | `summary.savings`, `summary.liquid_assets`, `summary.runway`, `summary.net_worth`, `summary.net_return` **(nueva)** |
| Jubilación | `retirement.target` **(nueva en 4.0.0)** |
| Ajustes → Plan | `settings.savings_source`, `settings.income_window`, `settings.expense_window`, `settings.window_mode`, `settings.swr`, `settings.inflation` |
| Movimientos | `expenses.expense_avg`, `expenses.income_avg`, `expenses.savings_transferred`, `expenses.transferred_rate` |

**`retirement.target` — «Patrimonio objetivo»** (4.0.0). La métrica más cara de la app no tenía
texto. Lo que dice, y por qué cada trozo: el objetivo es el gasto anual en jubilación **grosseado
por impuestos si están activados** dividido entre el SWR; **la cifra grande está en euros de hoy** y
**el paréntesis es ese mismo objetivo llevado al mes del cruce con la inflación configurada**. Ese
último matiz es el que existía mal en la interfaz: el rótulo «Patrimonio objetivo (con inflación)»
etiquetaba justo la cifra que NO la lleva. Si tocas la base del target, el gross-up o el SWR, esta
entrada es la que hay que revisar (y ver `futurefin-fire-domain-reference`).

**`summary.net_return` — «Rendimiento neto»** (2026-08-25). Rendimiento anual **esperado** del
patrimonio neto: `Σ valor·rentabilidad − Σ principal·TAE` sobre el patrimonio neto, con los
pasivos vencidos fuera (mismo filtro que el resto del Resumen). Tres cosas que el texto dice a
propósito, y que son justo las que se rompen en silencio si alguien toca el cálculo: (1) un activo
**sin rentabilidad configurada cuenta 0 % y sigue pesando en el denominador** —diluye, no se
excluye—; (2) la cifra grande es la **real** y el paréntesis la **nominal**, y la real sale de
dividir factores, no de restar puntos; (3) **no es rentabilidad realizada**, y **no cuadra con la
proyección**, que solo cobra intereses a **algunas** deudas — es la única entrada del catálogo que
documenta una divergencia viva de modelo, y esconderla habría convertido «¿por qué la simulación va
más rápido que mi rendimiento?» en un bug fantasma.

**Actualizada en 4.2.0** — y es el caso de libro de por qué esta skill es un gate. La entrada
decía «la proyección… todavía no le cobra los intereses a tus deudas» y avisaba: «si alguna vez el
engine empieza a cobrar el interés de la deuda, esta entrada es la que hay que revisar». 4.2.0 es
ese día: el engine devenga interés en los pasivos con `repayment_model` francés o revolving y plan
de pago activo. La frase no se borra, se **matiza**, porque la divergencia se estrecha pero **no
desaparece**: el KPI cuenta la TAE de **todas** las deudas vivas, sin condiciones, así que sigue
siendo algo más prudente que la simulación mientras quede alguna deuda en cuota fija (el default de
la columna, o sea: todas las que existían antes de 4.2.0). Para un hogar que declare su hipoteca
como francesa, las dos cifras convergen. Si algún día `fixed_payments` deja de ser el default, o el
KPI aprende a mirar el modelo, esta entrada vuelve a tocar.

### 6.1 Auditoría de 4.0.0 — seis entradas a la deriva, y el patrón que las produjo

Seis de las quince (las cuatro de Movimientos, `summary.runway` y `summary.savings`) seguían
describiendo una métrica que el código había dejado atrás **sin que nada fallara**: el test de cobertura (§5) comprueba que cada texto tiene consumidor y cada
consumidor texto, pero **no puede comprobar que el texto sea verdad**. Esa mitad es humana, y es
justo la que esta skill existe para forzar.

| Id | Decía | Realidad (código) |
|---|---|---|
| `expenses.expense_avg` · `income_avg` · `savings_transferred` · `transferred_rate` | Nada sobre qué meses entran, más allá de «meses reales» | El tramo es **medio-abierto `[window_start, selected)`** (`transactions/summary.rs`: `in_window = ym >= window_start_ym && ym < selected_ym`): **el mes que estás mirando NO se promedia**. Y las **transferencias conciliadas** quedan fuera de todos los buckets desde 3.5.0 |
| `summary.runway` | «tus activos líquidos cubrirían tu gasto», sin decir qué gasto | La base es `expense_total_monthly_equivalent`, que **sigue el modo** `savings_source`: presupuestado en A, promedio real en B/C |
| `summary.savings` | Mandaba a «Ajustes → Proyección» | Esa sub-pestaña se llama **«Plan»** desde 3.10.0 (`SETTINGS_SUBTAB_LABEL`, `lib/navigation.ts`) |

**El patrón**: ninguna de las seis fue un cambio de métrica «con su texto olvidado». Fueron
cambios de OTRA cosa —el predicado de mes real, la conciliación, el modo de ahorro, el nombre de
una sub-pestaña— que **movieron el significado de una métrica de rebote**. La §3 ya lo cubre en
teoría («lo que se excluye cambia», «pasa a depender del modo»); lo que faltaba era aplicarla
cuando el cambio no se siente como «tocar una métrica». Regla práctica: si tu cambio altera **qué
filas entran en un agregado** o **cómo se llama algo que un texto cita**, `grep` el id en
`helpTexts.ts` antes de cerrar.

## 7. Campos declarativos que no son texto de ayuda: `basis` y las marcas de unidad (4.4.0)

La Fase 5 del tren MCP (issue #86) añadió dos campos **declarativos** — no cambian ninguna cifra,
declaran su procedencia —: `financial_health.basis` (`GET /v1/summary`, `"plan"` | `"actual"` |
`"mixed"`, derivado de los dos `savings_*_basis` que ya existían) y `totals.basis`
(`GET /v1/budget`, constante `"plan"`, `BUDGET_TOTALS_BASIS` en `handlers/budget.rs`).

**Decisión: no entran en `helpTexts.ts`.** Dos razones, no una:

1. **Hoy no tienen consumidor en la SPA.** `apps/web/src/api/types.ts` no tipa ninguno de los dos
   campos (verificado: ningún `basis` en `FinancialHealthMetrics`/`BudgetTotalsApi`) y ningún
   `.tsx` los lee. Una entrada sin `helpId=` que la cite es exactamente la mitad huérfana que el
   test de cobertura (§5) existe para cazar — añadirla habría sido un texto correcto el día de
   hoy y sin dueño, la misma clase de deriva silenciosa que §6.1 documenta.
2. **Su prosa ya tiene sitio, y no es este catálogo.** Ambos campos hablan a quien lee el JSON
   directamente — un cliente MCP comparando `get_budget.totals` con
   `get_summary.financial_health` —, no a una persona mirando una tarjeta del Resumen. Esa prosa
   vive donde debe: el doc-comment de `basis` en `FinancialHealthMetrics`
   (`apps/api/src/handlers/summary.rs`) y en `BudgetTotalsResponse`
   (`apps/api/src/handlers/budget.rs`), que fluye a OpenAPI y a la descripción de la tool MCP.

Si algún día la SPA pinta un badge «plan» / «real» sobre estas tarjetas, ESE es el momento de
darle entrada aquí — el mismo criterio que hizo esperar a `retirement.target` hasta que 4.0.0 le
puso una tarjeta (§6). Hasta entonces, `grep -rn 'financial_health\.basis\|totals\.basis'
apps/web/src` en vacío es la señal de que la decisión sigue vigente — ojo, un `grep 'basis'` a
secas NO sirve de prueba: `SavingsAvgBasisApi` (`savings_income_basis`/`savings_expense_basis`,
ya consumidos por `ProjectionNetWorthChart.tsx`) y el `basis` de los markers históricos son campos
homónimos preexistentes y no tienen nada que ver con este.

**Lo que sí es una regla de lectura permanente, entre o no en el catálogo**: `get_budget.totals`
y `get_summary.financial_health` comparten CUATRO nombres de campo —
`income_monthly_equivalent`, `expense_regular_monthly_equivalent`,
`expense_total_monthly_equivalent`, `net_monthly_equivalent` — y valen cosas distintas. Los de
`budget` son SIEMPRE el plan (`totals.basis == "plan"`, constante). Los de `summary` siguen
`fire_settings.savings_source`: modo A (`budget`, default) coincide con el plan; modos B
(`transactions_avg`) y C (`budget_income_real_expense`) son el promedio real. **Regla: si
`financial_health.basis != "plan"`, las dos cuartetas NO son comparables campo a campo** — restar
una de la otra no es un error de tipos, es un error semántico silencioso. Es la misma familia de
incidente que abrió este catálogo (§1: las tres cifras de ahorro de 3.9.0, correctas y
mutuamente irreconciliables porque nada decía su base). La diferencia esta vez es que el propio
dato declara su base — por eso el arreglo fue un campo nuevo, no una entrada de texto nueva.

**Por qué no se renombraron los cuatro campos en su lugar**: renombrar (p. ej.
`net_monthly_equivalent` → algo que lleve el modo en el nombre dentro de `financial_health`) es
breaking sobre seis campos que la SPA ya lee, y **no** habría arreglado nada — seguirías sin saber
en qué modo está el summary sin mirar `basis`. Lo que faltaba no era un nombre distinto: era
declarar la procedencia.

**Marcas de unidad (`**Unidad:**`) — el mismo argumento, un nivel más abajo.** La misma auditoría
anotó cada campo de `FinancialHealthMetrics` con su unidad en el doc-comment
(`apps/api/src/handlers/summary.rs`) en vez de sufijarla en el nombre (`savings_rate` →
`savings_rate_fraction`, `debt_to_assets_ratio` → …). Motivo: la unidad es propiedad del CAMPO,
constante en todas las respuestas — su sitio es el esquema (fluye a OpenAPI y a la tool MCP), no
200 bytes repetidos en el endpoint más caliente de la app. Regla de lectura vigente en todo el
API, no solo en `financial_health`: un campo `_rate`/`_ratio` es **fracción** (`0.35` = 35 %); uno
`_pct`/`_percent` es **porcentaje** (`3.5` = 3,5 %). No es una convención nueva — ya regía
(`swr_pct`, `savings_rate`); lo nuevo es que ahora está escrita donde un cliente la puede leer sin
adivinar. Verificable sin compilar: `grep -n '\*\*Unidad:' apps/api/src/handlers/summary.rs`.

**Deriva comprobada en esta pasada**: releídas las 16 entradas de `helpTexts.ts` contra la Fase 5
(nuevo default de ventana en `/v1/history/series`, `view` ecoado, `events` en la proyección,
`source: capture|backfill`, `fine_absent_reason`…), ninguna quedó falsa — la Fase 5 no tocó base
ni ventana de ninguna métrica ya catalogada, solo añadió procedencia a datos que el catálogo no
describe (no hay entrada de histórico ni de proyección-como-serie en `helpTexts.ts`; esas vistas
usan el chart, no tarjetas con popover).

## 8. Provenance and maintenance

Introducido en 3.9.0 junto al popover de ayuda. **Re-verificado y ampliado el 2026-08-22 (4.0.0)**:
§6 (estado del catálogo, `retirement.target`) y §6.1 (las cuatro derivas de la auditoría previa a la
publicación, ya corregidas en `helpTexts.ts`). **Ampliado el 2026-08-28 (Fase 5 del tren 4.4.0,
issue #86)**: §7 — decisión razonada de NO dar entrada a `financial_health.basis`/`totals.basis`
(sin consumidor en la SPA hoy) y la regla permanente de las cuatro cuartetas homónimas
`get_budget.totals` ↔ `get_summary.financial_health`. El catálogo en sí (§6) no cambió: sigue en
16 entradas — este pase fue sobre campos que deliberadamente NO entraron. Re-verificación:

```bash
# Entradas del catálogo y consumidores
grep -c '^  "' apps/web/src/lib/helpTexts.ts        # 16 a 2026-08-25 (15 el 2026-08-22)
grep -rn 'helpId=' apps/web/src --include='*.tsx' | wc -l
# Los dos hechos que §6.1 afirma sobre el código, sin compilar:
grep -n 'in_window\|window_start_ym' apps/api/src/handlers/transactions/summary.rs  # tramo medio-abierto
grep -n 'plan:' apps/web/src/lib/navigation.ts                                      # la sub-pestaña se llama «Plan»
# Los hechos que §7 afirma sobre el código, sin compilar:
grep -n 'pub basis: &.static str' apps/api/src/handlers/summary.rs apps/api/src/handlers/budget.rs
grep -n 'BUDGET_TOTALS_BASIS' apps/api/src/handlers/budget.rs
grep -n '\*\*Unidad:' apps/api/src/handlers/summary.rs
# vacío hoy = todavía sin consumidor en la SPA. NO uses `grep 'basis'` a secas: da falsos
# positivos por SavingsAvgBasisApi y el `basis` de los markers históricos, que no son este campo.
grep -rn 'financial_health\.basis\|totals\.basis' apps/web/src
# Las dos direcciones de cobertura
npm test --workspace futurefin-web -- helpTexts
```
