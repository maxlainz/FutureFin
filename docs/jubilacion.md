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

Lo primero de la pestaña Jubilación son **cinco tarjetas**. Eliges una y el formulario de abajo se
queda solo con los campos que esa estrategia necesita: no hay que rellenar lo que no aplica.

### Cuanto antes (FIRE clásico)

> Ahorras todo lo que puedes y te jubilas el mes en que tu patrimonio líquido cubre el objetivo.

Es la conducta de siempre. **No necesita nada más**: ni edad, ni pensión. La fecha de jubilación es
el mes del **cruce** — cuando la línea de tu patrimonio líquido alcanza la del objetivo.

### A una edad fija

> Eliges la edad; el plan te dice cuánto necesitas ahorrar y cuánto margen te sobra.

**Necesita** tu edad objetivo y tu **fecha de nacimiento** (en «Tu cuenta»; el propio formulario te
lo pide si falta).

Aquí **manda la edad**, no el capital: te jubilas ese mes llegues o no. Lo que publica:

- **Ahorro necesario** — la aportación mensual mínima que te hace llegar al objetivo justo en esa
  edad. No se despeja de una fórmula: se **simula** el plan entero probando aportaciones hasta dar
  con la mínima que llega, con tu cascada de reparto, tus topes, tus deudas y tus impuestos dentro.
  Es un **techo** sobre lo que tu reparto invierte cada mes, no un importe que se aporte pase lo que
  pase: un mes con menos sobrante aporta lo que hay.
- **Margen disponible** — lo que te sobra por encima de eso, al mes y acumulado.
- **Capital necesario** — una línea de puntos en el gráfico: la trayectoria que tendrías ahorrando
  exactamente lo necesario. La tuya por encima es margen.

**El estado rojo**: si ni invirtiendo cada euro que te sobra llegas, sale un aviso en rojo —
*«Con tu ahorro actual no llegas a los N años. Te jubilarás igual —manda la edad— pero por debajo de
tu objetivo»*— y la tarjeta «Tu plan» del Resumen también se pone en rojo. No es un error: la
simulación existe, se jubila igual y se publica entera. Lo que te está diciendo es que te jubilas
**por debajo** de tu objetivo.

### Ahorrar ahora y dejar crecer (Coast FIRE)

> Aportas fuerte hasta que el capital llegue solo a tu edad objetivo; después, cada euro es margen.

**Necesita** edad objetivo y fecha de nacimiento, igual que la anterior. Publica además:

- **Mes coast** — el primer mes a partir del cual puedes **dejar de aportar** y aun así llegar al
  objetivo en tu edad. Sale de simular el plan cortando la aportación en cada mes candidato.
- **Número coast** — el patrimonio líquido con el que entras en ese mes. Ojo: no es tu objetivo de
  jubilación ni tu patrimonio total (la vivienda no cuenta).
- Una **línea discontinua** en el gráfico: «si dejas de aportar aquí».

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
Al lado se dice si el patrimonio **creció o menguó** durante esa fase.

### Puente hasta la pensión

> Te jubilas por cruce y vives del capital hasta que llegue la pensión pública; el objetivo se
> dimensiona con ese puente.

