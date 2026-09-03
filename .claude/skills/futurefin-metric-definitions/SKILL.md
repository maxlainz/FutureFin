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

**Cómo encuentra a los consumidores — y por qué son TRES patrones desde 5.0.0**: el test escanea
`helpId="…"` (prop JSX), `HELP_TEXTS[…]` (incluidos los ternarios multilínea) y, nuevo,
`helpId: "…"` en **forma de objeto**. Ese tercero se añadió porque los KPIs por estrategia se
declaran como datos en `apps/web/src/lib/retirement-tiles.ts`, no como JSX: sin él, la mitad
«ningún texto huérfano» habría empujado a **borrar seis textos vivos**. Lección general: cuando
muevas la declaración de un tile de la vista a una tabla de datos, comprueba que el escáner sigue
viéndola antes de creerte el verde.
## 6. Estado del catálogo

**El recuento no se congela aquí: se cuenta.** `grep -c '^  "' apps/web/src/lib/helpTexts.ts` da
**52** el 2026-09-03 (rama `release/5.0.0`; 29 en `main`, o sea **+23 en 5.0.0** — el mayor salto
del catálogo desde que existe), y `grep -n '^  "' …` da la lista. Contraste cruzado, por si la
indentación del fichero se moviera: `grep -cE '^    title: "' …` y `grep -cE '^    body:$' …`
tienen que dar el mismo número.

| Vista | Ids |
|---|---|
| Resumen · salud financiera | `summary.savings`, `summary.liquid_assets`, `summary.runway`, `summary.net_worth`, `summary.net_return`, `summary.plan` **(5.0.0)**, `summary.success` **(5.0.0)** |
| Jubilación · plan y perfil | `retirement.target`, `retirement.crossing_reading` · `retirement.strategy` · `retirement.target_age` · `retirement.pension` · `retirement.partial` · `retirement.withdrawal_rule` · `retirement.spend_mode` · `retirement.target_basis` · `retirement.bridge_discount` · `retirement.cash_buffer` · `retirement.success_threshold` **(las once, 5.0.0)** |
| Jubilación · KPIs por estrategia (`lib/retirement-tiles.ts`) | `retirement.required_contribution` · `retirement.disposable` · `retirement.coast_month` · `retirement.coast_number` · `retirement.partial_gap` · `retirement.bridge` **(5.0.0)** |
| Jubilación · Riesgo | `retirement.bands` · `retirement.success` · `retirement.depletion_by_age` **(5.0.0)** |
| Ajustes → Plan | `settings.savings_source`, `settings.income_window`, `settings.expense_window`, `settings.window_mode`, `settings.swr`, `settings.inflation` |
| Activos | `assets.volatility` **(5.0.0)** |
| Movimientos | `expenses.expense_avg`, `expenses.income_avg`, `expenses.savings` **(4.15.0)**, `expenses.savings_rate` **(4.15.0)**, `expenses.refunds` **(4.15.0)** — `expenses.savings_transferred` y `expenses.transferred_rate` se **retiraron** en 4.15.0 |

(El estado anterior era de **22** entradas el 2026-08-31 —Ola 2: +6 de activos, ratio deuda/activos
y los 4 KPIs de Pasivos— repartidas en cinco zonas.)

**4.15.0 — el «Ahorro» de Movimientos cambia de base, y es el caso de libro del §3.** Hasta 4.14.x la
tarjeta rotulada «Ahorro»/«Traspasado a ahorro» era `−Σ(kind = savings)` — lo movido a productos de
inversión —, mientras el motor, el Resumen y los modos B/C entienden ahorro como ingresos − gastos.
Dos métricas distintas compartían palabra. Resolución: la clase `savings` se rotula **«Inversión»** en
toda la UI, la tarjeta **«Ahorro»** (`expenses.savings`) pasa a ser `totals.net_avg` = `income_avg −
expense_avg` (misma ventana y denominador que sus vecinas) con el desglose «invertido · en cuenta», y
**«Tasa de ahorro»** (`expenses.savings_rate`) es `net_avg / income_avg`. Las dos entradas nuevas dicen
lo que NO son: el «Ahorro mensual» del Resumen (`summary.savings`) sigue el modo `savings_source` y en
modo A sale del presupuesto — homónimos con base distinta, declarados en los dos textos. Entra además
`expenses.refunds` («Devoluciones»: gastos con importe positivo, ya descontados dentro de su categoría —
ni categoría aparte ni ingreso; `totals.refunds_actual/_avg`). Retiradas `expenses.savings_transferred`
y `expenses.transferred_rate` (sus consumidores desaparecen con las tarjetas; el test de cobertura lo
exige en las dos direcciones).

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

### 6.2 — 5.0.0: veintitrés entradas nuevas y una regla que se convierte en norma

