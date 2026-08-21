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

## 6. Provenance and maintenance

Introducido en 3.9.0 junto al popover de ayuda. Re-verificación:

```bash
# Entradas del catálogo y consumidores
grep -c '^  "' apps/web/src/lib/helpTexts.ts
grep -rn 'helpId=' apps/web/src --include='*.tsx' | wc -l
# Las dos direcciones de cobertura
npm test --workspace futurefin-web -- helpTexts
```
