/**
 * Catálogo de descripciones de métricas y ajustes — FUENTE DE VERDAD en prosa.
 *
 * Cada entrada es el CONTRATO de una métrica: qué mide, con qué base y con qué ventana. Si el
 * código y el texto discrepan, uno de los dos es un bug — nunca una divergencia tolerable. Esa
 * disciplina, y cuándo hay que actualizar una entrada, viven en la skill
 * `futurefin-metric-definitions`.
 *
 * Reglas de redacción:
 *  - Español, tuteando, sin jerga de implementación (nada de «endpoint», «JSONB», «engine»).
 *  - La BASE siempre explícita: de dónde salen los números y qué queda fuera.
 *  - Si una cifra depende del modo o de una ventana, se dice — es justo la ambigüedad que este
 *    catálogo existe para eliminar.
 *  - Un par de frases. Lo que no quepa, no es ayuda: es documentación.
 */

export type HelpText = { title: string; body: string };

export const HELP_TEXTS = {
  // --- Resumen · Salud financiera ------------------------------------------
  "summary.savings": {
    title: "Ahorro mensual",
    body:
      "Lo que la simulación da por ahorrado cada mes, y la única cifra de ahorro con la que se " +
      "proyecta tu patrimonio. De dónde salen el ingreso y el gasto depende de la fuente del " +
      "ahorro que hayas elegido en Ajustes → Plan. El porcentaje de abajo es esa misma " +
      "cifra sobre ese mismo ingreso, así que las dos nunca pueden contradecirse. La flecha " +
      "compara ese ahorro con el que sale de tu presupuesto: en verde ahorras más de lo planeado, " +
      "en rojo menos.",
  },
  "summary.liquid_assets": {
    title: "Activos líquidos",
    body:
      "Suma de los activos que marcaste como líquidos, es decir, de los que podrías disponer sin " +
      "venderlos ni penalización. El porcentaje es la parte que representan sobre tus activos " +
      "totales.",
  },
  "summary.runway": {
    title: "Autonomía",
    body:
      "Meses que tus activos líquidos cubrirían tu gasto si dejaras de ingresar hoy. No es una " +
      "división simple: se vacían en el mismo orden que en la simulación —primero los de menor " +
      "rentabilidad esperada, mientras el resto sigue rindiendo la suya—, y el gasto con el que " +
      "se compara crece cada año con la inflación que tengas configurada, la misma regla que " +
      "aplica la Proyección a tu gasto. Qué gasto se usa depende de la fuente del ahorro " +
      "(Ajustes → Plan): con " +
      "el presupuesto, el presupuestado; con los movimientos reales, tu gasto real promediado. " +
      "«Indefinida» exige dos cosas: que tu retirada anual quepa en el SWR y que tus líquidos " +
      "tengan rentabilidad esperada positiva — el dinero parado al 0 % siempre se agota, y aquí " +
      "se dice en meses. Vender para vivir tributa: en los activos con precio de compra " +
      "declarado, solo su ganancia real; en el resto, la fracción de Plusvalía gravable. " +
      "«+100 años» es un suelo, no una medida exacta.",
  },
  "summary.net_return": {
    title: "Rendimiento neto",
    body:
      "Lo que se espera que rinda tu patrimonio en un año: la rentabilidad que has configurado en " +
      "cada activo, pesada por lo que vale cada uno, menos el interés de tus deudas, pesado por lo " +
      "que te queda por pagar, todo sobre tu patrimonio neto. Un activo sin rentabilidad puesta " +
      "cuenta como 0 % y sigue restando media. La cifra grande ya descuenta la inflación que " +
      "tengas configurada; el paréntesis es la misma sin descontarla. No es lo que has ganado: es " +
      "una expectativa, no tu rentabilidad pasada. Aquí el interés de tus deudas se cuenta con " +
      "la MISMA base que la simulación: solo devengan las que tienen un modelo con intereses, " +
      "TIN puesto y plan de pagos vivo. Una deuda sin intereses, sin cuota o con el plan " +
      "vencido no genera coste, aunque su saldo sí sigue restando en el patrimonio del " +
      "denominador. Tu rentabilidad de activos compone mes a mes; el " +
      "interés de tus deudas se aplica como un tipo simple sobre lo que debes, y no distingue " +
      "meses cortos de largos —un 3 % de activo y un 3 % de deuda no son la misma velocidad—. " +
      "Si debes más de lo que tienes, la tarjeta desaparece: el " +
      "porcentaje diría lo " +
      "contrario de lo que pasa.",
  },
  "assets.expected_return": {
    title: "Rentabilidad anual esperada",
    body:
      "La tasa con la que la simulación hace crecer este activo cada año. Es la rentabilidad " +
      "ANUALIZADA que tu fondo o tu cuenta YA publica —la compuesta, neta de sus comisiones: no " +
      "le restes nada tú— y es NOMINAL: la inflación se aplica aparte. Los escenarios la tratan " +
      "como lo que es, un crecimiento compuesto: el escenario central crece a este ritmo, y la " +
      "media aritmética de todos sale algo por encima porque la volatilidad separa las dos " +
      "cifras. Si metes aquí la rentabilidad «real» (la ya descontada de inflación) en vez de la " +
      "nominal, tu plan se vuelve más pesimista de lo que crees y tu fecha se retrasa sin que " +
      "sepas por qué.",
  },
  "assets.volatility": {
    title: "Volatilidad anual",
    body:
      "Cuánto se mueve este activo arriba y abajo en un año: la desviación típica de su " +
      "rentabilidad, no una pérdida esperada. Como orientación, la renta variable global ronda " +
      "el 15–18 %, la renta fija el 4–6 % y el efectivo es 0. Son cifras de referencia, no un " +
      "dato de tu cartera: pon la de tu fondo si la conoces. Es lo que separa unos escenarios de " +
      "otros y, con ellos, tu éxito y tu fecha: sin volatilidad declarada en ningún activo los " +
      "miles de escenarios son el mismo, el éxito sale 0 % o 100 % y deja de medir riesgo. " +
      "Subirla NO cambia el ritmo central —el escenario central sigue creciendo a la " +
      "rentabilidad que declaras—: abre el abanico. Dejarla vacía significa «este activo crece " +
      "sin sobresaltos».",
  },
  "summary.debt_to_assets_ratio": {
    title: "Ratio deuda / activos",
    body:
      "Cuánta deuda tienes por cada euro de activo: tus pasivos entre tus activos totales. " +
      "Un préstamo vencido con saldo vivo sigue contando (marcado «plan vencido con saldo»), " +
      "igual que en el resto del Resumen; solo el vencido y saldado desaparece. Sin activos " +
      "registrados, esta cifra no se calcula.",
  },
  "liabilities.principal_total": {
    title: "Principal total",
    body:
      "Suma del principal pendiente de los pasivos de esta lista. Un plan vencido con saldo " +
      "vivo sigue aquí y sigue sumando; solo el vencido y saldado desaparece. No incluye los " +
      "intereses que te queden por pagar, solo el capital.",
  },
  "liabilities.monthly_service": {
    title: "Servicio mensual equivalente",
    body:
      "Suma de la cuota de cada pasivo, llevada a mensual: la que declaraste mensual se queda " +
      "igual, la semanal se multiplica por 52 semanas y se reparte entre 12 meses. Es la cuota " +
      "que tú registraste, no una estimación de lo que deberías pagar; un pasivo sin cuota o sin " +
      "frecuencia cuenta como 0 €, y un plan ya vencido (aunque siga en la lista con su saldo) " +
      "tampoco suma: esa cuota ya no se gira — la misma base que usa el presupuesto.",
  },
  "liabilities.weighted_apr": {
    title: "TIN medio ponderado",
    body:
      "El tipo medio que tu deuda te cuesta HOY: media del TIN, ponderada por el principal, " +
      "solo de los pasivos que devengan interés — modelo con intereses, TIN puesto y plan de " +
      "pagos vivo, la MISMA base que la simulación y que el Rendimiento neto del Resumen. Un " +
      "pasivo sin intereses, sin TIN o con el plan vencido (saldo congelado) queda fuera del " +
      "cálculo entero: sigue siendo deuda, pero no te cuesta tipo. (Es el TIN nominal del " +
      "cuadro de amortización, no la TAE con comisiones de tu contrato.)",
  },
  "liabilities.approx_monthly_interest": {
    title: "Interés mensual aprox.",
    body:
      "Estimación de orden de magnitud: principal actual × TIN ÷ 12 de cada pasivo que DEVENGA " +
      "— modelo con intereses, TIN puesto y plan de pagos vivo, la misma base que usa la " +
      "simulación (antes esta cifra cobraba interés a pasivos que la proyección simulaba a " +
      "0 €). No descuenta la amortización del mes, así que va ligeramente por encima del " +
      "interés exacto del calendario.",
  },
  "summary.net_worth": {
    title: "Patrimonio neto",
    body:
      "Activos totales menos pasivos pendientes. Un préstamo cuyo plan venció con saldo " +
      "vivo SIGUE restando —la deuda no se extingue por calendario; aparece marcada como " +
      "«plan vencido con saldo»—. Solo el vencido y saldado (capital a cero) deja de contar, " +
      "aunque sigue guardado en tu historial.",
  },

  "summary.plan": {
    title: "Tu plan",
    body:
      "Tu estrategia de jubilación y adónde te lleva: la frase dice en qué mes te jubila la " +
      "simulación, con qué edad y cuántos escenarios de cada 100 aguantan hasta el final del " +
      "horizonte. Con «Cuanto antes» y con la jornada reducida esa fecha la resuelve el sorteo " +
      "—el primer mes que cumple tu umbral de éxito—; con «A una edad fija» es la edad que " +
      "elegiste, aguante o no, y entonces el color lo dice. Las dos cifras de al lado, el éxito y " +
      "el capital necesario hoy, son las MISMAS del panel de Jubilación, copiadas del mismo " +
      "sorteo y nunca recalculadas aquí. Sin tu fecha de nacimiento no hay fecha, ni éxito, ni " +
      "capital: la tarjeta lo dice en vez de inventarlos. En la vista del hogar no hay un plan " +
      "común: se enseña una frase por persona.",
  },

  "summary.success": {
    title: "Éxito del plan",
    body:
      "De cada 100 escenarios, en cuántos NO tendrías que volver a trabajar jubilándote en la " +
      "fecha de tu plan. El color se compara con TU umbral de éxito —el de Jubilación → " +
      "Retirada—, no con un listón fijo, y se juzga contra el suelo del intervalo del sorteo: " +
      "verde cuando ese suelo ya cumple tu umbral, ámbar cuando el umbral cae dentro del " +
      "intervalo, rojo cuando no llega. Es el MISMO sorteo que dibuja la sección Riesgo de " +
      "Jubilación —no se recalcula aquí—, y allí está el detalle: por qué fallan los que fallan " +
      "y cuánto se apretaron los que aguantan.",
  },

  // --- Jubilación · plan y perfil -------------------------------------------
  "retirement.plan_sentence": {
    title: "Tu hito de jubilación",
    body:
      "El resultado de tu plan en una frase: cuándo te jubila la simulación, con qué edad y " +
      "cuántos escenarios de cada 100 aguantan hasta el final del horizonte. Qué manda depende de " +
      "tu estrategia: en «Cuanto antes» y en la jornada reducida la fecha la resuelve el sorteo " +
      "—es el primer mes que cumple tu umbral—; en «A una edad fija» la pones tú y lo que se " +
      "resuelve es si aguanta; en «Coast FIRE» el hito incluye el mes en que puedes dejar de " +
      "aportar. «Nunca» es una respuesta, no un dato que falte: significa que ningún mes del " +
      "horizonte llega a tu umbral.",
  },
  "retirement.strategy": {
    title: "Tu estrategia de jubilación",
    body:
      "Qué le pides al plan y, con ello, qué te pregunta. «Cuanto antes» busca tu primera fecha " +
      "válida. «A una edad fija» te jubila en la edad que pidas —aguante o no— y te dice cuántos " +
      "escenarios lo soportan y cuánto tendrías que aportar para llegar a tu umbral. «Coast " +
      "FIRE» resuelve cuándo puedes dejar de aportar. «Jornada reducida» mete una fase de menos " +
      "ingreso antes del final. El puente hasta la pensión ya NO es una estrategia: es un ajuste " +
      "de la tarjeta Pensión y funciona con las cuatro. Es tuya, no del hogar: cada persona " +
      "tiene la suya.",
  },
  "retirement.target_age": {
    title: "Edad de jubilación objetivo",
    body:
      "La edad en la que dejas de trabajar en la simulación. En «A una edad fija» manda: te " +
      "jubilas ahí aunque los escenarios no acompañen, y entonces el éxito te dice a qué te " +
      "expones y la aportación mínima cuánto costaría cumplir tu umbral. En «Coast FIRE» es la " +
      "edad contra la que se resuelve cuándo puedes dejar de aportar. En la jornada reducida es " +
      "opcional y marca el fin de la fase; sin ella, la jubilación total llega cuando tu fecha " +
      "válida lo permite. Necesita tu fecha de nacimiento para convertirse en un mes concreto.",
  },
  "retirement.coast_mode": {
    title: "Qué fijas en Coast",
    body:
      "En Coast FIRE hay dos preguntas y solo puedes contestar una: la otra la resuelve el plan. " +
      "Si fijas tu edad de jubilación, se resuelve el PRIMER mes en que puedes dejar de aportar " +
      "y aun así llegar a esa edad cumpliendo tu umbral. Si fijas la edad en que dejas de " +
      "aportar, se resuelve tu fecha válida, salga donde salga. En los dos casos el plan que ves " +
      "deja de aportar DE VERDAD desde ese mes —la línea, la banda y los escenarios corren sin " +
      "aportaciones— y el ahorro que liberas es dinero disponible para gastar: no vuelve a la " +
      "cartera.",
  },
  "retirement.partial_mode": {
    title: "Jornada reducida (Barista FIRE)",
    body:
      "Trabajar menos y cobrar menos durante una fase, antes de dejarlo del todo. Eliges cuándo " +
      "empieza: a una edad que fijas tú, o en cuanto tu plan pueda permitírsela —el primer mes " +
      "en el que la fase no deja tu cartera sin cubrir el gasto—. La jubilación total no es una " +
      "edad aparte: es tu fecha válida calculada ya con la fase dentro, así que meter la fase " +
      "puede retrasarla. El ingreso de la fase lo declaras en euros de hoy y se queda plano: no " +
      "se actualiza con la inflación, y compra menos cuanto más dure. NO es la jubilación " +
      "parcial de la Seguridad Social: aquí no hay cotización, ni porcentajes legales, ni " +
      "contrato de relevo — solo cuánto ingresas y desde cuándo.",
  },
  "retirement.partial": {
    title: "Media jornada",
    body:
      "Los datos de la fase: cuándo empieza, cuánto ingresas en ella y con qué gasto se compara " +
      "(el de jubilación, salvo que elijas tu gasto regular de hoy). El ingreso va en euros de " +
      "hoy y se queda PLANO —no sube con la inflación—, y 0 € es un año sabático. El hueco hasta " +
      "tu gasto lo cubres vendiendo cartera; el mes en que no llegue, ese escenario cuenta como " +
      "fallido. La fase no tiene fin propio: termina cuando llega la jubilación total. No es la " +
      "jubilación parcial de la Seguridad Social.",
  },
  "retirement.pension": {
    title: "Pensión pública",
    body:
      "Una renta vitalicia con FECHA: importe mensual en euros de hoy y edad a la que empieza a " +
      "cobrarse. Entra en el plan como un ingreso más el mes en que arranca — si sobra, tus " +
      "reglas de ahorro reparten el sobrante; si falta, se vende cartera. No dimensiona nada por " +
      "adelantado: lo que cambia su fecha es cuántos años tiene que pagar tu capital antes de " +
      "que llegue. Indexada sube cada año con tu inflación; sin indexar se queda plana y compra " +
      "menos con los años. En esta misma tarjeta vive el puente, que es el permiso para sacar " +
      "más de tu tasa durante esos años.",
  },
  "retirement.bridge_settings": {
    title: "Puente hasta la pensión",
    body:
      "Jubilarte antes de cobrar la pensión y pagar esos años vendiendo cartera: sin sueldo y " +
      "sin aportaciones, el gasto sale entero de tu capital. Activarlo cambia UNA cosa: si al " +
      "jubilarte falta menos que los años máximos que fijes para que entre la pensión, el tope " +
      "de lo que puedes sacar el primer año pasa a ser la tasa del puente en lugar de tu tasa de " +
      "retirada. Durante el puente no hay otro tope: si aguanta o no lo dicen las mismas reglas " +
      "que el resto del plan. Con el puente activado tu fecha válida nunca cae antes de la " +
      "pensión menos esos años. Viene apagado, y apagado la pensión sigue entrando igual el mes " +
      "que le toca.",
  },
  "retirement.success_threshold": {
    title: "Umbral de éxito",
    body:
      "La parte de los escenarios que tu plan tiene que aguantar hasta el final del horizonte " +
      "para que una fecha valga. No es un color: es la condición que DECIDE la fecha — subirlo " +
      "la retrasa, bajarlo la adelanta. No se compara con el porcentaje grande, sino con el " +
      "suelo de su intervalo de confianza: con 2.500 caminos un 95,0 % puede ser en realidad un " +
      "93,8 %, y es esa cota la que manda. Al 100 % la exigencia es que no falle NI UN escenario " +
      "y se publica hasta dónde puede llegar el riesgo que no se ve: cero fallos de 2.500 " +
      "caminos siguen siendo compatibles con un 0,12 % de fallo real. De serie, 95 %.",
  },
  "retirement.withdrawal_rule": {
    title: "Regla de retirada",
    body:
      "Cuánto sacas de tu patrimonio cada mes una vez jubilado. «Gasto fijo» retira lo que " +
      "necesitas, indexado, sin techo. «Un % del saldo» retira ese porcentaje de tu líquido del " +
      "mes anterior: nunca se agota, pero tu nivel de vida sube y baja con el mercado. " +
      "«Híbrida» empieza alta y baja al llegar a un saldo. «Con bandas» recorta o sube la " +
      "retirada al salirse de su banda. Con las tres reglas por saldo hay una condición dura: el " +
      "mes en que lo que la regla permite no cubra tu gasto ordinario, ese escenario cuenta como " +
      "fallido — el fracaso que este plan mide es tener que volver a trabajar. La tasa de " +
      "retirada es otra cosa y va aparte: es lo máximo que puedes sacar el PRIMER año, la puerta " +
      "que tu fecha tiene que pasar. Los porcentajes son BRUTOS: el impuesto de la venta va " +
      "dentro.",
  },
  "retirement.spend_mode": {
    title: "Cómo se aplica la regla",
    body:
      "Dos lecturas de la misma regla. Como TECHO, retiras lo que necesitas y nunca más de lo " +
      "que la regla permite: si tu gasto cabe, no vendes de más. Como GASTO, retiras lo que dice " +
      "la regla haya o no necesidad, y ese es tu nivel de vida — con un buen año sacas más y con " +
      "uno malo, menos. La condición es la misma en los dos: el mes en que lo permitido no llegue " +
      "a tu gasto ordinario, ese escenario cuenta como fallido. No mueve tu fecha por sí solo: " +
      "cambia cuánto sale de la cartera cada mes, y con ello cuántos escenarios aguantan.",
  },

  // --- Jubilación · resultado -----------------------------------------------
  "retirement.needed_capital": {
    title: "Capital necesario hoy",
    body:
      "El patrimonio LÍQUIDO que haría falta hoy —invertido con tu misma mezcla de activos— " +
      "para que, jubilándote ya, aguanten hasta el final del horizonte tantos escenarios de cada " +
      "100 como pida tu umbral. Tu vivienda no cuenta: no paga la compra del mes. Siempre en " +
      "euros de hoy —aquí, en el Resumen y en la Proyección, la misma cifra al euro— y " +
      "redondeado a cientos hacia arriba, porque sale de un sorteo y el euro exacto fingiría una " +
      "precisión que no hay. La curva del gráfico es esta misma cifra calculada para cada edad, " +
      "sin escalar: no tiene por qué cruzar tu línea de patrimonio, y que no la cruce no es un " +
      "error del dibujo — tu fecha la deciden los escenarios que aguantan, no un cruce. El " +
      "número FIRE clásico es otra cosa y está en «Detalle del cálculo».",
  },
  "retirement.safe_date": {
    title: "Fecha válida",
    body:
      "El primer mes en el que, jubilándote entonces, aguantan hasta el final del horizonte " +
      "tantos escenarios de cada 100 como pida tu umbral. Cada escenario llega a ese mes con su " +
      "propia historia —su racha buena o mala por el camino—, no con una media. Se busca a " +
      "saltos y se afina mes a mes, y el que se publica va confirmado con 2.500 caminos: no es " +
      "una interpolación. Al lado tienes las fechas al 100 % y al 90 %, que acotan la tuya por " +
      "arriba y por abajo. «Nunca» es un resultado, no un hueco: ningún mes del horizonte llega " +
      "a tu umbral.",
  },
  "retirement.required_contribution": {
    title: "Aportación mínima",
    body:
      "Lo MÍNIMO que tendrías que aportar cada mes, además de lo que ya aportas, para cumplir tu " +
      "umbral en la edad que pediste. No sale de despejar una fórmula: se prueba una cantidad, " +
      "se sortean los escenarios enteros y se ajusta hasta dar con la más pequeña que cumple. Es " +
      "una cifra PLANA —los mismos euros cada mes, sin subirla con la inflación—, así que sale " +
      "algo más alta que si la subieras cada año. Y es un TECHO sobre lo que tu reparto " +
      "invierte, no un importe que se aporte pase lo que pase: un mes con menos sobrante aporta " +
      "lo que hay. El paréntesis es tu sobrante máximo, para que la cifra tenga denominador. «Ni " +
      "ahorrándolo todo» significa que ni invirtiendo cada euro de sobrante llegas a esa edad.",
  },
  "retirement.coast_month": {
    title: "Mes coast",
    body:
      "El primer mes a partir del cual puedes dejar de aportar y aun así llegar a tu edad de " +
      "jubilación cumpliendo el umbral: desde ahí, lo que ya tienes invertido hace el resto " +
      "solo. Sale de simular el plan entero parando en cada mes candidato y contar escenarios, " +
      "no de una regla de tres. Y no es un ejercicio teórico: desde ese mes el plan que ves —la " +
      "línea, la banda, el éxito— ya no aporta nada, y el ahorro liberado es dinero para gastar, " +
      "no capital que vuelve a la cartera. «No puedes parar nunca» significa que ni aportando " +
      "todos los meses llegas a esa edad con tu umbral; no que falte el dato.",
  },
  "retirement.fire_number_classic": {
    title: "Número FIRE clásico",
    body:
      "La cuenta de toda la vida: tu gasto anual de jubilación dividido entre tu tasa de " +
      "retirada —25 veces el gasto con el 4 % clásico, más con tasas más bajas—, con los " +
      "impuestos por delante si los tienes activados y SIN restar la pensión que declares. Está " +
      "aquí porque es la cifra con la que todo el mundo compara, y para que puedas compararla. " +
      "No decide nada: tu fecha y tu capital necesario salen de contar escenarios, no de esta " +
      "multiplicación, y por eso casi nunca coinciden.",
  },

  // --- Jubilación · Riesgo --------------------------------------------------
  "retirement.bands": {
    title: "Escenarios con volatilidad",
    body:
      "Miles de futuros del mismo plan: cada mes el mercado sube o baja según la volatilidad que " +
      "hayas declarado en tus activos. La línea sólida es tu trayectoria central —la que el " +
      "resto de la app enseña como dinero— y crece a la rentabilidad compuesta que declaras en " +
      "cada activo. La franja recoge del escenario 10 al 90: uno de cada diez queda por encima y " +
      "uno de cada diez por debajo. No hay línea de mediana, y sus bordes no son futuros " +
      "concretos: cada mes se ordena por separado. El color no es decorativo — dice qué parte de " +
      "los escenarios ha fallado ya a esa edad. Con ingresos y gastos de por medio, el centro " +
      "del sorteo se separa unos puntos de la línea a veinte o treinta años: es esperable, no un " +
      "descuadre. La semilla es la misma en cada visita, así que las cifras no bailan.",
  },
  "retirement.success": {
    title: "Éxito del plan",
    body:
      "De cada 100 escenarios, en cuántos NO tendrías que volver a trabajar jubilándote en la " +
      "fecha de tu plan: ninguno de los tres fallos ocurre —quedarte sin cartera un mes, " +
      "pasarte del tope de retirada el primer año, o que la regla te deje por debajo de tu gasto " +
      "ordinario—. Es el mismo sorteo con el que se resolvió la fecha, no un segundo cálculo. El " +
      "color no lo pone un listón nuestro: se compara con TU umbral, y contra el suelo del " +
      "intervalo en vez del número grande — verde cuando el suelo ya lo cumple, ámbar cuando el " +
      "umbral cae dentro del intervalo (ni sí ni no), rojo cuando no llega. El «±» de al lado es " +
      "esa incertidumbre, en puntos. Sin volatilidad declarada en tus activos todos los " +
      "escenarios son el mismo y este número no mide nada.",
  },
  "retirement.failure_by_age": {
    title: "Escenarios que fallan por edad",
    body:
      "Qué parte de los escenarios ha fallado YA a esa edad, contando desde el mes en que te " +
      "jubilas. Fallar es tener que volver a trabajar, y pasa por tres motivos: la cartera no " +
      "cubre el gasto de un mes, el primer año de jubilación exige sacar más de lo que tu tope " +
      "permite, o la regla de retirada te deja por debajo de tu gasto ordinario. Es acumulada, " +
      "así que solo puede crecer con la edad: el 8 % a los 80 incluye a los que cayeron a los " +
      "75. Es lo que TIÑE la banda del gráfico, y al pasar el ratón salen el porcentaje exacto y " +
      "por cuál de los tres motivos.",
  },
  "retirement.coverage": {
    title: "Cobertura de tu gasto",
    body:
      "Qué parte de tu gasto de jubilación se pagó DE VERDAD, y en cuántos meses te quedaste por " +
      "debajo. Cuentan las dos formas de quedarse corto: lo que la regla de retirada se negó a " +
      "sacar y lo que la cartera ya no pudo pagar. Arriba se cuenta lo que cubrió tu necesidad, " +
      "nunca lo que sacaste de más: un mes generoso de una regla por saldo no compensa uno " +
      "corto, y por eso la cifra no pasa del 100 %. Son medianas entre escenarios, así que " +
      "describen el caso central: si más de la mitad de tus escenarios aguanta, aquí verás cero " +
      "meses y el gasto entero. Sirven para medir el daño de los que no aguantan — quedarse " +
      "corto un mes ya cuenta como fallo en el éxito de arriba.",
  },

  // --- Retirado en 5.0.0 con el objetivo (modelo v2) -------------------------
  // `retirement.target`, `retirement.crossing_reading`, `retirement.target_basis`,
  // `retirement.bridge_discount`, `retirement.cash_buffer`, `retirement.disposable`,
  // `retirement.coast_number`, `retirement.partial_gap`, `retirement.bridge` y
  // `retirement.depletion_by_age` describían cifras que ya no existen: el capital objetivo dejó
  // de disparar la jubilación (la fecha la decide el éxito), el colchón dejó de ser un ajuste y
  // el agotamiento de cartera pasó a ser uno de los tres fallos. Se BORRAN, no se comentan: un
  // texto sin superficie sigue describiendo una métrica que quizá cambió, y nadie se entera.
  // --- Ajustes · Proyección -------------------------------------------------
  "settings.savings_source": {
    title: "Fuente del ahorro",
    body:
      "Decide de dónde salen el ingreso y el gasto con los que se simula tu futuro: de tu " +
      "presupuesto (lo que planeas), de tus movimientos reales (lo que de verdad pasó), o una " +
      "mezcla con el ingreso del presupuesto y el gasto real. Con movimientos reales, las cuotas " +
      "de préstamos cuentan como un gasto más y, si un lado no tiene datos, ese lado cae al " +
      "presupuesto. La mezcla solo acierta mientras mantengas el presupuesto de ingresos al día. " +
      "El Resumen, la proyección y tu plan de jubilación siguen el modo elegido.",
  },
  "settings.income_window": {
    title: "Ventana de ingreso",
    body:
      "Cuántos meses de movimientos se promedian para estimar tu ingreso. Conviene corta: los " +
      "ingresos cambian a escalones (una subida de sueldo) y una ventana larga tarda meses en " +
      "reflejarla, tirando la estimación hacia abajo mientras tanto.",
  },
  "settings.expense_window": {
    title: "Ventana de gasto",
    body:
      "Cuántos meses de movimientos se promedian para estimar tu gasto. Conviene larga: el gasto " +
      "es irregular mes a mes, y una ventana corta deja que una compra grande redefina lo que la " +
      "simulación cree que gastas habitualmente.",
  },
  "settings.window_mode": {
    title: "Cómo se cuentan los meses",
    body:
      "«Meses de calendario» mira hacia atrás ese número de meses y promedia los que tengan " +
      "datos; si dejaste de importar, la media se apoya en menos meses. «Meses con datos» coge " +
      "siempre ese número de meses con movimientos, saltando los vacíos, aunque para reunirlos " +
      "haya que ir más atrás en el tiempo.",
  },
  "settings.swr": {
    title: "Tasa de retirada (SWR)",
    body:
      "Lo MÁXIMO que te dejas sacar el primer año de jubilación, sobre el patrimonio líquido con " +
      "el que llegues a ella. Es una puerta sobre la fecha, no un límite mes a mes: si el año de " +
      "gasto que necesitas no cabe en ese porcentaje de tu capital, ese escenario cuenta como " +
      "fallido y esa fecha no vale. Bajarla te obliga a llegar con más capital y retrasa tu " +
      "fecha; subirla la adelanta y sube el riesgo, con 6 % como máximo. Es BRUTA: el impuesto " +
      "de la venta va dentro. Y es también la tasa de las reglas que retiran un porcentaje del " +
      "saldo mientras no pongas otra.",
  },
  "settings.taxable_gain": {
    title: "Plusvalía gravable de la retirada",
    body:
      "Qué parte de cada euro vendido es ganancia que tributa (de 0 a 1; de serie, 1 = todo " +
      "tributa, lo más prudente). El mejor dato no es esta perilla: DECLARA el precio de compra " +
      "en cada activo y la simulación derivará su plusvalía real mes a mes al vender — y la " +
      "dejará crecer sola, que es lo que hace de verdad (un número fijo aquí se queda corto con " +
      "los años). Esta fracción gobierna los activos sin coste declarado, el número FIRE clásico " +
      "y el umbral de Autonomía, que dimensionan a perpetuidad — y a perpetuidad casi todo acaba " +
      "siendo ganancia, por eso su valor de serie es 1.",
  },
  "settings.horizon_age": {
    title: "Edad límite del horizonte",
    body:
      "Hasta qué edad simula la proyección (85 a 105; de serie, 90). Subirla revela la cola de " +
      "longevidad: en una pareja de 65, la probabilidad de que al menos uno pase de los 95 ronda " +
      "el 25 %, y un plan que «llega justo» a los 90 puede no llegar a los 95. Dos avisos: el " +
      "horizonte total está topado a 70 años vista (si eres joven, subir la edad puede no " +
      "cambiar nada), y el margen al final se lee en la propia curva — lo que te queda en el " +
      "último mes, en euros de hoy.",
  },
  "settings.inflation": {
    title: "Inflación anual asumida",
    body:
      "Cuánto encarece cada año lo que compras. La simulación trabaja en euros corrientes y " +
      "reparte la inflación así: tu GASTO sube con ella mes a mes (el de ahora y el de " +
      "jubilación — vivir igual costará más), tus INGRESOS quedan planos a propósito (las " +
      "subidas de sueldo hay que pelearlas, no se regalan en la simulación), y el capital que " +
      "hace falta para sostener ese gasto crece con él. La Autonomía del Resumen infla su gasto " +
      "con la misma regla. Puedes ponerla en negativo (hasta −2 %) para estresar tu plan con " +
      "deflación: entonces el gasto y el capital necesario bajan. El gasto que declaras está " +
      "siempre en euros de HOY; la simulación lo actualiza sola.",
  },

  // --- Movimientos ----------------------------------------------------------
  "expenses.expense_avg": {
    title: "Gasto promedio",
    body:
      "Media de tu gasto en tus últimos meses completos —la ventana elegida arriba, contada " +
      "hacia atrás desde hoy: es la MISMA media que usa la proyección, aunque estés mirando un " +
      "mes antiguo—. El denominador NO son los meses del calendario: son los meses con " +
      "movimientos reales clasificados. Un mes vacío, uno cuyo único contenido son movimientos " +
      "recurrentes, o uno donde todo está aún sin clasificar, queda fuera entero —ni suma ni " +
      "divide—, así que no hunde la media. El mes en curso tampoco entra (está a medias), y las " +
      "transferencias conciliadas quedan fuera, porque mover dinero entre tus cuentas no es " +
      "gasto. Los importes se promedian tal cual, en euros de su fecha: un histórico de hace " +
      "muchos años pesa igual que el reciente, sin ajustar por inflación. El paréntesis dice de " +
      "qué meses sale.",
  },
  "expenses.income_avg": {
    title: "Ingreso promedio",
    body:
      "Media de tus ingresos en tus últimos meses completos, con la misma ventana y el mismo " +
      "denominador que el gasto promedio: solo los meses con movimientos reales clasificados, " +
      "sin el mes en curso y sin las transferencias conciliadas.",
  },
  "expenses.savings": {
    title: "Ahorro",
    body:
      "Lo que te quedó: tus ingresos menos tus gastos en los últimos meses completos, con la " +
      "misma ventana y el mismo denominador que las tarjetas de al lado — solo los meses con " +
      "movimientos reales clasificados, sin el mes en curso y sin las transferencias " +
      "conciliadas. El desglose dice qué hiciste con ese ahorro: cuánto se fue a inversión " +
      "(los movimientos marcados como inversión) y cuánto se quedó en cuenta; si invertiste más " +
      "de lo que ahorraste, la diferencia salió de reservas anteriores y así lo dice. Ojo: no es " +
      "el «Ahorro mensual» del Resumen, que sigue la fuente del ahorro que tengas configurada y " +
      "puede salir de tu presupuesto en vez de tus movimientos.",
  },
  "expenses.savings_rate": {
    title: "Tasa de ahorro",
    body:
      "Qué parte de tus ingresos te queda después de gastar, sobre los mismos meses reales que " +
      "promedian las tarjetas de al lado. Puede ser negativa: si gastaste más de lo que " +
      "ingresaste, no hay tasa positiva que enseñar. No es la tasa de ahorro del Resumen — " +
      "aquella se calcula con el ingreso y el gasto de la fuente del ahorro configurada, no con " +
      "tus movimientos reales, y las dos pueden diferir en varios puntos.",
  },
  "expenses.refunds": {
    title: "Devoluciones",
    body:
      "Los gastos que llegaron con importe positivo: un reembolso, un abono del banco, la parte " +
      "que te devolvió alguien de una compra. Ya están descontadas DENTRO de la categoría de lo " +
      "que compensan, así que la cifra que ves aquí no hay que sumarla ni restarla otra vez. No " +
      "son una categoría aparte y no son un ingreso: si contaran como ingreso, tu ahorro y tu " +
      "tasa de ahorro saldrían inflados.",
  },

  // --- Próximos ------------------------------------------------------------
  "upcoming.inflows": {
    title: "Entradas (suma)",
    body:
      "Suma de tus entradas previstas puntuales: importes totales en euros, sin ventana " +
      "temporal — una del mes que viene y una a diez años suman igual. Las recurrentes no " +
      "entran aquí: son euros al mes y tienen su propia tarjeta.",
  },
  "upcoming.outflows": {
    title: "Salidas (suma)",
    body:
      "Suma de tus salidas previstas puntuales: importes totales en euros, sin ventana " +
      "temporal. Las recurrentes no entran aquí: son euros al mes y tienen su propia tarjeta.",
  },
  "upcoming.net": {
    title: "Neto planificado",
    body:
      "Entradas menos salidas puntuales, en euros totales. No incluye los flujos recurrentes: " +
      "su neto es una cifra al mes y se muestra aparte para no mezclar magnitudes. Los Próximos " +
      "mueven la caja de tu proyección —y con ella tu fecha de jubilación—, pero no cuentan como " +
      "gasto ordinario: la regla de retirada y el tope del primer año se miden sin ellos.",
  },
  "upcoming.recurring_net": {
    title: "Recurrentes (neto /mes)",
    body:
      "Entradas menos salidas de tus flujos recurrentes, en euros AL MES mientras cada periodo " +
      "esté vigente. Es un ritmo mensual, no un total: por eso no se suma con las tarjetas de " +
      "al lado.",
  },
} as const satisfies Record<string, HelpText>;

export type HelpTextId = keyof typeof HELP_TEXTS;

export function helpText(id: HelpTextId): HelpText {
  return HELP_TEXTS[id];
}