La jubilación por estrategias añadió **23 entradas de golpe** —el catálogo pasa de 29 a 52— y una
sola edición: `retirement.target` («Patrimonio objetivo») conserva su texto de 4.x y le **añade** la
cláusula que lo subordina a la estrategia. Ese matiz es el cambio de semántica de la ola: el
objetivo se sigue calculando y dibujando siempre, pero **solo DECIDE la fecha en «Cuanto antes» y
«Puente hasta la pensión»**; en las estrategias por edad manda la edad y el objetivo baja a ser la
referencia contra la que se lee si llegas o no. Un texto que hubiera seguido diciendo «cuando tu
patrimonio alcanza esta cifra, te jubilas» habría sido falso en tres de las cinco estrategias.

Contrato de una línea por entrada (el texto completo vive en `helpTexts.ts`; esto es el índice):

| Id | Qué mide, en una línea |
|---|---|
| `retirement.strategy` | Qué **dispara** la jubilación y cómo se dimensiona el objetivo. Es por usuario, no del hogar |
| `retirement.target_age` | Edad a la que dejas de trabajar **en la simulación**: manda sobre el capital en «A una edad fija» y «Coast FIRE», es el fin de la fase parcial en «Media jornada», y exige fecha de nacimiento |
| `retirement.crossing_reading` | El mes en que la simulación te jubila **de verdad** (el que marcan chart y Resumen); con las estrategias por edad el «cruce del objetivo» pasa a ser **solo una lectura** |
| `retirement.pension` | Renta vitalicia **con fecha**: importe mensual en euros de hoy + edad de inicio. Su fecha cambia el **objetivo**, no solo el flujo de caja |
| `retirement.partial` | Fase de media jornada desde una edad, con un ingreso declarado en euros de hoy (0 = año sabático); el hueco hasta el gasto lo cubre el capital y termina en la jubilación total |
| `retirement.withdrawal_rule` | Cuánto sacas del patrimonio cada mes ya jubilado. **Los porcentajes son BRUTOS: el impuesto de la venta va dentro** |
| `retirement.spend_mode` | Dos lecturas de la misma regla: como **techo** (retiras lo necesario, nunca más de lo permitido) o como **gasto** (retiras lo que dice la regla haya o no necesidad). No cambia el objetivo |
| `retirement.target_basis` | Sobre qué se dimensiona el objetivo: renta perpetua (ignora la pensión) o puente hasta la pensión. **Si no eliges**, puente cuando hay pensión declarada y perpetua cuando no |
| `retirement.bridge_discount` | Tasa con la que se descuentan los años de puente (rentabilidad de tus líquidos · tu SWR · sin descuento). Solo afecta al objetivo si la base es el puente |
| `retirement.cash_buffer` | Meses de gasto siempre en efectivo. **Solo existe en los escenarios con volatilidad** y **cuesta rentabilidad** — ver abajo |
| `retirement.success_threshold` | Tu listón de escenarios sin agotar cartera (de serie 95 %). «Es tu listón, no una predicción» |
| `retirement.required_contribution` | Aportación mensual **mínima** que llega al objetivo en la edad elegida, hallada **simulando el plan entero**. Es un **techo** sobre lo que el reparto invierte, no un importe garantizado |
| `retirement.disposable` | Lo que **sobra** por encima de lo que exige la estrategia. **Dos bases declaradas** — ver la norma de abajo |
| `retirement.coast_month` | Primer mes desde el que puedes dejar de aportar y llegar igual. «No alcanzable» ≠ dato ausente |
| `retirement.coast_number` | Patrimonio **líquido** con el que se ENTRA en el mes coast. No es el objetivo de jubilación ni el patrimonio total |
| `retirement.partial_gap` | Capital a perpetuidad para lo que la media jornada NO paga (gasto − ingreso parcial − parte de pensión, con impuestos por delante, ÷ SWR). Informativo: no dispara nada |
| `retirement.bridge` | Años entre jubilación y pensión, pagados enteros por el patrimonio; el paréntesis es la tasa de retirada **efectiva** de esos años y **puede superar el SWR sin ser un error** |
| `retirement.bands` | Miles de futuros del mismo plan sorteando cada mes; la franja va del escenario 10 al 90. **La mediana es el valor central de cada mes por separado: NO es un futuro concreto** |
| `retirement.success` | % de escenarios en que la cartera aguanta el horizonte **sin agotarse**. Un **recorte** de la regla de retirada NO cuenta como fracaso: se mide aparte <!-- MC: revisar tras el pase de correcciones --> |
| `retirement.depletion_by_age` | Fracción **acumulada** de escenarios que agotaron la cartera a esa edad **o antes**; solo puede crecer con la edad |
| `summary.plan` | Estrategia + fecha y edad en que la simulación jubila de verdad. Las dos cifras del margen son **las mismas** del panel de Jubilación, copiadas, nunca recalculadas |
| `summary.success` | El KPI coloreado del Resumen; es el **MISMO sorteo** que dibuja la sección Riesgo |
| `assets.volatility` | Desviación típica **anual** del retorno del activo (no una pérdida esperada). **Solo alimenta las bandas**: la línea de proyección no la usa |

