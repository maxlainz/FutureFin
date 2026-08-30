<!--
Las puertas viven en CONTRIBUTING.md §3 (dueño único desde la consolidación 2026-08-30) — esta
plantilla solo las marca, no las repite. Pega la salida real de los comandos: «lo he probado» no
es evidencia.
-->

## Qué cambia y por qué

<!-- El problema primero, la solución después. Si es un fix: síntoma → causa raíz → arreglo. -->

Closes #

## Evidencia

<!--
Qué demuestra que esto hace lo que dice:
- fix → el test que falla con el código viejo y pasa con el nuevo (nómbralo).
- refactor sin cambio de salida → la captura de regresión, con el valor de antes.
- cambio de modelo o de cifras → el número esperado, calculado ANTES de ejecutar, y el obtenido.
- cambio visual → dilo explícitamente: verificado en claro y en oscuro.
-->

## Puertas

- [ ] `./scripts/test-all.sh` en verde (o las puertas individuales de
      [CONTRIBUTING §3](../CONTRIBUTING.md#3-las-puertas--ejecútalas-antes-de-proponer-el-cambio),
      con la salida pegada). CI las repite, pero lo que CI **no** ve sigue siendo tuyo: la
      interfaz en claro **y** oscuro, y los drills de Docker previos a un release.
- [ ] Las puertas **por clase de cambio** que apliquen (motor, contrato de API + paridad MCP,
      migración, visual, métrica/KPI, contenedor) — la tabla vive en CONTRIBUTING §3 y en
      `futurefin-change-control` §1.
- [ ] Documento de record del área actualizado, y entrada en `CHANGELOG.md` bajo `[Unreleased]`
      si el cambio es observable (estilo forense; *breaking* marcado explícito).
- [ ] Ni el diff, ni los tests, ni las capturas llevan datos reales de nadie;
      `./scripts/scan-sensitive.sh` sale limpio.
