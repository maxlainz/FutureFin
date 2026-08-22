---
name: futurefin-data-hygiene
description: >
  Ningún dato real de ninguna persona entra en el repositorio de FutureFin. Carga esta skill
  ANTES de: añadir o modificar un fixture de test (CSV bancario, `.ffbackup`, JSON de paridad),
  escribir una entrada de CHANGELOG que ilustre un cambio con números, capturar pantallas para
  la documentación, pegar la salida de una instalación en un issue o en un doc, o preparar el
  repositorio para hacerse público. Triggers: "añadir un fixture", "un CSV de ejemplo",
  "exportar mi extracto", "pego mis números", "capturas para el README", "datos de demo",
  "seed", "esto es de mi instalación", "anonimizar", "IBAN", "nómina", "scan-sensitive",
  "secrets-scan", "se ha colado un dato", "hay que reescribir el historial". NO la uses para:
  secretos de despliegue y variables de entorno (futurefin-config-and-flags), el cifrado del
  backup por usuario (futurefin-architecture-contract), ni cómo se ejecutan los tests
  (futurefin-validation-and-qa) — esta skill dice QUÉ puede contener un fixture, no cómo correrlo.
---

# Higiene de datos — el repositorio es público

## 1. La regla

**Ningún dato real de ninguna persona entra en el repositorio.** Ni en el árbol de trabajo, ni en
un fixture, ni en un comentario, ni en el CHANGELOG, ni en una captura. Da igual que sea tuyo: el
repositorio es público y el historial de git es para siempre.

Aplica a: IBAN y números de cuenta o tarjeta · nombres y apellidos de personas · nóminas, alquileres
y saldos concretos de una instalación real · direcciones, barrios y sucursales · referencias de
operación de compras reales · correos electrónicos personales · nombres de host, dominios o rutas
de tu infraestructura.

## 2. El incidente que la creó (agosto de 2026)

Al auditar el repositorio antes de hacerlo público se encontró que
`apps/api/tests/fixtures/n26_junio.csv` y `myinvestor_junio.csv` eran **exportaciones bancarias
reales**, no fixtures fabricados: IBAN español completo, nombre y apellidos de una persona, nómina
al céntimo de dos meses consecutivos, gimnasio con sucursal, calle y barrio, y el perfil completo
de suscripciones. El IBAN estaba en el árbol de **109 commits**.

La cabecera del propio fichero de tests decía «Los CSV son fixtures **anonimizados** de los bancos
reales». Nadie mintió: se anonimizó *algo* —lo bastante para tranquilizar a quien lo escribió— y
nunca se volvió a mirar. Esa es la trampa: «anonimizar» un export real es un trabajo de borrado
sobre datos que siguen ahí; **fabricar** un fixture es un trabajo de construcción sobre datos que
nunca existieron. Solo el segundo tiene un estado final verificable.

El CHANGELOG cargaba la segunda mitad del problema: varias entradas razonaban «sobre una instalación
**real**», con el alquiler, el ingreso mensual y la tasa de ahorro del owner en tablas de antes/después.

## 3. Cómo se fabrica un fixture que sigue valiendo como prueba

Un fixture existe para ejercitar un **comportamiento**, no para parecerse a tu vida. Enumera primero
lo que el test necesita y construye el mínimo que lo cubra:

| El test necesita… | Lo que hay que conservar | Lo que NO hay que copiar |
|---|---|---|
| Autodetección del banco | La cabecera literal del export | Ninguna fila real |
| Parseo de importes | La escala rara (`-26.000000000`), la coma decimal, el separador | El importe concreto |
| Codificación | Acentos y `€` en Windows-1252 | El concepto real que los llevaba |
| Heurística de transferencia | Un par opuesto a ≤3 días, un partner «Cuenta de Ahorro», un token `TRANSFERENCIA` | El destinatario |
| Regla aprendida | Varias filas con el mismo prefijo y sufijo numérico variable | El comercio real |
| Dedup por huella | Dos filas idénticas | — |

