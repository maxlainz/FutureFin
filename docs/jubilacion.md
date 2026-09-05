# Tu plan de jubilación

Desde la **5.0.0**, la jubilación de FutureFin no es un ajuste del hogar: es **tuya**. Cada persona
de la instalación elige su estrategia, su edad, su pensión y cómo piensa sacar el dinero cuando deje
de trabajar — y la proyección se calcula con eso.

Todo vive en la pestaña **Jubilación**. Se guarda solo, como el resto de la app: cambias un campo,
esperas un segundo y la curva, las cifras y el riesgo se recalculan.

> **Si vienes de la 4.x**: al actualizar no se te mueve ningún número. Arrancas con la estrategia
> «Cuanto antes», que es exactamente cómo funcionaba FutureFin hasta ahora, y con tu SWR, tu modo de
> objetivo y tu edad límite tal y como los tenías. Lo que cambia es que ahora son tuyos y no del
> hogar. Los detalles del salto están en [Actualizar](actualizar.md#actualizar-a-la-500).

Si acabas de crear tu cuenta, el **asistente de bienvenida** ya te ha preguntado lo mínimo para
tener un plan: tu fecha de nacimiento, con qué estrategia quieres jubilarte y el único dato que esa
estrategia necesita (la edad objetivo; el ingreso de la media jornada; o el importe y la edad de tu
pensión). La inflación y la tasa de retirada se quedan en su valor por defecto (2,5 % / 3,5 %) —
todo lo demás, incluido afinar esos dos valores, se hace aquí, en Jubilación.

---

## Cómo está organizada la pestaña

La pantalla tiene **dos bloques y ningún acordeón**, siempre en este orden:

1. **Cabecera** — el título y un único aviso de guardado («Guardando…» / «Guardado» / «Sin guardar
   · falta un dato» / «No se pudo guardar»). Antes había un pie «Guardado automático.» por panel;
   ahora hay uno solo, y manda el peor estado vivo.
2. **«Tu plan»** — dónde configuras, en **una tarjeta por tema**: Estrategia · Edades · Pensión ·
   Gasto en jubilación · Retirada · Horizonte. Cada tarjeta abre con una frase de qué hace y qué
   implica cambiarla, y trae **solo los campos que tu estrategia necesita** — un campo que no ves es
   un campo que la simulación no está mirando. Una tarjeta que se quedaría vacía tampoco se pinta:
   con «Cuanto antes» y tu fecha de nacimiento ya puesta, «Edades» no aparece.
3. **«Resultado»** — dónde lees lo que da tu plan: una frase con tu hito, como mucho tres cifras, un
   único gráfico —con su eje de importes y la banda de escenarios coloreada por riesgo— y un bloque
   de riesgo compacto. Lo que no cabe en esas tres cifras no desaparece: baja a «Detalle del
   cálculo», plegado al final del panel.

**Ya no hay un acordeón «Avanzado».** Lo hubo, y era un cajón de sastre: repartía trece campos en
seis secciones plegadas, con tres mandos de la pensión a dos pantallas de la casilla «Cuento con una
pensión». Con las tarjetas por tema, todo lo que tu plan asume está **a la vista, junto a lo que
significa** — que es lo que aquel acordeón intentaba compensar con su línea «Supuestos: …», hoy
retirada por innecesaria.

En **Hogar** los bloques anteriores desaparecen: solo ves el aviso de solo lectura y una
frase por persona — ver [El hogar](#el-hogar).

---

## Antes de nada: el control «Yo | Hogar»

Arriba a la derecha, junto al tema, hay un control con dos botones: **Yo** y **Hogar**.

- **Yo** (por defecto) — tus activos, tus deudas, tu presupuesto y **tu** plan de jubilación.
- **Hogar** — la suma de todo el mundo, **en solo lectura**. Sale un aviso bajo la barra superior
  («Vista agregada del hogar · solo lectura») y desaparecen los botones de crear y editar.

El ámbito es **global**: se aplica a todas las pestañas a la vez, no solo a la que estés mirando.

**Esto cambió en la 5.0.0.** Hasta la 4.15 la vista por defecto era el hogar; ahora es la tuya.
Con la jubilación convertida en un plan por persona, enseñar el hogar por defecto mezclaba las filas
de dos personas bajo el plan de una sola.

---

## Las cinco estrategias

La primera tarjeta de **«Tu plan»** es **«Estrategia»**, con cinco opciones. Eliges una y las demás
tarjetas se quedan solo con los campos que esa estrategia necesita: no hay que rellenar lo que no
aplica, y lo que no ves no lo está mirando la simulación (ver
[más abajo](#nada-se-fuerza-sin-que-lo-veas-todo-campo-en-vigor-está-en-su-tarjeta)).

En su propia tarjeta vive siempre **«Gasto en jubilación»**, con tres formas de fijar cuánto vas
a necesitar cada año: **el gasto actual** (tus partidas de jubilación del presupuesto), **tus
ingresos actuales** (para mantener tu nivel de vida) o **una cifra manual**. Elijas la que elijas,
debajo aparece la cifra mensual/anual derivada con su procedencia («1.250 €/mes · 15.000 €/año · de
tus partidas de jubilación del presupuesto»), para que dos hogares con el mismo número no crean
estar mirando lo mismo cuando uno lee su presupuesto y el otro sus ingresos reales.

### Cuanto antes (FIRE clásico)

> Ahorras todo lo que puedes y te jubilas el mes en que tu patrimonio líquido cubre el objetivo.

Es la conducta de siempre. **No necesita nada más**: ni edad, ni pensión. La fecha de jubilación es
el mes del **cruce** — cuando la línea de tu patrimonio líquido alcanza la del objetivo.

### A una edad fija

> Eliges la edad; el plan te dice cuánto necesitas ahorrar y cuánto margen te sobra.

**Necesita** tu edad objetivo y tu **fecha de nacimiento** (en «Tu cuenta»; el propio formulario te
lo pide si falta).

Aquí **manda la edad**, no el capital: te jubilas ese mes llegues o no. Lo que publica, en las
tarjetas de «Resultado» (junto al «Objetivo», que sale siempre primero):

- **Ahorro necesario** — la aportación mensual mínima que te hace llegar al objetivo justo en esa
  edad. No se despeja de una fórmula: se **simula** el plan entero probando aportaciones hasta dar
  con la mínima que llega, con tu cascada de reparto, tus topes, tus deudas y tus impuestos dentro.
  Es un **techo** sobre lo que tu reparto invierte cada mes, no un importe que se aporte pase lo que
  pase: un mes con menos sobrante aporta lo que hay.
- **Margen disponible** — lo que te sobra por encima de eso, al mes y acumulado (en dinero de hoy,
  en el «Detalle del cálculo» plegado).

> El gráfico de Jubilación dibuja tu patrimonio, tu objetivo y los hitos del plan — no la
> trayectoria de «ahorrando exactamente lo necesario». Esa línea de puntos («Capital necesario»)
> sigue existiendo, pero vive en el chart grande de la pestaña **Proyección**, que es donde tiene
> sentido compararla trazo a trazo con tu curva real.

**El estado rojo**: si ni invirtiendo cada euro que te sobra llegas, sale un aviso en rojo —
*«Con tu ahorro actual no llegas a los N años. Te jubilarás igual —manda la edad— pero por debajo de
tu objetivo»*— y la tarjeta «Tu plan» del Resumen también se pone en rojo. No es un error: la
simulación existe, se jubila igual y se publica entera. Lo que te está diciendo es que te jubilas
**por debajo** de tu objetivo.

### Ahorrar ahora y dejar crecer (Coast FIRE)

> Aportas fuerte hasta que el capital llegue solo a tu edad objetivo; después, cada euro es margen.

**Necesita** edad objetivo y fecha de nacimiento, igual que la anterior. Publica además:

- **Mes coast** — el primer mes a partir del cual puedes **dejar de aportar** y aun así llegar al
  objetivo en tu edad. Sale de simular el plan cortando la aportación en cada mes candidato. El
  chart de Jubilación marca ese mes con un hito propio («Coast») sobre el patrimonio.
- **Número coast** — el patrimonio líquido con el que entras en ese mes. Ojo: no es tu objetivo de
  jubilación ni tu patrimonio total (la vivienda no cuenta).

La trayectoria «si dejas de aportar aquí» como línea discontinua, comparable trazo a trazo con tu
curva real, vive en el chart grande de la pestaña **Proyección** — igual que «Capital necesario»
más arriba.

Si no hay mes coast, el aviso lo dice con todas las letras: no es que falte el dato, es que **ni
aportando todos los meses** llegas al objetivo en esa edad.

### Media jornada

> Reduces jornada a una edad y cubres el hueco con el capital hasta el cruce total.

**Necesita** el bloque de media jornada: la edad en la que empieza y el ingreso mensual que tendrás
entonces, en euros de hoy (un **0** es un año sabático). Sustituye a tu ingreso actual desde esa
edad, y la fase **termina** cuando llega la jubilación total — por cruce, o en tu edad objetivo si
la has puesto.

Publica el **hueco de media jornada**: el capital que haría falta, a perpetuidad, para pagar lo que
la media jornada no cubre (tu gasto menos el ingreso parcial menos la parte de pensión que cobres
entonces, con los impuestos por delante, dividido entre tu SWR). Es informativo: no dispara nada.
Al lado se dice si el patrimonio **creció o menguó** durante esa fase. El chart marca el inicio de
la fase con su propio hito («Media jornada»), además del de la jubilación total.

### Puente hasta la pensión

> Te jubilas por cruce y vives del capital hasta que llegue la pensión pública; el objetivo se
> dimensiona con ese puente.

**Necesita** el bloque de pensión. Cambia el **objetivo**, no solo la caja — ver
[La base del objetivo](#la-base-del-objetivo-perpetua-o-puente) más abajo. Entre las tarjetas de
«Resultado» aparece «Puente N→M» (las edades de jubilación y de inicio de la pensión) con cuántos
años dura y la tasa de retirada efectiva durante ese tramo — la misma tarjeta aparece con
**cualquier** estrategia que declare una pensión con fecha, no solo con esta.

---

## La pensión pública, con fecha

En la 4.x una pensión se apuntaba como un ingreso que «persiste tras la jubilación», y la simulación
se la restaba desde el primer mes de jubilación aunque fuera a llegar veinte años más tarde. Desde
la 5.0.0 la pensión tiene **fecha**.

Se declara con dos datos: el **importe mensual en euros de hoy** y la **edad a la que empieza a
cobrarse**. Además:

- **Indexada** (por defecto): crece con la inflación que tengas configurada. Puedes desactivarlo si
  prefieres asumir que se queda plana.
- **Parte que cobras durante la media jornada**: qué porcentaje de esa pensión cobras ya durante la
  fase parcial, si es que cobras alguna. Se declara en **porcentaje (0 a 100 %)**; por defecto, 0.

Lo importante: **su fecha cambia el objetivo**. Los años entre que dejas de trabajar y que empiezas
a cobrarla hay que pagarlos **enteros** con el patrimonio, y eso es justo lo que la cuenta clásica
del «25× tu gasto» esconde.

FutureFin no asume nada sobre si la pensión te cubre o no: lee lo que has declarado frente a lo que
gastas. Si la cubre entera, el objetivo se queda en el puente; si cubre una parte, queda además la
renta perpetua sobre lo que falte.

---

## Las reglas de retirada

Ya jubilado, ¿cuánto sale del patrimonio cada mes? Hasta la 4.15 había una sola respuesta: tu gasto
declarado, indexado, sin techo. Sigue siendo la de serie, pero ahora hay cuatro:

| Regla | Qué hace |
|---|---|
| **Gasto fijo en euros de hoy** (por defecto) | Retiras tu necesidad declarada, indexada con la inflación, sin tope. Es exactamente la conducta de la 4.x |
| **Un % del saldo cada año** | Retiras un porcentaje de lo que te quede: en un año malo sacas menos, y el dinero técnicamente nunca se acaba — pero puede quedarse muy corto |
| **Híbrida (empiezo alto y bajo)** | Empiezas con un porcentaje y bajas a otro más prudente cuando la cifra se sostiene |
| **Con bandas (Guyton-Klinger)** | Retiras un importe indexado y lo ajustas arriba o abajo cuando la tasa efectiva se sale de su banda |

**Los porcentajes son BRUTOS de impuestos.** Dicho en claro: si pones un 4 %, ese 4 % es lo que
**sale de la cartera**, y el impuesto de la venta va incluido dentro — no encima. Lo que te llega
al bolsillo es algo menos. Es la misma convención que ya usaba el SWR, y por eso las dos cifras se
pueden comparar.

**En pantalla solo hay UN porcentaje de retirada**: el que fijas en la tarjeta «Retirada» (tu tasa
de retirada segura, la misma que dimensiona el objetivo). «Un % del saldo» y la primera mitad de
«Híbrida» retiran exactamente ese porcentaje — no hay un segundo campo que puedas desincronizar del
primero. Mover el deslizador mueve las dos cosas a la vez: el objetivo y lo que la regla retira.
Debajo del selector de regla verás una nota con el porcentaje efectivo («Retira el 3,5 %: tu tasa
de retirada»); si en algún momento fijaste ese porcentaje **por API o por el conector MCP** en vez
de por aquí, la nota lo dice explícitamente («Regla al 4,0 %, fijado por API») para que sepas por
qué mover el deslizador no cambia lo que se retira.

**Y cada regla se puede leer de dos maneras**, con el selector «Cómo se aplica la regla»:

- **Techo: retiro como mucho la regla.** Sacas lo que necesitas, y nunca más de lo que la regla
  permite. Si necesitas menos, sacas menos.
- **La regla es mi gasto.** Sacas lo que la regla dice, necesites o no. Un año bueno significa
  gastar más.

Elegir uno u otro **no mueve el objetivo**: el objetivo lo dimensiona el SWR, la regla gobierna lo
que sale una vez jubilado. Son dos cosas distintas y conviene no confundirlas.

---

## La base del objetivo: perpetua o puente

En «Objetivo anual» eliges sobre qué se dimensiona el objetivo:

- **Renta perpetua** — el capital para vivir de él para siempre, ignorando la pensión. Es la cuenta
  clásica y la más conservadora.
- **Puente hasta la pensión** — el capital para llegar hasta la pensión, **más** la renta perpetua
  sobre lo que la pensión no cubra.

**Si no eliges**, FutureFin usa el puente cuando hay pensión declarada y la renta perpetua cuando no
la hay. Y si eliges «Puente hasta la pensión» como estrategia, el puente se aplica siempre.

La diferencia no es pequeña. Con un ejemplo de laboratorio —gasto de 2.000 €/mes, SWR del 4 %, sin
impuestos, pensión de 1.200 €/mes a los 67, cartera líquida al 5 %— el objetivo pasa de **600.000 €**
con renta perpetua a **≈435.300 €** con puente: 380.700 € del puente descontado más 54.600 € de la
perpetuidad sobre los 800 €/mes que la pensión no cubre. *(Cifras inventadas para el ejemplo, no de
ninguna instalación real.)*

### El descuento del puente

Los años del puente están en el futuro, así que no hace falta tener hoy el dinero entero: lo que
tengas seguirá creciendo mientras lo gastas. Con **el descuento del puente** eliges a qué tasa se
cuenta eso:

- **Rentabilidad esperada de tus líquidos** (por defecto) — la media de lo que esperas de tus
  activos líquidos, ponderada por su valor.
- **Tu tasa segura de retirada** — más prudente.
- **Sin descuento** — el máximo de conservador: como si el dinero del puente no creciera nada.

En el mismo ejemplo de arriba, **sin descontar** ese puente costaría 968.000 € en vez de 435.300 €.
Si no tienes ni un euro líquido del que sacar la tasa, FutureFin cae a cero y **lo dice**, en vez de
encarecerte el objetivo en silencio.

Al lado del objetivo verás dos cifras que la cuenta clásica esconde:

- **Tasa de retirada efectiva del puente** — lo que de verdad sacas de la cartera mientras la pensión
  no llega. **Puede estar por encima de tu SWR sin que sea un error**: dura pocos años, no toda la
  vida.
- **Cobertura de la pensión** — qué parte de tu gasto cubre la pensión cuando empieza.

---

## La sección «Riesgo»

El riesgo —*¿y si los mercados no se portan como la media?*— ya no es un panel aparte con su propio
gráfico: es la **banda** que puedes encender sobre el mismo chart de «Resultado», más un bloque
**«Riesgo» compacto** justo debajo («Éxito del plan» y la línea de tu colchón de caja), con todo lo
demás en «Detalle del cálculo», plegado.

### Qué es la banda

FutureFin corre **cientos de veces el mismo plan** —el mismo motor, las mismas reglas, tus mismos
datos— sorteando cada mes cuánto suben o bajan tus inversiones. El interruptor «Banda 10–90 %» del
chart pinta la **franja** que sale de ese sorteo, del escenario 10 al escenario 90 —ocho de cada
diez futuros caen dentro—, sobre la misma línea sólida de siempre (tu patrimonio) y el mismo
objetivo discontinuo. Está encendida por defecto cuando hay escenarios que mostrar: el plan
determinista es una lectura posible, no la única, y esconder la dispersión tras un clic la convierte
en una curiosidad opcional.

### Qué dice el color de la banda

La franja **no es de un solo color**: se tiñe por edad según **qué parte de los escenarios ya se ha
quedado sin capital a esa altura del plan**. Verde donde no falla ninguno, ámbar en cuanto empiezan
a fallar, rojo desde uno de cada diez. Los cortes son fijos y no dependen de nada que tú configures:

| Escenarios agotados a esa edad | Color |
|---|---|
| ninguno (0 %) | verde |
| hasta el 5 % | verde → ámbar, degradando |
| del 5 % al 10 % | ámbar → rojo, degradando |
| 10 % o más | rojo |

Ese último corte no es arbitrario: **el 10 % de escenarios agotados es exactamente el 90 % de
éxito**, el listón por debajo del cual FutureFin pinta tu plan en rojo. La escala se lee del otro
lado, pero es el mismo semáforo.

Pasa el ratón por encima y verás **el porcentaje exacto de esa edad**, no una aproximación del
color: sale del mismo dato que lo tiñe. Y la cifra que el color no puede rotular —el total, cuántos
escenarios se quedaron sin capital **en algún momento** del plan— está en «Detalle del cálculo».

Sin volatilidad declarada en tus activos la banda no se colorea: no hay escenarios que contar, y
pintarla de verde diría que ninguno falla cuando lo que pasa es que no se ha sorteado ninguno.

Los dos bordes de la franja llevan su nombre a la derecha: **optimista (p90)** arriba y **pesimista
(p10)** abajo.

**La banda NO dibuja la mediana como una línea propia.** El valor central de cada mes —el que
usan otras lecturas de esta sección, como los meses por debajo del gasto— sigue viviendo en los
datos, pero no se traza aparte: solo se pinta el área entre el escenario 10 y el 90. Si buscas el
trazo de la mediana en el gráfico, no está — la nota «Bandas puntuales» de «Detalle del cálculo»
te lo recuerda: cada mes se ordena por separado, así que ni siquiera esa mediana correspondería a
un futuro concreto.

### Qué significa «Éxito del plan»

**El éxito exige DOS cosas a la vez: que llegues a jubilarte dentro de tu horizonte —o en tu edad
objetivo— y que, una vez jubilado, la cartera no se agote nunca**, con tus pensiones y tus fases ya
dentro de la simulación. Un plan que nunca te jubila no cuenta como éxito aunque el dinero siga
entero: quedarte para siempre en la fase de ahorro no es lograr tu plan, es no haberlo intentado.
Por eso, cuando algún escenario se queda sin jubilar, debajo aparecen dos cifras más: cuántos son
los que no llegan a jubilarse, y qué parte de los que **sí** se jubilan aguanta el capital hasta el
final. El KPI principal es un porcentaje —«87,0 %»— con debajo, en pequeño, de qué es: «de los
escenarios no agotan el capital».

**El corte del color es fijo y ya no lo eliges tú.** Hubo un ajuste «umbral de éxito» (de serie,
95 %) y se retiró: un listón configurable no hacía tu plan más seguro, solo más indulgente contigo
mismo. Ahora es **verde solo con el 100,0 %** —cero escenarios agotados—, ámbar entre el 90 % y el
100 %, rojo por debajo del 90 %. Con quinientos caminos sorteados, un solo fallo ya es ámbar; por
eso, cuando el tile está verde, el subtítulo dice el recuento exacto («0 de 500 escenarios agotan el
capital»), que es lo que hace auditable ese 100 %.

**Que una regla de retirada te recorte el gasto en un año malo NO cuenta como fracaso.** Eso se mide
aparte, en la **cobertura de tu gasto**: dos cifras que ahora se calculan igual con cualquier regla,
incluida «Gasto fijo» — antes se escondían ahí porque solo miraban lo que la regla se negaba a
sacar, y esa regla no recorta nunca por definición, así que siempre habrían dado «100 % cubierto».
Ahora cuentan también lo que la cartera no pudo pagar porque ya no quedaba de dónde, que es la otra
manera —y la peor— de quedarte corto. Son dos preguntas distintas: «¿me quedo sin dinero?» y «¿tengo
que apretarme el cinturón?». Mezclarlas da un diagnóstico falso — con la regla del «% del saldo»
nunca te quedas a cero, pero puedes acabar retirando céntimos.

La misma cifra de éxito aparece en el Resumen, como KPI «Éxito del plan», y sale **del mismo
sorteo**: no se recalcula en dos sitios, porque entonces la misma pantalla enseñaría dos
probabilidades del mismo plan.

La **probabilidad de agotar el capital** por edad ya no se tabula: **la dice el color de la banda**
(arriba), con más detalle del que cabía en una tabla y con el porcentaje exacto en el hover. Es
**acumulada**: el porcentaje a los 80 incluye a todos los que se quedaron sin dinero a los 75, así
que solo puede crecer con la edad. El total —todos los escenarios que se agotaron en algún momento—
está en «Detalle del cálculo», y esa cifra y el «éxito» cuentan lo mismo desde los dos lados.

### La semilla

El sorteo usa siempre **la misma semilla** para ti, así que la cifra **no baila entre visitas**: la
misma pregunta da la misma respuesta hoy y dentro de un año. Sin eso, una herramienta como esta sería
inservible — refrescarías la página y cambiaría el resultado.

### La volatilidad se declara por activo

Los escenarios necesitan saber **cuánto se mueve** cada cosa que tienes, y eso se declara en
**Activos**, junto a la rentabilidad esperada: el campo **«Volatilidad anual (%)»**.

Es la desviación típica **anual** de sus retornos — no una pérdida esperada ni un peor caso. Como
referencia: renta variable global ~15–18 %, renta fija ~4–6 %, efectivo 0. Son cifras orientativas,
no un dato de tu cartera: si conoces la de tu fondo, pon la suya.

Dos cosas importan:

- **La proyección de siempre —la línea— la ignora.** Declararla no mueve ni un euro de tu curva.
  Solo alimenta las bandas.
- **Si no la declaras en ningún activo, la banda es la línea** y el «éxito» sale del 100 %. FutureFin
  lo avisa, porque un 100 % ahí es aritméticamente cierto y no significa nada.

### El colchón de caja, contado honestamente

El **colchón de caja** es dinero que la simulación mantiene siempre en efectivo durante la
jubilación, para no tener que vender en un año malo. **Ya no se lo pides tú**: sale del **tope de tu
regla de ahorro**. Si tienes una regla del tipo «hasta 6.000 € en la Cuenta corriente, el resto al
fondo indexado», esos 6.000 € SON tu colchón — ya declaraste cuánto querías en caja al escribir la
regla, y volver a preguntártelo en meses era pedirte el mismo dato dos veces con otra unidad.

El bloque «Riesgo» te dice de dónde sale, cuánto es y a cuántos meses de tu gasto de hoy equivale, y
enlaza a Reglas de ahorro, que es donde se cambia. **Es el importe, no los meses**: el tope de tu
regla es una cifra fija en euros, mientras que «ocho meses de gasto» crecería con la inflación hasta
valer casi el doble a veinte años — dos cosas distintas, y la que tú declaraste es la primera. Los
meses que ves al lado son solo una equivalencia informativa.

Si no tienes ninguna regla con tope apuntando a un activo líquido **sin volatilidad** declarada, no
hay colchón, y la línea te dice cuál de las dos cosas falta en vez de callarlo. (Un colchón que
también baja con el mercado no es un colchón.) Solo existe en los escenarios con volatilidad — en la
línea determinista no hay meses buenos ni malos que distinguir, así que no tendría criterio.

Se rellena vendiendo del resto de la cartera **solo con el shock del mes YA ocurrido**, nunca
adelantándose al mercado, y **solo entre tus activos líquidos** — nunca vendiendo la vivienda para
llenarlo.

Se puede fijar a mano por API o por MCP (en meses), y entonces manda sobre la derivación; la línea
lo dice y te ofrece «Volver al tope de tu regla» para soltarlo.

**Y aquí va lo honesto, con las dos caras**: el colchón **SÍ protege** — medido, sube 3,9 puntos la
probabilidad de éxito y casi dobla tu liquidez en el peor 10 % de los futuros —, pero **esa
protección se paga**, y el precio es real: es la rentabilidad a la que renuncias por tener esos
meses de gasto fuera del mercado en vez de invertidos. Si el dinero del colchón vive en una cuenta
remunerada al 0 % —el caso más habitual—, ese coste se come la protección y el balance neto sale
**negativo**, −3,5 puntos. El colchón no es gratis ni es mala idea por sistema: es tranquilidad que
cuesta rentabilidad, y cuánto cuesta depende de dónde lo alojes. Se ofrece porque hay hogares que
prefieren esa tranquilidad, y porque el número que importa es el tuyo: míralo en esta misma sección,
con tus datos, antes de decidir.

---

## Nada se fuerza sin que lo veas: todo campo en vigor está en su tarjeta

«Tu plan» te enseña solo los campos de tu estrategia — nada de fecha de nacimiento sin usarla, nada
de bloque de pensión si no la has activado. Pero esconder un campo no es lo mismo que borrarlo: tu
plan sigue asumiendo una tasa de retirada, una regla concreta y un horizonte aunque no te los esté
preguntando en ese momento.

La respuesta a eso **son las tarjetas**. Todos esos supuestos viven en una —«Retirada»,
«Horizonte»— junto a la frase que explica qué mueven, a la vista y sin desplegar nada. La regla es
simple: **si tu plan lo asume, está en una tarjeta; si no está en ninguna tarjeta, tu plan no lo
usa.**

> **Antes había un acordeón «Avanzado» y una línea «Supuestos: retirada 3,5 % · gasto fijo en euros
> de hoy · horizonte 90 años · sin colchón · umbral 95,0 %».** Los dos se retiraron a la vez, y no
> por separado: la línea existía **para enunciar lo que el acordeón escondía**. Sin acordeón no hay
> nada escondido que enunciar. Dos de los campos que vivían ahí ya ni existen: el colchón de caja se
> deriva de tu regla de ahorro y el umbral de éxito es fijo.

---

## El hogar

Con **Hogar** activo, la pestaña Proyección enseña la suma de todo el mundo:

- Una **línea gruesa** con el total, y **una línea fina por persona** con sus propios marcadores,
  identificadas por nombre en la leyenda.
- Los tiles del Resumen agregados, más **«Planes del hogar»**: no una rejilla de tarjetas, sino una
  lista de frases, una por miembro (el mismo hito que da la pestaña Jubilación en tercera persona),
  cada una con el punto de color de su línea fina en el chart.

En la propia pestaña **Jubilación**, Hogar sustituye los tres bloques de configuración/resultado
por un único aviso de solo lectura, la misma lista de frases por persona, y el enlace
**Cambia a «Yo» para editar tu plan**.

Tres cosas que conviene entender:

1. **El hogar no tiene plan propio.** No hay una «jubilación del hogar», porque no existe: hay N
   planes. Los campos de jubilación aparecen vacíos y con su razón, y el hito de cada persona va en
   su frase.
2. **El hogar no simula, suma.** El servidor corre una simulación por persona —con su perfil, su
   fecha de nacimiento y sus filas— hasta el horizonte más largo de los miembros, y las suma.
   **Si sois dos o más, los números del gráfico cambian respecto de la 4.x**: antes se simulaba una
   sola cartera con una sola estrategia. Si vives solo, no se mueve nada.
3. **El hogar es de solo lectura, y de verdad.** No es que se escondan los botones: **el servidor
   también lo impide**. Editar o borrar una fila de otra persona devuelve un error de permisos, y
   **el propietario de la instalación tampoco puede** — ser dueño del hogar no es ser dueño de la
   fila. Leer sigue siendo libre: en Hogar ves el conjunto entero.

La **banda de escenarios y el bloque «Riesgo»** no están disponibles en Hogar: los percentiles no se
suman entre personas (el escenario 90 del hogar no es la suma de los escenarios 90 de cada uno), así
que FutureFin no lo intenta en vez de enseñar una banda que nadie podría interpretar. Pero en Hogar
tampoco hay «Tu plan»/«Resultado» que enseñar — ver arriba.

---

## Qué cambió en Ajustes → Plan

`Ajustes → Plan` **ya no tiene** tu SWR, tu modo de objetivo ni tu edad límite: se mudaron a
Jubilación, que es donde vive tu plan. En su lugar hay un puntero a esa pestaña.

Lo que **sigue** en `Ajustes → Plan`, porque es del hogar y no de una persona:

- La **inflación anual asumida**.
- El **modo de edad** (fechas o edades en el eje del gráfico).
- Los **tramos de IRPF** del ahorro y si los impuestos están activados.
- La **fuente del ahorro** de la simulación y sus ventanas de promedio (ver
  [Configuración](configuracion.md#la-fuente-del-ahorro-el-ajuste-que-redefine-tu-proyección)).

Es el reparto que la 5.0.0 establece y que conviene tener en la cabeza: **el hogar guarda los
supuestos compartidos; cada persona guarda su plan.**

---

## Ver también

- [Actualizar](actualizar.md#actualizar-a-la-500) — qué pasa al saltar desde la 4.x, y qué verá un
  hogar de dos personas.
- [Configuración](configuracion.md#ajustes-de-la-instalación-dentro-de-la-app-no-por-entorno) — los
  ajustes que siguen siendo del hogar.
- [Conectar Claude](mcp.md) — desde el chat puedes preguntar «¿y si me jubilo a los 55?» sin guardar
  nada.
