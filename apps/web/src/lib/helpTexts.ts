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
      "La tasa con la que la simulación hace crecer este activo cada año. Es NOMINAL —sin " +
      "descontar la inflación, que se aplica aparte— y es la que tu fondo o cuenta YA publica, " +
      "neta de sus comisiones: no le restes nada tú. Si metes la rentabilidad «real» (la ya " +
      "descontada de inflación) en vez de la nominal, tu fecha de cruce con el objetivo FIRE se " +
      "retrasa sin que sepas por qué.",
  },
  "assets.volatility": {
    title: "Volatilidad anual",
    body:
      "Cuánto se mueve este activo arriba y abajo en un año: la desviación típica de su " +
      "rentabilidad, no una pérdida esperada. Como orientación, la renta variable global ronda " +
      "el 15–18 %, la renta fija el 4–6 % y el efectivo es 0. Son cifras de referencia, no un " +
      "dato de tu cartera: pon la de tu fondo si la conoces. Solo alimenta las bandas de " +
      "escenarios; la proyección de siempre —la línea— no la usa. Dejarla vacía significa " +
      "«este activo crece sin sobresaltos».",
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
      "Tu estrategia de jubilación y adónde te lleva: la fecha en la que la simulación te jubila " +
      "de verdad y la edad que tendrás entonces. Con «Cuanto antes» esa fecha es el mes en que tu " +
      "patrimonio líquido cubre el objetivo; con las estrategias por edad es la edad que elegiste, " +
      "llegue o no el capital. El estado avisa cuando el plan corre con algo distinto de lo que " +
      "configuraste: sin tu fecha de nacimiento las estrategias por edad se simulan como «Cuanto " +
      "antes», y en rojo cuando tu ahorro actual no basta para llegar a esa edad. En la vista del " +
      "hogar hay una tarjeta por persona: cada una lleva su propio plan.",
  },

  // --- Jubilación -----------------------------------------------------------
  "retirement.target": {
    title: "Patrimonio objetivo",
    body:
      "Lo que necesitas acumular para vivir de tu patrimonio: tu gasto anual en jubilación —de " +
      "tus partidas de presupuesto marcadas para jubilación, con los impuestos por delante si " +
      "los tienes activados— dividido entre la tasa segura de retirada, MÁS cada euro de cuota " +
      "que te quede por pagar de tus préstamos (tus activos deben cubrir la renta perpetua y " +
      "además terminar de pagar la deuda; por eso el objetivo baja según amortizas y deja de " +
      "moverse cuando el préstamo muere). Esas partidas van en NETO: lo que de verdad gastas o " +
      "cobras cada mes, nunca la cifra bruta — el cálculo ya lo asume así. Y solo cuenta tu " +
      "patrimonio LÍQUIDO para cruzarlo: una vivienda no produce retirada mensual, aunque siga " +
      "sumando en tu patrimonio total. La cifra grande está en euros de hoy; el paréntesis es " +
      "ese mismo objetivo llevado al mes del cruce con la inflación configurada. El objetivo se " +
      "calcula y se dibuja siempre, pero solo DECIDE la fecha en «Cuanto antes» y en «Puente " +
      "hasta la pensión»: en las estrategias por edad manda la edad y este número pasa a ser la " +
      "referencia contra la que se lee si llegas o no.",
  },
  "retirement.crossing_reading": {
    title: "Jubilación",
    body:
      "El mes en que la simulación te jubila DE VERDAD: es la fecha que marcan el chart y el " +
      "Resumen, y el primer mes en que dejas de ingresar tu sueldo y empiezas a vivir de tu " +
      "patrimonio. Con «Cuanto antes» y con «Puente hasta la pensión» ese mes es el del cruce con " +
      "tu objetivo. Con las estrategias por edad manda la edad que elegiste, llegue o no el " +
      "capital, y entonces aparece debajo una segunda fecha: el «cruce del objetivo», que es solo " +
      "una LECTURA — cuándo tu patrimonio líquido habría bastado. Si cae después, te jubilas sin " +
      "haber llegado; si cae antes, podrías haberte ido antes. Cuando las dos fechas coinciden se " +
      "muestra una sola: es el mismo hecho.",
  },
  "retirement.strategy": {
    title: "Tu estrategia de jubilación",
    body:
      "Decide QUÉ dispara tu jubilación en la simulación y, con ello, cómo se dimensiona el " +
      "objetivo. «Cuanto antes» se jubila el mes del cruce con el objetivo; las estrategias por " +
      "edad se jubilan en la edad que elijas, llegue o no el capital; «Media jornada» mete una " +
      "fase intermedia; «Puente hasta la pensión» dimensiona el objetivo contando con la " +
      "pensión que declares. Es tuya, no del hogar: cada persona tiene la suya.",
  },
  "retirement.target_age": {
    title: "Edad de jubilación objetivo",
    body:
      "La edad en la que dejas de trabajar en la simulación. En «A una edad fija» y en «Coast " +
      "FIRE» manda la edad: te jubilas ahí aunque el capital no haya llegado, y entonces el " +
      "plan te lo dice en rojo en vez de moverte la fecha. En «Media jornada» es el fin de la " +
      "fase parcial, y es opcional: sin ella, la jubilación total llega por cruce. Necesita tu " +
      "fecha de nacimiento para convertirse en un mes concreto.",
  },
  "retirement.pension": {
    title: "Pensión pública",
    body:
      "Una renta vitalicia con FECHA: importe mensual en euros de hoy y edad a la que empieza a " +
      "cobrarse. No es una partida más del presupuesto — su fecha cambia el objetivo, no solo " +
      "el flujo de caja: los años anteriores hay que cubrirlos entero con capital. Indexada " +
      "sube cada año con tu inflación; sin indexar se queda plana y pierde poder adquisitivo. " +
      "Puede cubrir todo tu gasto o solo una parte: eso lo decide el importe que declares.",
  },
  "retirement.partial": {
    title: "Media jornada",
    body:
      "Una fase intermedia: a partir de la edad que elijas cobras el ingreso mensual que " +
      "declares (en euros de hoy; 0 es un año sabático) en lugar de tu ingreso actual, y el " +
      "hueco hasta tu gasto lo cubre el capital. No tiene fin propio: termina cuando llega la " +
      "jubilación total. El gasto de la fase es el de jubilación salvo que elijas el gasto " +
      "regular de hoy.",
  },
  "retirement.withdrawal_rule": {
    title: "Regla de retirada",
    body:
      "Cuánto sacas de tu patrimonio cada mes una vez jubilado. «Gasto fijo» retira lo que " +
      "necesitas, indexado, sin techo — es lo que hacía FutureFin hasta ahora. «Un % del saldo» " +
      "retira ese porcentaje de tu líquido del mes anterior: nunca se agota, pero tu nivel de " +
      "vida sube y baja con el mercado. «Híbrida» empieza alta y baja al llegar a un saldo. " +
      "«Con bandas» recorta o sube la retirada al salirse de su banda. Los porcentajes son " +
      "BRUTOS: el impuesto de la venta va dentro.",
  },
  "retirement.spend_mode": {
    title: "Cómo se aplica la regla",
    body:
      "Dos lecturas de la misma regla. Como TECHO, retiras lo que necesitas y nunca más de lo " +
      "que la regla permite: si tu gasto cabe, no vendes de más. Como GASTO, retiras lo que dice " +
      "la regla haya o no necesidad, y ese es tu nivel de vida — con un buen año sacas más y " +
      "con uno malo, menos. No cambia el objetivo: cambia lo que la simulación saca cada mes.",
  },
  "retirement.target_basis": {
    title: "Base del objetivo",
    body:
      "Sobre qué se dimensiona el patrimonio objetivo. «Renta perpetua» pide capital para vivir " +
      "de él para siempre y NO cuenta con tu pensión: es la opción conservadora. «Puente hasta " +
      "la pensión» pide capital para cubrir los años que faltan hasta cobrarla, más lo que haga " +
      "falta a perpetuidad para el gasto que la pensión no cubra. Si no eliges, se usa el " +
      "puente cuando hay pensión declarada y la renta perpetua cuando no.",
  },
  "retirement.bridge_discount": {
    title: "Descuento del puente",
    body:
      "Los años de puente no cuestan su suma nominal: ese dinero sigue invertido mientras lo " +
      "gastas. Esto elige con qué tasa se descuenta. Con la rentabilidad esperada de tus " +
      "líquidos (lo habitual) el objetivo baja; con tu tasa segura de retirada baja menos; sin " +
      "descuento el puente cuesta la suma entera, que es lo más prudente. Solo afecta al " +
      "objetivo cuando la base es el puente.",
  },
  "retirement.cash_buffer": {
    title: "Colchón de caja",
    body:
      "Meses de gasto que quieres tener siempre en efectivo para no verte obligado a vender en " +
      "un mal año. Solo actúa en los escenarios con volatilidad: en la proyección de siempre —la " +
      "línea— no cambia nada, porque ahí no hay malos años que esquivar. Vacío significa sin " +
      "colchón declarado.",
  },
  "retirement.success_threshold": {
    title: "Umbral de éxito",
    body:
      "A partir de qué porcentaje de escenarios sin agotar la cartera das el plan por bueno. De " +
      "serie, 95 %. Es tu listón, no una predicción: subirlo no hace tu plan más seguro, solo " +
      "más exigente contigo. Se compara con la probabilidad de éxito de los escenarios con " +
      "volatilidad; sin volatilidad declarada en tus activos no hay nada que comparar.",
  },
  "retirement.disposable": {
    title: "Margen disponible",
    body:
      "Lo que te SOBRA respecto a lo que tu estrategia necesita: cuánto de tu ahorro mensual no " +
      "hace falta para llegar a tu edad objetivo, y cuánto capital llevas por encima del camino " +
      "mínimo. No es tu patrimonio ni tu ahorro: es la diferencia entre lo que tienes y lo que " +
      "el plan exige. Solo existe en las estrategias con una edad objetivo — en «Cuanto antes» " +
      "todo el ahorro va al objetivo por definición. Todavía no se calcula.",
  },

  // --- Ajustes · Proyección -------------------------------------------------
  "settings.savings_source": {
    title: "Fuente del ahorro",
    body:
      "Decide de dónde salen el ingreso y el gasto con los que se simula tu futuro: de tu " +
      "presupuesto (lo que planeas), de tus movimientos reales (lo que de verdad pasó), o una " +
      "mezcla con el ingreso del presupuesto y el gasto real. Con movimientos reales, las cuotas " +
      "de préstamos cuentan como un gasto más y, si un lado no tiene datos, ese lado cae al " +
      "presupuesto. La mezcla solo acierta mientras mantengas el presupuesto de ingresos al día. " +
      "El Resumen, la proyección y el objetivo FIRE siguen el modo elegido.",
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
    title: "Tasa segura de retirada (SWR)",
    body:
      "Porcentaje de tu patrimonio que asumes poder retirar cada año sin agotarlo. Es lo que " +
      "convierte tu gasto anual en el objetivo FIRE: cuanto más baja la tasa, más grande el " +
      "objetivo.",
  },
  "settings.taxable_gain": {
    title: "Plusvalía gravable de la retirada",
    body:
      "Qué parte de cada euro vendido es ganancia que tributa (de 0 a 1; de serie, 1 = todo " +
      "tributa, lo más prudente). El mejor dato no es esta perilla: DECLARA el precio de compra " +
      "en cada activo y la simulación derivará su plusvalía real mes a mes al vender — y la " +
      "dejará crecer sola, que es lo que hace de verdad (un número fijo aquí se queda corto con " +
      "los años). Esta fracción gobierna los activos sin coste declarado, tu objetivo y el " +
      "umbral de Autonomía, que dimensionan a perpetuidad — y a perpetuidad casi todo acaba " +
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
      "subidas de sueldo hay que pelearlas, no se regalan en la simulación), y el objetivo " +
      "FIRE crece para conservar tu poder adquisitivo. La Autonomía del Resumen infla su gasto " +
      "con la misma regla. Puedes ponerla en negativo (hasta −2 %) para estresar tu plan con " +
      "deflación: entonces el gasto y el objetivo bajan. El gasto que declaras está siempre en " +
      "euros de HOY; la simulación lo actualiza sola.",
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
      "mueven la caja de tu proyección, nunca tu objetivo de jubilación.",
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
