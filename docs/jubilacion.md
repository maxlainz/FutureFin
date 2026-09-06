# Tu plan de jubilación

Desde la **5.0.0**, la jubilación de FutureFin no es un objetivo que tu patrimonio persigue: es un
plan que **el éxito decide**. Cada persona de la instalación elige su estrategia, su edad, su
pensión y cómo piensa sacar el dinero — y la simulación resuelve, sorteando miles de futuros
posibles, la primera fecha en la que ese plan aguanta sin que tengas que volver a trabajar.

Todo vive en la pestaña **Jubilación**, es tuyo y no del hogar, y se guarda solo: cambias un campo,
esperas un segundo y la frase, las tarjetas, el gráfico y el riesgo se recalculan.

> **Modelo v2 (5.0.0).** Esta página describe el modelo tal y como quedó tras la revisión
> adversarial de septiembre de 2026: «el éxito define la fecha», no un capital estático que tu
> patrimonio cruza. Si buscas algo que esta página describía antes de esa revisión, el apéndice
> [Qué desapareció en la 5.0.0](#apéndice--qué-desapareció-en-la-500-y-por-qué) dice qué cambió y
> por qué.

Si acabas de crear tu cuenta, el asistente de bienvenida te pide lo mínimo para tener un plan: tu
fecha de nacimiento, tu estrategia y el dato esencial que esa estrategia y su modo necesitan (una
edad objetivo, la edad en que dejas de aportar, o el ingreso de la media jornada). Tu tasa de
retirada se queda en su valor por defecto (3,5 %) hasta que la ajustes aquí, en la tarjeta
«Retirada»; la inflación anual, en el suyo (2,5 %), vive en `Ajustes → Plan` porque es del hogar,
no de una persona.

---

## Qué decide la fecha

Tu fecha de jubilación **no es un cruce de tu patrimonio contra un objetivo**: es el primer mes en
el que, jubilándote ahí, un número suficiente de futuros posibles aguanta hasta el final de tu
horizonte sin que tengas que volver a trabajar.

FutureFin sortea miles de futuros del mismo plan —Monte Carlo: cada mes, tus inversiones suben o
bajan según la volatilidad que hayas declarado— y cuenta cuántos aguantan. Esa fracción es tu
**éxito**. Tu **umbral de éxito** es la parte que exiges que aguante —de serie, el 95 %, ajustable
entre el 80 % y el 100 %— y tu **fecha válida** es el primer mes en que el éxito llega a ese umbral.

Dos matices que cambian cómo se lee la cifra:

- **No se compara con el porcentaje que ves, sino con su cota inferior.** Con un sorteo de miles de
  caminos, un 95,0 % observado puede ser en realidad un 93,8 % de la población entera; FutureFin
  compara tu umbral contra el **límite inferior del intervalo de confianza de Wilson**, que es
  estable frente a la semilla y al número de caminos. Es más prudente que comparar contra el
  número grande, y es la razón por la que dos sorteos con la misma semilla siempre coinciden en
  qué fecha es válida.
- **Al 100 % la exigencia es «ningún escenario falla de los sorteados»**, y FutureFin publica hasta
  dónde puede llegar el riesgo que no ves: con 2.500 caminos y cero fallos, el riesgo real puede
  llegar al 0,12 % (la cota de la regla de tres). No es un 100 % certificado; es un 100 % muestral
  con su margen declarado.

Al lado de tu fecha ves siempre otras dos: **la fecha al 100 %** y **la fecha al 90 %**, que acotan
la tuya por arriba y por abajo — «al 100 % sería ocho años más tarde; al 90 %, tres antes». Y si
ningún mes de tu horizonte llega a tu umbral, la respuesta es **«nunca»**: no un hueco, no un cero,
un resultado.

---

## Qué cuenta como fallar

Un futuro del sorteo («un escenario») falla —tiene que volver a trabajar— por una de tres vías, y
solo una a la vez:

| | Cuándo | Ejemplo |
|---|---|---|
| **F1 — sin dinero** | En algún mes de jubilación (total o de media jornada) la cartera no cubre lo que necesitas ese mes. | El líquido se agota en un año malo. |
| **F2 — tasa inicial excesiva** | En el mes en que te jubilas del todo, lo que necesitas para el año que empieza supera el tope vigente sobre tu líquido: tu tasa de retirada, o la tasa del puente si lo tienes activado y tu pensión llega dentro de los años máximos que fijaste. | Te jubilas necesitando sacar el 6 % el primer año con una tasa del 4 %. |
| **F3 — la regla no cubre el gasto** | Con una regla que retira un % del saldo («Un % del saldo», «Híbrida» o «Con bandas»), lo que la regla te deja sacar ese mes no llega a tu gasto ordinario. | Un año malo con «Un % del saldo» recorta más de lo que tu gasto exige. |

La prioridad, dentro de un mismo mes, es **F1 > F2 > F3**: si te quedas sin dinero no importa si
además te pasaste de tasa. Con la regla de serie, «Gasto fijo», solo pueden ocurrir F1 y F2, porque
esa regla no tiene techo que recortar. **Durante la media jornada solo actúa F1**: la puerta de
tasa inicial es de la jubilación total, no de la fase parcial.

Tu **necesidad ordinaria** del mes —la que F1 y F3 comparan— es tu gasto más cualquier retirada
extra, menos tus ingresos (pensión incluida): **sin deuda y sin «Próximos»**. Esas dos partidas
mueven tu caja y, con ella, tu fecha, pero no son gasto ordinario — igual que la cuota de un
préstamo no entra en lo que tu tasa de retirada tiene que cubrir.

Y lo que **no** cuenta como fallo: que una regla por saldo te recorte el gasto en un año malo **y
aun así te deje cubierto**. Bajar de nivel de vida no es fallar; quedarte corto, sí.

---

## Las cuatro estrategias

La tarjeta «Estrategia» tiene cuatro opciones. Cambiarla cambia qué te pregunta el resto de «Tu
plan» y cómo se resuelve tu fecha.

### Cuanto antes (FIRE clásico)

> Ahorras todo lo que puedes y te jubilas en la primera fecha en la que tu plan aguanta hasta el
> final con el nivel de éxito que exijas.

No necesita ni edad objetivo ni pensión: solo tu fecha de nacimiento, para poder publicar fecha,
éxito y capital. Publica tu fecha válida, el éxito con el que llega, las fechas al 100 % y al 90 %,
y —de lectura— cuánto se apretó el cinturón la mediana de los escenarios que aguantan.

### A una edad fija

> Eliges la edad y el plan te dice cuánto tienes que aportar cada mes para llegar a ella con tu
> nivel de éxito.

Aquí **manda la edad**: te jubilas ese mes, aguante o no. Publica dos cosas más:

- **Un veredicto** — cuántos escenarios de cada 100 aguantan jubilándote a esa edad, contra tu
  umbral. Si ya llegas de sobra, el aviso dice hasta qué edad podrías haberte jubilado.
- **Una aportación mínima** — la menor aportación extra, **plana y nominal** (los mismos euros cada
  mes, sin subirla con la inflación), que hace cumplir tu umbral a esa edad. No sale de una
  fórmula: se prueba una cantidad, se sortea el plan entero con ella dentro —tu cascada de ahorro,
  tus topes, tus impuestos— y se ajusta hasta dar con la más pequeña que llega. Es un TECHO sobre
  lo que tu reparto invierte, no un importe fijo que se aporte pase lo que pase: un mes con menos
  sobrante aporta lo que hay. Si ni invirtiendo cada euro de tu sobrante llegas, la tarjeta lo dice
  con todas las letras —«Ni ahorrándolo todo»— en vez de fingir una cifra.

### Ahorrar ahora y dejar crecer (Coast FIRE)

> Aportas fuerte y luego dejas de aportar. Puedes fijar la edad de jubilación —y el plan te dice
> cuándo puedes dejar de ahorrar— o fijar la edad a la que dejas de ahorrar y ver a qué fecha te
> lleva.

Tiene dos modos, y solo contestas uno: el otro lo resuelve el plan.

- **Modo A — fijo la edad de jubilación.** Se busca el **primer** mes en que, dejando de aportar
  ahí, tu éxito a esa edad sigue cumpliendo tu umbral.
- **Modo B — fijo la edad en que dejo de aportar.** Tu fecha válida sale de ahí, sea la que sea.

En los dos modos, el plan que ves **deja de aportar de verdad** desde ese mes: la línea, la banda y
los escenarios corren sin aportaciones a partir de ahí, y lo que dejas de ahorrar es dinero
disponible para gastar — no vuelve a la cartera. Si no existe tal mes (ni aportando siempre llegas)
o, en modo B, parar ahí no llega a tu umbral, el aviso lo dice sin rodeos: no puedes dejar de
aportar y llegar.

### Jornada reducida (Barista FIRE)

> Bajas de jornada a una edad —o en cuanto el plan pueda permitírsela— y cubres el hueco con tu
> capital hasta la jubilación total.

En el selector de estrategias se llama **«Jornada reducida (Barista FIRE)»**; sus campos (más
abajo) siguen llamándose «media jornada» — es la misma fase con dos nombres. Declaras el
**ingreso mensual** de la fase (en euros de hoy, **plano**: no sube con la inflación — un 0 € es un
año sabático) y con qué gasto se compara. Dos modos:

- **Modo A — a una edad S.** Tu jubilación total es la fecha válida calculada **con la fase ya
  dentro** desde S.
- **Modo B — en cuanto pueda.** El primer mes S en que la fase no falla (solo mira F1: sin dinero),
  y desde ahí se resuelve tu fecha válida. Si no hay ninguna, el aviso es: pasas a la fase, pero no
  llegas a jubilarte del todo dentro del horizonte.

**No es la jubilación parcial legal de la Seguridad Social**: aquí no hay cotización, ni
porcentajes legales, ni contrato de relevo — solo cuánto ingresas y desde cuándo.

### El puente hasta la pensión, como ajuste — ya no como estrategia

Si declaras una pensión con fecha, en la tarjeta **Pensión** aparece un interruptor: **el puente**.
Apagado por defecto, y disponible con **cualquiera** de las cuatro estrategias de arriba.

Actívalo y cambia UNA cosa: si al jubilarte falta menos que los años máximos que fijes para que
entre tu pensión, el tope de lo que puedes sacar el primer año pasa de tu tasa de retirada a la
**tasa del puente** —más alta— mientras dure ese tramo. Es jubilación anticipada de verdad: sin
sueldo, sin aportaciones, el gasto sale entero de tu capital hasta que llega la pensión. No hay
ningún otro tope durante el puente: si aguanta o no lo deciden las mismas reglas que el resto del
plan (F1 y F3). Al activarlo, FutureFin rellena sus dos números con un default —la tasa, `max(5,
tu tasa de retirada + 1)` %; los años, 7— y puedes cambiarlos (la tasa hasta el 20 %, los años
hasta 20).

---

## Las tarjetas y sus campos

«Tu plan» tiene **seis tarjetas** y, entre todas, **25 campos posibles** — pero nunca ves los 25 a
la vez: solo aparece el campo que tu estrategia y su modo necesitan. Un campo que no ves es un
campo que la simulación no está mirando; una tarjeta que se quedaría vacía tampoco se pinta.

| Campo | Tarjeta | ¿Cuándo se ve? | ¿Obligatorio? |
|---|---|---|---|
| Fecha de nacimiento | Edades | Solo si te falta | Sí, con pensión declarada o en «A una edad», «Coast» o «Media jornada» |
| Qué fijas tú | Edades | Estrategia Coast | No (trae default) |
| Edad de jubilación objetivo (o «edad de jubilación total» en Media jornada) | Edades | «A una edad»; Coast modo A; Media jornada | Sí, salvo en Media jornada |
| Edad en que dejo de aportar | Edades | Coast modo B | Sí |
| Cuándo empieza la media jornada | Edades | Media jornada | No (trae default) |
| Edad de inicio de la media jornada | Edades | Media jornada, modo A | Sí |
| Ingreso mensual en media jornada | Edades | Media jornada | Sí |
| Gasto durante la media jornada | Edades | Media jornada | No |
| Pensión mensual / Edad de inicio de la pensión | Pensión | Siempre visible (la casilla vive ahí) | Sí, en cuanto declaras pensión |
| Pensión indexada a la inflación | Pensión | Con pensión declarada | No |
| Pensión cobrada durante la media jornada | Pensión | Media jornada y pensión | No |
| Puente hasta la pensión (el interruptor) | Pensión | Con pensión declarada, cualquier estrategia | No |
| Tasa máxima / Años máximos del puente | Pensión | Con pensión **y** puente activado | Sí |
| Cómo se calcula el gasto de jubilación | Gasto en jubilación | Siempre | No |
| Gasto anual manual | Gasto en jubilación | Modo «Manual» | Sí |
| Umbral de éxito | Retirada | Siempre, el primero | No (default 95) |
| Tasa de retirada | Retirada | Siempre | No (default 3,5 %) |
| Regla de retirada | Retirada | Siempre | No |
| Baja al… | Retirada | Regla «Híbrida» | No |
| Banda / Ajuste | Retirada | Regla «Con bandas» | No |
| Cómo se aplica la regla | Retirada | Regla ≠ «Gasto fijo» | No |
| Edad límite del horizonte | Horizonte | Siempre | No (default 90) |

La **fecha de nacimiento es obligatoria** por dos motivos, no solo uno: las estrategias por edad la
necesitan para tener un mes contra el que resolver nada, y **cualquier pensión declarada
también** — la pensión entra al bucle a una edad, y sin fecha de nacimiento no se sabe si ya se
cobra en tu fecha válida. Sin ella, el panel de resultado no publica ni fecha, ni éxito, ni
capital: lo dice, no lo inventa.

La **volatilidad** de tus activos no es un campo de esta tabla —vive en Activos, junto a la
rentabilidad esperada— pero condiciona todo lo de arriba: sin volatilidad declarada en ningún
activo, el sorteo no dispersa y el éxito sale 0 % o 100 % por construcción. FutureFin te avisa en
vez de dejarte leer un 100 % que no mide nada.

---

## La pensión, como un flujo más

En la 4.x, una pensión se apuntaba como un ingreso «que persiste tras la jubilación» y se restaba
desde el primer mes de jubilación, aunque fuera a tardar veinte años en llegar. Desde el modelo v2,
**la pensión es un flujo de caja como cualquier otro, con fecha**:

- Entra en el bucle como un ingreso más, el mes exacto en que empieza a cobrarse (tu edad de
  inicio), indexada a la inflación por defecto o plana si lo desactivas.
- Si sobra, tus reglas de ahorro reparten el sobrante — la cascada corre igual jubilado o no
  (4.12.1). Si falta, se vende cartera.
- **No hay ningún «objetivo» que la pensión dimensione por adelantado.** FutureFin no asume que te
  vaya a cubrir ni te la descuenta de nada: simplemente la cuenta cuando llega, mes a mes, como el
  resto de tus ingresos.
- Durante la media jornada puedes declarar qué **parte** de esa pensión ya cobras en esa fase
  (0–100 %, por defecto 0 %). Si tu pensión empieza durante la fase y no has declarado esa parte,
  un aviso (`pension_unpaid_during_partial`) te recuerda que el plan no la cobra entera hasta la
  jubilación total.

---

## El gasto en jubilación

Cuánto gasta tu plan cada mes jubilado lo fija uno de tres modos, en la tarjeta «Gasto en
jubilación» — y ese gasto (más las retiradas extra, menos tus ingresos y tu pensión) es exactamente
la necesidad ordinaria que F1 y F3 comparan:

- **Gasto actual** — tus partidas de jubilación del presupuesto.
- **Ingresos actuales** — para mantener tu nivel de vida.
- **Manual** — una cifra anual que decides tú.

Junto al selector verás la cifra mensual/anual derivada, con su procedencia, para que dos planes
con el mismo número no crean estar mirando lo mismo si uno lee su presupuesto y otro sus ingresos
reales.

Aparte, y solo como lectura, está el **número FIRE clásico**: tu gasto anual de jubilación dividido
entre tu tasa de retirada —25 veces el gasto solo si tu tasa es el 4 % clásico; con cualquier otra
tasa, otro múltiplo— sin restar la pensión que declares. No decide nada: tu fecha y tu capital
salen de contar escenarios, no de esta división, y por eso casi nunca coinciden. Vive en «Detalle
del cálculo».

---

## El capital que necesitas

FutureFin publica el capital necesario de dos formas, y **no son la misma cifra escalada**: cada
una contesta una pregunta distinta.

- **Capital necesario hoy** — el patrimonio LÍQUIDO que haría falta **ahora mismo**, invertido con
  tu misma mezcla de activos (se escala el valor y el precio de compra de tus activos líquidos; tu
  vivienda no cuenta), para que jubilándote hoy aguanten tantos escenarios de cada 100 como pida tu
  umbral. Sale de una bisección sobre un factor de escala de tu cartera, con 500 caminos para
  buscar y 2.500 para confirmar. Siempre en **euros de hoy** y redondeado a **cientos hacia
  arriba** —sale de un sorteo, y el euro exacto fingiría una precisión que no hay—. Es la misma
  cifra en Jubilación, en el Resumen y en la Proyección.
- **La curva del gráfico** — la misma pregunta, contestada para cada edad de tu horizonte: «si te
  jubilaras a esta edad, ¿cuánto líquido haría falta?». Viaja en euros **nominales** —los mismos
  que tu línea de patrimonio—, así que el interruptor «En dinero de hoy» la deflacta igual que a
  ella, y las dos siguen siendo comparables punto a punto. Internamente se resuelve solo en unos
  pocos puntos —cada cinco años, más tu fecha exacta— y se interpola para que encaje mes a mes con
  tu patrimonio en el gráfico; por eso puede calcularse en segundo plano sin bloquear la pantalla.

La curva **no tiene por qué cruzar tu línea de patrimonio**, y que no la cruce no es un error del
dibujo: tu fecha la deciden los escenarios que aguantan, no un cruce de dos líneas. Lo único que
marca tu fecha válida en el gráfico es una **marca vertical**, con el éxito con el que llega escrito
al lado.

---

## La pantalla

### La frase-hito

El resultado de tu plan abre con una frase, no con una tarjeta suelta. Cambia con tu estrategia y
con el estado del cálculo (cifras de ejemplo, inventadas):

| Estrategia / estado | Frase |
|---|---|
| Cuanto antes | «Con tu plan te jubilas en 2043 (a los 55): aguantan 95 de cada 100 escenarios. Al 100 % sería 2051; al 90 %, 2040.» |
| A una edad, ya llegas | «A los 55, como pediste: aguantan 82 de cada 100 escenarios (tu umbral es 95). Para llegar harían falta 300 € más al mes.» |
| Coast, modo A | «Puedes dejar de aportar en 2031 (a los 41) y jubilarte a los 55 con 95 de cada 100.» |
| Coast, modo B | «Dejando de aportar a los 41, te jubilas en 2047 (a los 57) con 95 de cada 100.» |
| Media jornada | «Puedes pasar a jornada reducida en 2031 (a los 41) y jubilarte del todo en 2045 (a los 55) con 95 de cada 100.» |

Y tres estados ganan a la estrategia, en este orden: que falte un dato imprescindible —empezando
por la fecha de nacimiento—, que el cálculo siga en marcha («Calculando tu fecha…») y que ningún
mes del horizonte llegue a tu umbral («no hay ninguna fecha… Lo más cerca: 2058 con 62 de cada
100»).

### Las tres tarjetas

Como mucho tres, nunca más, una cifra por tarjeta:

1. **Capital necesario hoy** — siempre.
2. **Éxito del plan** — siempre, con tu umbral en el subtítulo.
3. **La de tu estrategia** — «Fecha válida» (Cuanto antes), «Aportación mínima» (A una edad),
   «Mes coast» (Coast), «Inicio de la jornada reducida» (Media jornada). Si el plan no está
   resuelto, no hay tercera tarjeta: repetir la misma razón tres veces no ayuda.

### El gráfico

Un único gráfico, con un único eje de importes:

- **Una línea**: tu patrimonio, la trayectoria central, creciendo con la rentabilidad **compuesta**
  que declaraste en cada activo.
- **Una banda p10–p90** —ocho de cada diez futuros caen dentro—, teñida por edad con la
  probabilidad ACUMULADA de fallo (F1, F2 o F3 juntos): verde donde no falla ninguno, ámbar en
  cuanto empiezan a fallar, rojo desde el 10 % — o desde el complemento de tu umbral si pides más
  de un 90 % de éxito: con un umbral del 80 %, el rojo empieza en el 20 %. El corte ámbar va a la
  mitad del rojo. Pasa el ratón por encima y ves el porcentaje exacto y, entre paréntesis, cuánto
  es F1, cuánto F2 y cuánto F3 en ese punto.
- **La curva de capital necesario**, discontinua.
- **Una marca vertical** en tu fecha válida, con el éxito con el que llega.
- **Una tira bajo el eje**, con el éxito si te jubilaras en cada año («si te fueras en 2036: 78 de
  cada 100»).

Sin volatilidad declarada en ningún activo, la banda no se colorea: no hay nada que colorear, y
pintarla de verde diría que ningún escenario falla cuando lo que pasa es que no se ha sorteado
ninguno.

### Riesgo

Debajo del gráfico, un bloque compacto con lo que hace **auditable** el «Éxito del plan» de arriba
(el propio KPI vive en la cabecera; no se repite aquí):

- La **precisión** del sorteo: el semiancho del intervalo de Wilson en puntos porcentuales
  («±1,2 pp»), o la cota de la regla de tres con cero fallos; y, en claro, el límite inferior de
  Wilson («con 95 % de confianza, al menos un 93,8 %») — es justo lo que se compara con tu umbral,
  no el número grande de la cabecera.
- Los **fallos por tipo** —cuántos escenarios cayeron por F1, F2 o F3, sobre el total— cuando ha
  fallado alguien; si no ha fallado nadie, las tres filas no se pintan.
- La **cobertura de tu gasto, corregida** — qué parte de tu gasto se pagó de verdad (nunca por
  encima del 100 %) y en cuántos meses te quedaste corto, contando las DOS formas de quedarte
  corto: lo que la regla se negó a sacar y lo que la cartera ya no pudo pagar. Antes esta cifra
  solo miraba lo primero, así que con una regla que no recorta nunca —«Gasto fijo»— siempre daba el
  gasto entero cubierto; hoy también cuenta el descubierto.
- Cuántos escenarios **fallan en algún momento** del horizonte, por cualquiera de los tres
  motivos — el total que el color de la banda no puede rotular en un solo punto.
- El coste, el número de caminos y la semilla del sorteo, para que se pueda auditar y reproducir.

### Detalle del cálculo, plegado

Lo que no cabe en las tres tarjetas de arriba, sin desaparecer: el **número FIRE clásico**, las
**fechas al 100 % y al 90 %**, y la **semilla y los caminos** con los que se resolvió tu plan — la
identidad del resultado, no un metadato.

---

## Por qué la rentabilidad ahora es compuesta (CAGR)

La rentabilidad que declaras en cada activo (Activos → «Rentabilidad anual esperada») siempre fue
la anualizada que tu fondo publica. Lo que cambió con el modelo v2 es **cómo la lee el sorteo**:
antes se trataba como una media aritmética de años sueltos; ahora se lee como lo que de verdad es,
una tasa **compuesta** (CAGR), y el sorteo la convierte por dentro a la media aritmética que
necesita para simular mes a mes.

Para ti, en la práctica:

- **Tu línea no se mueve.** La trayectoria central sigue creciendo exactamente al ritmo que
  declaraste: nada cambia en la cifra que ves como «tu patrimonio».
- **El sorteo es más fiel.** Con activos volátiles, la media aritmética de muchos futuros sale algo
  por encima de la línea central —es matemática, no un error—, y antes esa separación no estaba
  bien calibrada.
- **Tus cifras guardadas no se tocan.** No hay conversión que hacer: es la misma rentabilidad de
  siempre, reinterpretada.

En Activos aparece, una única vez, un aviso que lo resume: «Desde 5.0.0 la rentabilidad que
escribes se lee como compuesta (la anualizada que publica tu fondo). Tus cifras no cambian; el
sorteo es más fiel para los activos volátiles.» Se apaga solo y no vuelve a aparecer.

---

## Cuánto tarda y por qué

Resolver tu plan no es una fórmula: es sortear el mismo plan miles de veces y contar cuántos
aguantan, y eso tiene un coste.

- **Lo que ves al momento** —con un indicador de «Resolviendo tu fecha válida» mientras corre—: tu
  fecha válida, tu capital necesario hoy y el número propio de tu estrategia (aportación mínima,
  mes coast, inicio de la fase). Cada uno se busca con **500 caminos** y se confirma con
  **2.500** — los primeros 500 son los mismos en la búsqueda y en la confirmación, así que no se
  tira trabajo.
- **Lo que llega después, en segundo plano** —sin bloquear la pantalla, con su propio aviso:
  «Calculando el capital necesario por edad…»—: la curva de capital necesario por edad, las
  fechas al 100 % y al 90 %, el fallo acumulado por edad que tiñe la banda, y el éxito por año de
  jubilación de la tira. Se resuelve una sola vez por plan —si dos pantallas piden lo mismo, no se
  duplica el sorteo— y se apoya en los mismos 500/2.500 caminos que el resto.

El sorteo de referencia son **2.500 caminos**, siempre con la misma semilla por persona: la cifra
no baila entre visitas. La vista Hogar no resuelve una fecha propia por miembro: cada persona
resuelve la suya en su vista «Yo».

---

## Lo que el modelo NO hace (todavía)

Siete carencias, declaradas y con su issue, en vez de calladas:

| No hace | Por qué importa | Issue |
|---|---|---|
| El sorteo es i.i.d. lognormal con un shock común, sin bootstrap histórico | Un «100 %» de verdad acotado exigiría muestrear de la historia real, no de una distribución teórica; el 100 % de hoy no converge con más caminos | #217 |
| Los activos con volatilidad se mueven con correlación 1 entre ellos | Diversificar entre dos activos volátiles no reduce el riesgo del sorteo tanto como debería en la realidad | #217 |
| El horizonte se fija por edad, sin ponderar por mortalidad | Un plan que «llega justo» a tu edad límite no dice qué pasa si vives más — y en una pareja de 65 años, que uno de los dos pase de los 95 no es tan raro | #218 |
| La vista Hogar no resuelve fecha ni banda de escenarios por miembro | Cada persona sortea la suya en su vista «Yo»; el hogar solo suma lo determinista | #219 |
| La regla «Con bandas» (Guyton-Klinger) no aplica su regla de inflación | El guardarraíl de subir o bajar la retirada existe; el ajuste por inflación que lo acompaña en la literatura, no | #220 |
| Sin avisos legales sobre pensión anticipada o media jornada | Jubilarte antes de tiempo o reducir jornada tiene consecuencias legales sobre tu pensión pública real que este plan no modela | #221 |
| El excedente en jubilación sigue tus reglas de ahorro sin publicar su coste fiscal | Reinvertir el sobrante tiene un coste en impuestos que el plan no enseña, y hace que los tres modos de gasto acaben pareciendo el mismo «gasto de hoy» por dentro | #227 |

Los impuestos se aplican como en el resto de la app —tramos de IRPF del ahorro, si los tienes
activados—: el modelo v2 no cambió esa parte, y sigue siendo una simplificación conocida del motor.

---

## Apéndice — Qué desapareció en la 5.0.0, y por qué

Si buscas aquí algo que esta página describía antes, es porque el panel adversarial de septiembre
de 2026 lo retiró. Ninguno se fue por descuido: cada uno tenía una razón.

- **El objetivo como disparador de la fecha.** Un capital estático que tu patrimonio cruzaba
  decidía cuándo te jubilabas; ahora decide el éxito —el primer mes en que aguantan los escenarios
  que tu umbral exige—, porque un objetivo que se cruza en un año malo dice que estás bien
  jubilándote justo cuando el sorteo dice lo contrario.
- **La base del objetivo (perpetuidad o puente).** Elegías si tu objetivo FIRE se calculaba como
  una renta perpetua —vivir de él para siempre, ignorando la pensión— o como un puente hasta la
  pensión; sin objetivo que dimensionar, no queda base que elegir.
- **El descuento del puente.** La tasa a la que se descontaban los años del puente —tus líquidos,
  tu tasa de retirada, o sin descontar nada— desapareció con el objetivo que descontaba: el puente
  de hoy es un tope de tasa inicial, no una cifra que traer a valor presente.
- **El colchón de caja derivado de una regla de acumulación propia.** Una reserva en meses de gasto
  que el motor mantenía aparte; hoy la caja es un activo más, y el tope en euros de tu regla de
  ahorro ES tu colchón, sin una segunda cifra que pudiera desincronizarse de la primera.
- **El semáforo fijo al 100 %.** Con la versión anterior el umbral es fijo: el color de «Éxito del
  plan» solo se ponía verde con el 100 % exacto, decidiera el usuario lo que decidiera; hoy el
  umbral es tuyo (80–100, de serie 95) y el color se compara contra él, no contra un listón que a
  nadie servía si pedías un 80 %.
- **«Puente hasta la pensión» como una estrategia más.** El selector tenía cinco estrategias, y
  esta era la quinta, con su propio disparador; hoy son cuatro, y el puente vive como un ajuste de
  la tarjeta Pensión disponible en cualquiera de ellas. Un perfil guardado con la estrategia
  `pension_bridge` migra solo a «Cuanto antes» con el puente activado, con un aviso.
- **Los percentiles del cruce por camino.** Antes se guardaba en qué mes cruzaba el objetivo cada
  camino individual del sorteo; sin objetivo que cruzar, la pregunta deja de tener sentido, y lo
  que queda es el éxito por fecha que ya cuenta esta página.

---

## Ver también

- [Actualizar](actualizar.md#actualizar-a-la-500) — qué pasa al saltar desde la 4.x o desde la
  primera versión de la 5.0.0.
- [Configuración](configuracion.md#la-fuente-del-ahorro-el-ajuste-que-redefine-tu-proyección) — la
  fuente del ahorro, la inflación anual y los tramos de IRPF, que siguen siendo del hogar.
- [Conectar Claude](mcp.md) — desde el chat puedes preguntar «¿y si me jubilo a los 55?» sin
  guardar nada.