**La norma que esta ola convierte en obligación: si una cifra tiene DOS bases, el texto declara las
dos.** El precedente ya no es una anécdota, es la forma canónica —`retirement.disposable`, el mismo
campo `disposable_monthly` significando cosas distintas—:

> «La base cambia con la estrategia y por eso conviene mirarla: **con una edad objetivo es tu
> sobrante mensual máximo menos el ahorro necesario; con Coast FIRE es TODO tu sobrante, pero solo a
> partir del mes coast** — antes vale cero de verdad […]. No existe en «Cuanto antes»: ahí todo el
> ahorro va al objetivo por definición.»

Tres cosas en un párrafo: las dos bases, un **cero de verdad** distinguido de una ausencia, y el
modo en el que la métrica **no existe**. Es exactamente lo que el incidente de 3.9.0 pedía (§1) y lo
contrario de lo que la arqueología prohíbe: **declarar la base, jamás renombrar el campo** para
desambiguarlo de otro con el mismo nombre. La misma forma la usan `retirement.target_basis` (dos
bases + default implícito), `retirement.spend_mode` («dos lecturas de la misma regla») y
`retirement.bridge_discount` (tres tasas para la misma cifra).

**Una entrada documenta un resultado incómodo, y así debe quedarse.** `retirement.cash_buffer` no
promete protección: dice que el colchón **cuesta rentabilidad** y que, en el modelo actual —que
sortea cada mes de forma independiente—, **baja** la probabilidad de éxito en vez de subirla, y
manda al usuario a mirar la sección Riesgo con sus propios números. El texto se escribió **después**
de que la medición falsara la predicción del diseño; escribir la promesa que se esperaba habría
convertido una opción honesta en una recomendación falsa.
<!-- MC: revisar tras el pase de correcciones -->

**Y una que declara su propia inutilidad cuando falta el dato**: `retirement.success` y
`retirement.success_threshold` dicen las dos que **sin volatilidad declarada en los activos el
número no significa nada** (la respuesta lo publica como `any_volatility_declared: false`). Un
«éxito 100 %» sobre una cartera sin σ es aritméticamente cierto y semánticamente vacío: es la clase
de cifra que este catálogo existe para no dejar suelta.

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

Introducido en 3.9.0 junto al popover de ayuda. **Ampliado el 2026-09-03 (5.0.0, issue #207)**:
§5 (el tercer patrón del escáner de cobertura, la forma `helpId:` de objeto), §6 (recuento por
comando: 52; el mapa de vistas gana Jubilación·plan, Jubilación·KPIs, Jubilación·Riesgo y Activos) y
§6.2 (las 23 entradas nuevas, la única edición —`retirement.target` subordinado a la estrategia—, la
norma de las dos bases y las dos entradas que documentan un resultado incómodo). **Re-verificado y
ampliado el 2026-08-22 (4.0.0)**:
§6 (estado del catálogo, `retirement.target`) y §6.1 (las cuatro derivas de la auditoría previa a la
publicación, ya corregidas en `helpTexts.ts`). **Ampliado el 2026-08-28 (Fase 5 del tren 4.4.0,
issue #86)**: §7 — decisión razonada de NO dar entrada a `financial_health.basis`/`totals.basis`
(sin consumidor en la SPA hoy) y la regla permanente de las cuatro cuartetas homónimas
`get_budget.totals` ↔ `get_summary.financial_health`. El catálogo en sí (§6) no cambió: sigue en
16 entradas — este pase fue sobre campos que deliberadamente NO entraron. Re-verificación:

```bash
# Entradas del catálogo y consumidores
grep -c '^  "' apps/web/src/lib/helpTexts.ts        # 52 el 2026-09-03 (29 en main; 16 a 2026-08-25)
grep -cE '^    title: "' apps/web/src/lib/helpTexts.ts   # mismo número: contraste de indentación
grep -rn 'helpId=' apps/web/src --include='*.tsx' | wc -l   # 23 — consumidores en JSX
grep -rn 'helpId:' apps/web/src --include='*.ts' | wc -l    # 9 — consumidores en forma de OBJETO (5.0.0)
# Los nombres de producto de las 5 estrategias viven una sola vez (D33):
grep -n 'RETIREMENT_STRATEGY_LABEL' -A 6 apps/web/src/lib/retirementProfile.ts
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