**Necesita** el bloque de pensión. Cambia el **objetivo**, no solo la caja — ver
[La base del objetivo](#la-base-del-objetivo-perpetua-o-puente) más abajo.

---

## La pensión pública, con fecha

En la 4.x una pensión se apuntaba como un ingreso que «persiste tras la jubilación», y la simulación
se la restaba desde el primer mes de jubilación aunque fuera a llegar veinte años más tarde. Desde
la 5.0.0 la pensión tiene **fecha**.

Se declara con dos datos: el **importe mensual en euros de hoy** y la **edad a la que empieza a
cobrarse**. Además:

- **Indexada** (por defecto): crece con la inflación que tengas configurada. Puedes desactivarlo si
  prefieres asumir que se queda plana.
- **Fracción durante la media jornada**: qué parte de esa pensión cobras ya durante la fase parcial,
  si es que cobras alguna. Por defecto, ninguna.

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

Justo debajo del gráfico, la sección **Riesgo** contesta la pregunta que una sola línea no puede
contestar: *¿y si los mercados no se portan como la media?*

### Qué son las bandas

FutureFin corre **cientos de veces el mismo plan** —el mismo motor, las mismas reglas, tus mismos
datos— sorteando cada mes cuánto suben o bajan tus inversiones. De ahí salen tres líneas:

- La **franja** va del escenario 10 al escenario 90: ocho de cada diez futuros caen dentro.
- La **discontinua** es la mediana.
- La **sólida** es la proyección de siempre, la que ya conocías.

**Cuidado con la mediana**: es el valor central **de cada mes por separado**, así que no es un futuro
concreto y no cuadra con ninguna simulación individual. Es una lectura de la franja, no un camino.

Y una cosa que se ve enseguida: la mediana queda **por debajo** de la línea determinista. No es un
fallo. Es el coste de la volatilidad — con la misma rentabilidad media, un camino con altibajos
termina más abajo que uno liso.

### Qué significa «Éxito del plan»

**El éxito exige DOS cosas a la vez: que llegues a jubilarte dentro de tu horizonte —o en tu edad
objetivo— y que, una vez jubilado, la cartera no se agote nunca**, con tus pensiones y tus fases ya
dentro de la simulación. Un plan que nunca te jubila no cuenta como éxito aunque el dinero siga
entero: quedarte para siempre en la fase de ahorro no es lograr tu plan, es no haberlo intentado.
Por eso, cuando algún escenario se queda sin jubilar, debajo aparecen dos cifras más: cuántos son
los que no llegan a jubilarse, y qué parte de los que **sí** se jubilan aguanta el capital hasta el
final. El KPI principal se colorea contra tu umbral (de serie, 95 %): verde en el umbral, ámbar
hasta diez puntos por debajo, rojo el resto.

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

Debajo, la tabla de **probabilidad de agotar el capital** por edad. Es **acumulada**: el porcentaje a
los 80 incluye a todos los que se quedaron sin dinero a los 75. Solo puede crecer con la edad, y su
última fila y el «éxito» cuentan lo mismo desde los dos lados.

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

En «Horizonte y riesgo» puedes declarar un **colchón de caja**: unos meses de gasto siempre en
efectivo, para no tener que vender en un año malo. Solo existe en los escenarios con volatilidad —
en la línea determinista no hay meses buenos ni malos que distinguir, así que no tendría criterio.

Se rellena vendiendo del resto de la cartera **solo con el shock del mes YA ocurrido**, nunca
adelantándose al mercado, y **solo entre tus activos líquidos** — nunca vendiendo la vivienda para
llenarlo. Y necesita un activo líquido **sin volatilidad** declarada donde vivir: sin uno así, el
colchón no se simula, y aquí se te dice por qué en vez de fingir que sí.

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

## El hogar

Con **Hogar** activo, la pestaña Proyección enseña la suma de todo el mundo:

- Una **línea gruesa** con el total, y **una línea fina por persona** con sus propios marcadores,
  identificadas por nombre en la leyenda.
- Los tiles del Resumen agregados, más **una tarjeta por miembro** con su estrategia, su hito y su
  estado.

Tres cosas que conviene entender:

1. **El hogar no tiene plan propio.** No hay una «jubilación del hogar», porque no existe: hay N
   planes. Los campos de jubilación aparecen vacíos y con su razón, y el hito de cada persona va en
   su tarjeta.
2. **El hogar no simula, suma.** El servidor corre una simulación por persona —con su perfil, su
   fecha de nacimiento y sus filas— hasta el horizonte más largo de los miembros, y las suma.
   **Si sois dos o más, los números del gráfico cambian respecto de la 4.x**: antes se simulaba una
   sola cartera con una sola estrategia. Si vives solo, no se mueve nada.
3. **El hogar es de solo lectura, y de verdad.** No es que se escondan los botones: **el servidor
   también lo impide**. Editar o borrar una fila de otra persona devuelve un error de permisos, y
   **el propietario de la instalación tampoco puede** — ser dueño del hogar no es ser dueño de la
   fila. Leer sigue siendo libre: en Hogar ves el conjunto entero.

La sección **Riesgo** no está disponible en Hogar: los percentiles no se suman entre personas (el
escenario 90 del hogar no es la suma de los escenarios 90 de cada uno), así que FutureFin no lo
intenta en vez de enseñar una banda que nadie podría interpretar.

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
