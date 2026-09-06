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

**El recuento no se congela aquí: se cuenta.** Aquí no se escribe la cifra — se escribe el comando
que la da, y quien la necesite la mide:

```bash
grep -c '^  "'  apps/web/src/lib/helpTexts.ts   # cuántas entradas hay
grep -n '^  "'  apps/web/src/lib/helpTexts.ts   # CUÁLES son — esta es la que importa
grep -cE '^    title: "' apps/web/src/lib/helpTexts.ts   # contraste cruzado: los tres
grep -cE '^    body:$'   apps/web/src/lib/helpTexts.ts   # tienen que dar el mismo número
```

La historia del número explica por qué no se congela: 29 en `main`, 52 al abrir 5.0.0, 55 tras U1b,
53 tras la tercera vuelta de UX y **de nuevo un número distinto** tras el modelo v2, que retiró diez
entradas y estrenó ocho. **El catálogo ha vuelto al mismo total por caminos distintos más de una
vez**: el recuento no identifica un estado, solo delata que algo se movió — por eso la LISTA importa
más que la cifra, y por eso la tabla de abajo es la que se mantiene al día.

| Vista | Ids |
|---|---|
| Resumen · salud financiera | `summary.savings`, `summary.liquid_assets`, `summary.runway`, `summary.net_worth`, `summary.net_return`, `summary.plan` **(5.0.0)**, `summary.success` **(5.0.0)** |
| Jubilación · plan y perfil (cableado como TABLA en `lib/retirement-form.ts`, `PLAN_FIELD_HELP`) | `retirement.strategy` (cuelga del `<h4>` de la tarjeta «Estrategia») · `retirement.target_age` · `retirement.coast_mode` **(v2)** · `retirement.partial_mode` **(v2)** · `retirement.partial` · `retirement.pension` · `retirement.bridge_settings` **(v2)** · `retirement.success_threshold` **(v2 — vuelve, con otro sujeto: ver §6.4)** · `retirement.withdrawal_rule` · `retirement.spend_mode` · `settings.swr` y `settings.horizon_age`, que hoy cuelgan de este formulario y no de Ajustes |
| Jubilación · frase-hito (`RetirementView.tsx`, U1b) | `retirement.plan_sentence` (cabecera de «Resultado» — HelpPopover del panel, no de una tarjeta) **(5.0.0, U1b, #207)** |
| Jubilación · resultado (`lib/retirement-tiles.ts`) | tiles FIJOS `retirement.needed_capital` **(v2)** y `retirement.success`; el tercero por estrategia — `retirement.safe_date` **(v2, `asap`)** · `retirement.required_contribution` (`retire_at_age`) · `retirement.coast_month` (`coast`) · `retirement.partial_mode` (`partial`); en «Detalle del cálculo», `retirement.fire_number_classic` **(v2)** y las dos cotas de `retirement.safe_date` (fechas al 100 % y al 90 %) |
| Jubilación · Riesgo (`lib/risk-bands.ts`) | `retirement.bands` · `retirement.success` (fila del primer tipo de fallo) · `retirement.failure_by_age` **(v2)** · `retirement.coverage` (sus dos filas) · `retirement.success_threshold` (fila del suelo de Wilson) |
| Proyección | `retirement.needed_capital` — el tile «Capital necesario hoy» de `ProjectionView.tsx` es la MISMA cifra, al euro, que el de Jubilación **(v2)** |
| Ajustes → Plan | `settings.savings_source`, `settings.income_window`, `settings.expense_window`, `settings.window_mode`, `settings.inflation`, `settings.taxable_gain` |
| Activos | `assets.expected_return` (reescrita en v2: es un CAGR) · `assets.volatility` **(5.0.0)** |
| Movimientos | `expenses.expense_avg`, `expenses.income_avg`, `expenses.savings` **(4.15.0)**, `expenses.savings_rate` **(4.15.0)**, `expenses.refunds` **(4.15.0)** — `expenses.savings_transferred` y `expenses.transferred_rate` se **retiraron** en 4.15.0 |

(El estado anterior era de **22** entradas el 2026-08-31 —Ola 2: +6 de activos, ratio deuda/activos
y los 4 KPIs de Pasivos— repartidas en cinco zonas.)

**`retirement.plan_sentence` (5.0.0, U1b, #207) — la frase-hito es también una superficie de
métrica**, no solo copy de layout. «Tu hito de jubilación» describe una lectura DERIVADA —qué
dispara el mes que se enseña, y que depende de la estrategia (capital para `asap`/`pension_bridge`,
edad para las tres restantes)—, no un campo crudo de la respuesta; su HelpPopover cuelga del título
del panel «Resultado», no de una tarjeta, que es la primera entrada del catálogo en ese sitio.

**`retirement.assumptions` — la única entrada que describía una LISTA, y por qué se retiró
(V3, 2026-09-05).** «Supuestos del plan» documentaba de golpe retirada, regla, horizonte, colchón y
umbral: no una cifra, sino la lectura de conjunto de la línea «Supuestos: …» que encabezaba el
acordeón «Avanzado». Retirado el acordeón (los campos están hoy en su tarjeta, a la vista), la línea
se quedó sin sujeto y el texto sin superficie — el test de cobertura bidireccional lo habría cazado
como huérfano. **La lección para el catálogo**: una entrada que describe un CONTENEDOR y no una
cifra vive exactamente lo que viva ese contenedor. Las entradas que describían esas mismas cifras
por separado (`retirement.withdrawal_rule`, `retirement.cash_buffer`…) siguen todas vivas, que es
justo por lo que su retirada no dejó ningún hueco.

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

**HISTORIA, no estado — `retirement.target` («Patrimonio objetivo», 4.0.0–4.15.x) murió ENTERO con
el modelo v2 (§6.4): no confundir con la entrada vigente.** Hasta la tercera vuelta de UX la
métrica más cara de la app era el gasto anual en jubilación **grosseado por impuestos si están
activados** dividido entre el SWR, **más** el término finito de deuda, y el mismo id alimentaba
**dos tarjetas que no medían lo mismo**: en Jubilación (`jubilacion_target_net_worth`, el objetivo
evaluado en el mes 0) y en Proyección (`jubilacion_target_net_worth_nominal`, el objetivo del mes
del cruce). El párrafo llegó a estar falso en Proyección y sostuvo el bug de 2,31× (F11, 5.0.0) —
la razón por la que en su día mereció una entrada tan larga. **Con el modelo v2 los dos campos, la
tarjeta «Objetivo» y el propio id `retirement.target` se retiraron ENTEROS** (`grep -c
'"retirement.target"' apps/web/src/lib/helpTexts.ts` → 0; `grep -rn jubilacion_target_net_worth
apps/web/src` sale vacío): no hay ya ni base que declarar ni cruce que disparar. La cifra de
referencia hoy es `retirement.needed_capital` («Capital necesario hoy»), la MISMA en Jubilación,
Resumen y Proyección — ver la tabla de §6 y §6.4. Si tocas el gross-up o el SWR del NÚMERO FIRE
CLÁSICO (que sí sobrevive, informativo, como `retirement.fire_number_classic`), esta nota explica
de dónde viene la vieja tarjeta; para la semántica vigente ve a `retirement.needed_capital` y a
`futurefin-fire-domain-reference` §7.

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

> **Historia, no estado.** Esta sección cuenta la PRIMERA ola de 5.0.0. El modelo v2 (§6.4) retiró
> diez de aquellas entradas y reescribió otras tantas: la tabla de una línea que hay debajo ya está
> actualizada al estado vigente, pero los párrafos que la rodean describen decisiones que v2
> sustituyó (el objetivo como disparador, el colchón derivado, el corte fijo al 100 %). Se conservan
> porque explican POR QUÉ se llegó hasta ahí; para saber qué mide hoy una cifra, la tabla y §6.4.

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
| `retirement.plan_sentence` | El resultado del plan en una frase: mes, edad y cuántos escenarios de cada 100 aguantan. Qué manda —el sorteo o la edad que pediste— depende de la estrategia; «nunca» es una respuesta, no un hueco |
| `retirement.strategy` | Qué le pides al plan y, con ello, qué te pregunta. **Cuatro** estrategias: el puente dejó de ser una de ellas. Es por usuario, no del hogar |
| `retirement.target_age` | Edad a la que dejas de trabajar **en la simulación**: manda en «A una edad fija», es la edad contra la que Coast resuelve la parada, y en la jornada reducida es opcional (fin de la fase). Exige fecha de nacimiento |
| `retirement.coast_mode` | **(v2)** Cuál de las dos edades fijas TÚ en Coast; la otra la resuelve el plan. En los dos modos **el plan simulado deja de aportar de verdad** desde el mes coast, y el ahorro liberado es gasto disponible: no vuelve a la cartera |
| `retirement.partial_mode` | **(v2)** Cuándo empieza la jornada reducida (a una edad / en cuanto pueda). La jubilación total **es la fecha válida con la fase dentro**, no una edad aparte. El ingreso de la fase es plano nominal. **NO es la jubilación parcial de la Seguridad Social** |
| `retirement.partial` | Los datos de la fase: inicio, ingreso y base del gasto. El ingreso va en euros de hoy y se queda PLANO; el hueco lo cubre la cartera, y el mes que no lo cubra ese escenario falla |
| `retirement.pension` | Renta vitalicia **con fecha**: entra como un ingreso más el mes en que arranca. **Ya no dimensiona ningún objetivo** — lo que mueve su fecha es cuántos años tiene que pagar tu capital antes de que llegue |
| `retirement.bridge_settings` | **(v2)** El puente como AJUSTE (tarjeta Pensión, apagado por defecto, disponible en cualquier estrategia): sube el tope de la tasa **inicial** cuando la pensión entra dentro de los años máximos, y la fecha válida nunca cae antes de «pensión − esos años». Durante el puente no hay tope mensual |
| `retirement.success_threshold` | **(v2 — vuelve con OTRO sujeto)** La parte de los escenarios que deben aguantar hasta el horizonte: una **restricción que decide la fecha**, no un corte de semáforo. Se evalúa contra el SUELO del intervalo de Wilson, no contra el porcentaje grande; al 100 % son cero fallos de N y se publica la cota de la regla de tres. Default 95, rango 80–100 |
| `retirement.withdrawal_rule` | Cuánto sacas del patrimonio cada mes ya jubilado. Con las reglas por saldo, el mes en que lo permitido no cubre el gasto ordinario **ese escenario falla**. La tasa de retirada es aparte: es el tope del PRIMER año. **Los porcentajes son BRUTOS: el impuesto de la venta va dentro** |
| `retirement.spend_mode` | Dos lecturas de la misma regla: como **techo** (retiras lo necesario, nunca más de lo permitido) o como **gasto** (retiras lo que dice la regla haya o no necesidad). No mueve la fecha por sí solo: mueve cuánto sale de la cartera y con ello cuántos escenarios aguantan |
| `retirement.needed_capital` | **(v2)** El líquido que haría falta HOY, con tu mezcla de activos, para que jubilándote ya aguanten los escenarios que pide tu umbral. **Euros de hoy siempre**, redondeado a cientos hacia arriba. La curva del chart es la misma cifra por edad, sin escalar: **no tiene por qué cruzar tu línea** |
| `retirement.safe_date` | **(v2)** Primer mes cuyo éxito cumple el umbral, **cada camino con su propia acumulación**; el mes publicado va confirmado con 2.500 caminos. Al lado, las fechas al 100 % y al 90 %, que la acotan. «Nunca» es un resultado |
| `retirement.required_contribution` | Aportación mensual **mínima** que hace cumplir el UMBRAL en la edad elegida, resuelta **con el sorteo**. Es **plana nominal** (sale más alta que si se indexara) y es un **techo** sobre lo que el reparto invierte, no un importe garantizado |
| `retirement.coast_month` | Primer mes desde el que puedes dejar de aportar y llegar igual; **desde ahí el plan simulado ya no aporta**. «No puedes parar nunca» ≠ dato ausente |
| `retirement.fire_number_classic` | **(v2)** Gasto anual de jubilación ÷ tu tasa de retirada (25× con el 4 % clásico), con los impuestos por delante y **sin restar la pensión**. Lectura informativa: no decide ni la fecha ni el capital |
| `retirement.bands` | Miles de futuros del mismo plan. **UNA línea** —tu trayectoria central, con la rentabilidad COMPUESTA— y la franja del escenario 10 al 90. **No hay línea de mediana** (issue #216) y los bordes no son futuros concretos. **El color no es decorativo**: es la probabilidad de fallo acumulada |
| `retirement.success` | % de escenarios en los que NO tienes que volver a trabajar jubilándote en la fecha del plan (ninguno de los tres fallos). **El color se compara con TU umbral y contra el suelo de Wilson**, no con un listón fijo — se acabó el corte al 100 % de V7. Sin volatilidad declarada no mide nada |
| `retirement.failure_by_age` | **(v2, sustituye a `depletion_by_age`)** Probabilidad **acumulada** de haber fallado a esa edad por CUALQUIERA de los tres motivos (sin dinero · tasa inicial por encima del tope · la regla no cubre el gasto). Tiñe la banda del gráfico y el hover dice el tipo |
| `retirement.coverage` | Dos lecturas de la cobertura real de tu gasto de jubilación, **en TODAS las reglas de retirada, incluida `fixed_real`**: meses por debajo del gasto (mediana) y qué fracción de la necesidad se pagó de verdad — contando tanto lo que la regla se negó a sacar como lo que la cartera no pudo financiar, y **nunca el exceso**: un mes generoso de una regla por saldo no compensa uno corto, por eso no pasa del 100 %. Son medianas: si más de la mitad de los escenarios aguanta, salen cero meses y el gasto entero |
| `summary.plan` | Estrategia + la frase del plan. El éxito y el capital necesario de al lado son **los mismos** del panel de Jubilación, copiados del mismo sorteo, nunca recalculados |
| `summary.success` | El KPI coloreado del Resumen, juzgado contra el umbral del perfil; es el **MISMO sorteo** que dibuja la sección Riesgo |
| `assets.expected_return` | **(v2)** La rentabilidad **anualizada** que publica tu fondo, tratada como COMPUESTA: el escenario central crece a ese ritmo y la media aritmética de todos queda por encima. Sigue siendo NOMINAL |
| `assets.volatility` | Desviación típica **anual** del retorno del activo (no una pérdida esperada). **Es lo que separa los escenarios**: sin ella el éxito sale 0 % o 100 % y deja de medir riesgo; subirla no mueve el centro, abre el abanico |

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

**Una entrada cambia de SUJETO sin cambiar de nombre, y por eso se reescribió entera (V6).**
`retirement.cash_buffer` describía un ajuste que el usuario declaraba en meses. Desde V6 describe un
valor que el servidor **DERIVA del tope de su regla de ahorro**, y las tres cosas que un texto de
métrica tiene que decir cuando eso pasa son: **de dónde sale** («lo inferimos del tope de tu regla»,
no «lo has pedido»), **dónde se cambia** (en Reglas de ahorro, no en esta pantalla) y **qué unidad
es de verdad** (el tope en euros, nominal y fijo; los meses son una equivalencia informativa,
porque un colchón indexado valdría casi el doble a veinte años). Sin la primera, un valor que
aparece solo se lee como un ajuste que alguien hizo.

Y el **signo del efecto va sin matizar**: el dinero fuera del mercado **resta** puntos de
probabilidad de éxito. La versión anterior contaba las dos caras («protege +3,9 pp si renta como tu
cartera, cuesta −3,5 pp netos con una cuenta al 0 %») porque el usuario ELEGÍA el colchón y merecía
el balance completo para decidir. Ahora no lo elige: le llega derivado de otra decisión suya, y lo
único accionable es el precio. El owner aceptó ese precio al elegir V6, y el texto lo dice en vez de
esconderlo detrás de un condicional que casi nadie cumple.

**Y una que declara su propia inutilidad cuando falta el dato**: `retirement.success` dice que
**sin volatilidad declarada en los activos el número no significa nada** (la respuesta lo publica
como `any_volatility_declared: false`). Un «éxito 100 %» sobre una cartera sin σ es aritméticamente
cierto y semánticamente vacío: es la clase de cifra que este catálogo existe para no dejar suelta.
`retirement.success_threshold` decía lo mismo y **se retiró en V7**: el listón dejó de ser del
usuario, así que ya no había ajuste que describir. (**Y volvió en el modelo v2 con otro sujeto** —
la restricción que decide la fecha, no un corte de color: §6.4.)

### 6.3 — U1b: la cabecera de resultados es un TOPE, no un catálogo distinto (5.0.0, U7/#207)

El rediseño UX (`RetirementView.tsx`, commit `debc52d`) no cambió el TEXTO de ninguna de las seis
entradas de §6.2 que alimentan las tarjetas de estrategia — **sigue siendo el mismo modelo, solo
cambia cuántas tarjetas se enseñan a la vez y dónde va lo que no cabe**. La regla de esta sección:
si tocas la base de una de estas cifras, revisa su entrada de §6.2; si tocas CUÁNTAS se enseñan o
en qué orden se caen, esta es la que hay que revisar.

- **El tope sigue siendo 3** (`RETIREMENT_TILES_V2_CAP`, `lib/retirement-tiles.ts`), pero **el
  modelo v2 rehízo su contenido** (§6.4): las dos primeras son FIJAS —«Capital necesario hoy»
  (`retirement.needed_capital`) y «Éxito del plan» (`retirement.success`), siempre en ese orden— y
  la tercera la pone la estrategia: `asap` → «Fecha válida» (`retirement.safe_date`);
  `retire_at_age` → «Aportación mínima» (`retirement.required_contribution`); `coast` → «Mes coast»
  (`retirement.coast_month`); `partial` → «Inicio de la jornada reducida»
  (`retirement.partial_mode`). Ya no hay truncado por el final: con el plan sin resolver se emiten
  solo las dos fijas, con su razón — una tercera tarjeta con guion afirmaría que esa cifra se
  calcula y hoy falta, y lo cierto es que esa estrategia no hace esa pregunta.
- **Lo que no cabe NO desaparece, baja a «Detalle del cálculo»** (`retirementDetailRows`): el
  número FIRE clásico (`retirement.fire_number_classic`), las fechas al 100 % y al 90 % (las dos
  con `retirement.safe_date`, que es la cifra que acotan) y la semilla y los caminos del sorteo —
  sin rótulo de ayuda: es la fila que hace auditable todo lo demás—, más los avisos de
  `buildRetirementNotices`. La regla no cambia: **una fila del detalle no es una entrada nueva del
  catálogo** mientras describa una cifra que ya tiene texto.
- **El puente se mide correctamente desde U1b (fix S8)**: `pension_start_month_index −
  jubilacion_month_index`, los dos en la MISMA rejilla 0-based (mes 0 = hoy), nunca «meses desde
  hoy hasta la pensión» (el bug de la v1: un puente real de 12 años se leía como 22 porque incluía
  los años que faltan para jubilarse — 35 años y 10 meses en la demo en vez de 12). El rótulo de la
  tarjeta es «Puente 60→72» cuando hay edades de los dos extremos (jubilación → inicio de la
  pensión), o «Puente hasta la pensión» sin ellas.
- **`retirement.plan_sentence` es ahora la primera lectura, no las tarjetas** (§6, fila nueva):
  antes de U1b el resultado se leía en tres tarjetas («Jubilación», «Años», «Edad») que había que
  volver a juntar; ahora es una oración (`lib/plan-sentence.ts`) y las tarjetas son el detalle.
- **`pct_source` (U0/U4, campo del contrato, no una entrada del catálogo)**: `withdrawal_rule.pct`
  y `hybrid.start_pct` pasan a opcionales — ausentes, se resuelven contra `swr_pct` en
  `resolve_withdrawal_rule` (servidor) / `effectiveWithdrawalPct` (cliente, `lib/retirementProfile.ts`)
  y la respuesta publica `pct_source: "swr" | "explicit"` (ausente en `fixed_real`, que no retira
  un porcentaje). No es un campo declarativo del mismo tipo que `basis`/`totals.basis` (§7): no
  describe la PROCEDENCIA de un agregado, describe si un valor concreto fue TECLEADO o HEREDADO, y
  su lectura vive en la UI como texto derivado (`withdrawalPctNote`, `lib/retirement-form.ts`:
  «Retira el X %: tu tasa de retirada» si `inherited`, «Regla al X %, fijado por API» si alguien
  puso un `pct` explícito por HTTP/MCP) — no entra en `helpTexts.ts` por la misma razón que §7 da
  para `basis`: hoy solo tiene un consumidor (esa nota bajo el selector de regla), y si algún día
  gana una tarjeta propia, ese es el momento de darle entrada aquí.

### 6.4 — El modelo v2: diez entradas mueren con el objetivo, nacen ocho (5.0.0, WP W8)

«El éxito define la fecha» no fue un retoque de copy: **retiró tres conceptos enteros** del producto
—el patrimonio objetivo como disparador, su base (perpetuidad / puente descontado) y el colchón de
caja— y con ellos sus textos. Las tres listas, para no tener que deducirlas del diff:

**Mueren** (su superficie desapareció; se BORRAN, no se comentan «por si vuelven»):
`retirement.target` y `retirement.crossing_reading` (ya no hay capital objetivo ni cruce que
dispare nada: la fecha la decide cuántos escenarios aguantan) · `retirement.target_basis` y
`retirement.bridge_discount` (no hay objetivo que dimensionar ni años que descontar: la pensión es
un flujo de caja más) · `retirement.cash_buffer` (la caja es un activo; la regla de ahorro dice
cuánto se guarda) · `retirement.disposable`, `retirement.coast_number`, `retirement.partial_gap` y
`retirement.bridge` (KPIs derivados del objetivo, sustituidos por las tarjetas de §6 «resultado») ·
`retirement.depletion_by_age` (agotarse pasó a ser **uno** de los tres fallos, no el único).

**Nacen ocho**: `retirement.needed_capital`, `retirement.safe_date`, `retirement.success_threshold`,
`retirement.failure_by_age`, `retirement.fire_number_classic`, `retirement.coast_mode`,
`retirement.partial_mode`, `retirement.bridge_settings` — definiciones en la tabla de §6.2.

**`retirement.success_threshold` es el caso que merece una regla**: el id **vuelve** (existió hasta
V7, que lo retiró al fijar el corte al 100 %) y **no significa lo mismo**. Antes describía un listón
de color; ahora es la RESTRICCIÓN que decide la fecha, evaluada sobre el suelo del intervalo de
Wilson. Un id reciclado es más peligroso que uno nuevo: nadie lo lee como estreno, y el texto viejo
habría sonado plausible. **Regla: si un id vuelve con otro sujeto, el texto tiene que decir qué
decide ahora, no solo qué mide** — y la fila de §6.2 lo marca como «vuelve con OTRO sujeto».

**El ricochet de §6.1 se repitió, y esta vez fuera de Jubilación.** Cuatro entradas que nadie habría
listado como «de jubilación» quedaron falsas por citar el objetivo: `settings.swr` («convierte tu
gasto anual en el objetivo FIRE»), `settings.inflation` («el objetivo FIRE crece»),
`settings.taxable_gain` («gobierna… tu objetivo»), `settings.savings_source` («el objetivo FIRE
sigue el modo») y `upcoming.net` («nunca tu objetivo de jubilación»). Se corrigieron en la misma
pasada. `settings.swr` además **cambia de título** —«Tasa segura de retirada (SWR)» → «Tasa de
retirada (SWR)»— porque ya no es una tasa de perpetuidad sino el tope de lo que puedes sacar el
PRIMER año, la puerta que tu fecha tiene que pasar: es un renombrado de métrica visible (§3), no una
mejora de estilo.

**Y una guarda nueva en el test** (`helpTexts.test.ts`): ninguna ayuda puede volver a mencionar
«patrimonio objetivo», «objetivo FIRE», «base del objetivo», «cruce con/del objetivo», «descuento
del puente» ni «colchón». Prohíbe FRASES, no la palabra «objetivo» —«edad de jubilación objetivo»
sigue siendo un campo vivo—, y existe porque el vocabulario retirado sobrevive en la cabeza de quien
escribe la siguiente entrada. El precedente exacto es el issue #216: una ayuda que prometía una
línea de mediana que el chart llevaba semanas sin dibujar.

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

**Re-sincronizado el 2026-09-06 con el MODELO v2 de jubilación (WP W8, rama `release/5.0.0`)**: se
retiran **diez** entradas de Jubilación y nacen **ocho** (listas completas en §6.4); se reescriben
sin cambiar de id `retirement.success`, `retirement.bands` (deja de prometer una línea de mediana —
issue #216), `retirement.coverage` (el numerador no cuenta el exceso), `retirement.required_contribution`,
`retirement.coast_month`, `retirement.pension`, `retirement.withdrawal_rule`, `retirement.spend_mode`,
`retirement.strategy`, `retirement.target_age`, `retirement.plan_sentence`, `retirement.partial`,
`assets.expected_return` y `assets.volatility` (CAGR y su hermana), `summary.plan` y
`summary.success`; y se corrigen por ricochet cuatro entradas de fuera de Jubilación que citaban el
objetivo (`settings.swr` —además **retitulada**—, `settings.inflation`, `settings.taxable_gain`,
`settings.savings_source`, `upcoming.net`). §6 pierde el recuento congelado y gana los comandos;
las filas del mapa de vistas y la tabla de una línea de §6.2 quedan al estado vigente; §6.3 se
actualiza a la composición nueva de las tres tarjetas; nueva §6.4 y una guarda nueva en
`helpTexts.test.ts` (ninguna ayuda vuelve a mencionar el objetivo, su base ni el colchón).
Verificación: `npx vitest run src/lib/helpTexts.test.ts src/lib/retirement-form.test.ts
src/lib/risk-bands.test.ts` en `apps/web` y `grep -rn 'as unknown as HelpTextId\|PendingHelpTextId'
apps/web/src` (los andamios temporales de W2/W6 se retiraron con este paquete).

**Re-sincronizado el 2026-09-05 tras la TERCERA vuelta de UX de Jubilación (V1–V7, feedback F2 y
F5–F10 del owner, mismo issue #207)**: el catálogo BAJA de 55 a **53** con dos retiradas —
`retirement.assumptions` (V3: se fue con el acordeón «Avanzado» que resumía) y
`retirement.success_threshold` (V7: el umbral configurable dejó de existir; el corte es fijo, verde
solo al 100 %)—; `retirement.cash_buffer` **reescrita entera** (V6: el colchón se DERIVA del tope de
la regla de ahorro, y el texto declara procedencia, salida y el signo del efecto sin matizar);
`retirement.depletion_by_age` reescrita (su superficie principal es ahora el COLOR de la banda, más
la fila `depletion_total` del detalle); `retirement.bands` gana una frase sobre ese color;
`retirement.success` y `summary.success` pasan del umbral configurable al corte fijo; y
`retirement.target` reescrita por WP-E (dos tarjetas, dos campos, base declarada en el subtítulo).
Las dos filas del mapa de vistas se actualizan en §6.

**Esta transición (umbral configurable → corte fijo) se DESHIZO al día siguiente y en dirección
CONTRARIA a como suena leída sola**: el modelo v2 (C3, 2026-09-06 — §6.4 arriba, que es la entrada
vigente) retira el corte fijo al 100 % y devuelve `success_threshold_pct` al perfil como la
restricción que decide la fecha y el semáforo (80–100, default 95, evaluado sobre el suelo de
Wilson). Quien lea solo este párrafo sin subir a §6.4 se queda con la dirección de V7, que hoy es
historia, no el estado del catálogo.

Introducido en 3.9.0 junto al popover de ayuda. **Ampliado el 2026-09-03 (5.0.0, issue #207)**:
§5 (el tercer patrón del escáner de cobertura, la forma `helpId:` de objeto), §6 (recuento por
comando: 52; el mapa de vistas gana Jubilación·plan, Jubilación·KPIs, Jubilación·Riesgo y Activos) y
§6.2 (las 23 entradas nuevas, la única edición —`retirement.target` subordinado a la estrategia—, la
norma de las dos bases y las dos entradas que documentan un resultado incómodo).

**Ampliado el 2026-09-03 en el pase de documentación U5b** (tras aterrizar `debc52d`, rediseño UX
U1b de Jubilación, issue #207): el catálogo sube de 53 a 55 con `retirement.plan_sentence` y
`retirement.assumptions` (§6, fila nueva «Jubilación · frase-hito y supuestos»); nueva §6.3 —la
cabecera de resultados es un TOPE de 3 tarjetas con orden de prioridad fijo, no un catálogo nuevo:
las seis entradas de §6.2 que alimentan tiles no cambiaron de texto, solo de cuántas se enseñan a
la vez y adónde va lo que no cabe (`retirementDetailRows`)—; y `pct_source` documentado como campo
declarativo (no entrada del catálogo, mismo criterio que §7 aplica a `basis`).

**Re-sincronizado el 2026-09-03 tras el pase de correcciones de la revisión adversarial** (commit
`0668f37` del motor + su seguimiento en `apps/web`, issue #207 cerrado): el catálogo sube de 52 a
**53** con la entrada nueva `retirement.coverage` (añadida a Jubilación·Riesgo); `retirement.success`
y `summary.success` pasan a exigir las DOS condiciones (jubilarse dentro del horizonte y no agotar,
nunca solo «no agotar»); `retirement.cash_buffer` gana el hallazgo corregido (protege +3,9 pp con el
colchón a la rentabilidad de la cartera, cuesta neto −3,5 pp con una cuenta al 0 %); y los dos
marcadores `<!-- MC: revisar tras el pase de correcciones -->` se resuelven y se retiran. **Re-verificado y
ampliado el 2026-08-22 (4.0.0)**:
§6 (estado del catálogo, `retirement.target`) y §6.1 (las cuatro derivas de la auditoría previa a la
publicación, ya corregidas en `helpTexts.ts`). **Ampliado el 2026-08-28 (Fase 5 del tren 4.4.0,
issue #86)**: §7 — decisión razonada de NO dar entrada a `financial_health.basis`/`totals.basis`
(sin consumidor en la SPA hoy) y la regla permanente de las cuatro cuartetas homónimas
`get_budget.totals` ↔ `get_summary.financial_health`. El catálogo en sí (§6) no cambió: sigue en
16 entradas — este pase fue sobre campos que deliberadamente NO entraron. Re-verificación:

```bash
# Entradas del catálogo y consumidores
grep -c '^  "' apps/web/src/lib/helpTexts.ts        # 53 el 2026-09-05 tras la 3.ª vuelta de UX (55 tras U1b;
  # 53 antes; 52 tras el pase de correcciones; 29 en main; 16 a 2026-08-25). OJO: el catálogo ha
  # pasado por 53 DOS veces con listas distintas — el número no identifica un estado, solo delata
  # que algo se movió. La lista (`grep -n`) es la que hay que mirar.
grep -cE '^    title: "' apps/web/src/lib/helpTexts.ts   # mismo número: contraste de indentación
grep -rn 'helpId=' apps/web/src --include='*.tsx' | wc -l   # 23 el 2026-09-05 — consumidores en JSX
grep -rn 'helpId:' apps/web/src --include='*.ts' | wc -l    # 38 el 2026-09-05 (40 tras U1b). El «12» que
  # este comando citaba ya estaba obsoleto ANTES de U1b: en el commit padre (`055a185`) daba 20
  # (`retirement-tiles.ts` 15 + `risk-bands.ts` 3 + `helpTexts.test.ts` 2), no 12 — nadie lo
  # re-verificó al escribirlo. U1b sube a 40 al añadir `lib/retirement-form.ts` con su tabla
  # PLAN_FIELD_HELP (20 hits, incluida la anotación de tipo). Consumidores en forma de OBJETO
  # (5.0.0). Repártelo por fichero si necesitas auditar uno:
  # `for f in helpTexts.test.ts retirement-form.ts retirement-tiles.ts risk-bands.ts; do grep -c 'helpId:' apps/web/src/lib/$f; done`
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