Los nombres inventados deben **parecer inventados**: `SUPERMERCADO ALMENDRO 21`,
`TIENDA ONLINE* AB12CD34`, `CAFETERIA LA GLORIETA`. Nada de marcas reales con una letra cambiada:
si se lee como real, la siguiente persona asumirá que lo es y lo tratará como intocable.

Referencias de operación: patrones obvios (`AB12CD34`, `EF56GH78`), nunca códigos copiados.
IBAN: **ninguno**, ni siquiera falso — el parser de N26 no lee esa columna, así que va vacía, y un
IBAN sintético solo consigue disparar el escáner para siempre.

## 4. Números en el CHANGELOG y en la documentación

El CHANGELOG de este proyecto es forense: enseña tablas de antes/después porque así se demuestra
que un cambio de números es el que se quería. Eso se conserva. Lo que cambia es de dónde salen:

- **Nunca** «sobre una instalación real» / «datos reales de mi instalación». Se escribe **«sobre una
  instalación de ejemplo»**.
- Los números son inventados pero **aritméticamente coherentes**: si la tabla dice `540,00 ÷ 6` y
  `540,00 ÷ 3`, los resultados tienen que ser 90 y 180. Un ejemplo que no cuadra vale menos que
  ninguno, porque el lector deja de fiarse de la entrada entera.
- Los ejemplos de tools MCP (`«apunta 23,50 € de cena de ayer»`) son ilustraciones de uso: importes
  redondos e inventados, sin categorías que describan tu vida.

Las capturas de pantalla salen **siempre** de una instalación sembrada con `scripts/seed-demo.sh`,
jamás de la tuya. Una captura del Resumen es tu patrimonio neto en un README público.

## 5. El gate

`scripts/scan-sensitive.sh` recorre los ficheros trackeados buscando IBAN, tarjetas, claves
privadas y tokens de varios proveedores. Corre en CI como job `secrets-scan` y es **bloqueante**.

```bash
./scripts/scan-sensitive.sh          # sale 1 si encuentra algo
./scripts/scan-sensitive.sh --list   # qué patrones aplica
```

Excepciones legítimas: `scripts/sensitive-allowlist.txt`, una regex por línea **con un comentario
encima que explique por qué**. Sin el porqué, la siguiente persona no sabrá si puede quitarla.

El escáner es una red, no un sustituto del criterio: no detecta un alquiler de 700 €/mes ni un
nombre de comercio. Esas las paras tú, al escribir.

## 6. Si algo ya se ha colado

Borrarlo del último commit **no basta**: sigue en el historial y en cada clon.

1. **No lo pushees más** y avisa al owner del repositorio antes de tocar nada.
2. Sustituye el contenido por su versión fabricada y verifica que los tests siguen cubriendo lo mismo.
3. Reescribe el historial con `git filter-repo` (`--invert-paths --path <fichero>` y/o
   `--replace-text` para cadenas sueltas), recrea los tags y `push --force-with-lease`.
4. **Verifica sobre todos los commits**, no sobre el árbol:
   ```bash
   git rev-list --all | xargs -n 50 git grep -l "<cadena>" 2>/dev/null || echo "limpio"
   ```
5. Si el repositorio ya era público, asume que el dato está copiado: además de reescribir, hay que
   **rotar** lo que sea rotable (tokens, contraseñas) y valorar avisar a la persona afectada.

## 7. Procedencia y mantenimiento

- La regla nace del hallazgo de agosto de 2026 descrito en §2, durante la preparación del
  repositorio para 4.0.0.
- Patrones vigentes del escáner: `./scripts/scan-sensitive.sh --list`
- El gate de CI: `grep -n "secrets-scan" -A 8 .github/workflows/ci.yml`
- Los fixtures actuales: `ls apps/api/tests/fixtures/` — los tres CSV son fabricados; comprueba
  que siguen siéndolo con `./scripts/scan-sensitive.sh`.
