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
      "división simple: mientras se consumen, los líquidos siguen rindiendo su rentabilidad " +
      "esperada, y el gasto con el que se compara crece cada año con la inflación que tengas " +
      "configurada —aquí sí, aunque la simulación de tu patrimonio (la que ves en Proyección) " +
      "mantenga ese mismo gasto congelado en euros de hoy—. Qué gasto se usa depende de la " +
      "fuente del ahorro (Ajustes → Plan): con el presupuesto, el presupuestado; con los " +
      "movimientos reales, tu gasto real promediado. «+100 años» es un suelo, no una medida " +
      "exacta.",
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

  // --- Jubilación -----------------------------------------------------------
  "retirement.target": {
    title: "Patrimonio objetivo",
    body:
      "Lo que necesitas acumular para vivir de tu patrimonio: tu gasto anual en jubilación —de " +
      "tus partidas de presupuesto marcadas para jubilación, sin la cuota de tus préstamos, así " +
      "que no es el mismo total que ves en Presupuesto—, con los impuestos por delante si los " +
      "tienes activados, dividido entre la tasa segura de retirada. Esas partidas van en NETO: " +
      "lo que de verdad gastas o cobras cada mes (una pensión, por ejemplo), nunca la cifra " +
      "bruta antes de impuestos — el cálculo ya lo asume así. La cifra grande está en euros de " +
      "hoy; el paréntesis es ese mismo objetivo llevado al mes del cruce con la inflación " +
      "configurada, lo que de verdad tendrás que haber reunido para entonces.",
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
  "settings.inflation": {
    title: "Inflación anual asumida",
    body:
      "Cuánto encarece cada año lo que compras. La simulación de tu patrimonio trabaja en euros " +
      "corrientes: tu ingreso, tu gasto y tus aportaciones NO suben con la inflación mes a mes " +
      "—reflejan «lo que haces ahora», tal cual—. Dos cifras sí la usan: el objetivo FIRE, que " +
      "crece porque mantener tu nivel de vida costará más, y la Autonomía del Resumen, cuyo " +
      "gasto de comparación también se infla, aunque el patrimonio que proyectas no lo haga.",
  },

  // --- Movimientos ----------------------------------------------------------
  "expenses.expense_avg": {
    title: "Gasto promedio",
    body:
      "Media de tu gasto en la ventana elegida arriba. El denominador NO son los meses del " +
      "calendario: son los meses con movimientos reales. Un mes vacío, o uno cuyo único " +
      "contenido son movimientos recurrentes, queda fuera entero —ni suma ni divide—, así que " +
      "no hunde la media mientras no tengas histórico. Dos cosas más: la ventana NO incluye el " +
      "mes que estás mirando (con junio elegido y 3 meses, promedia marzo, abril y mayo), y las " +
      "transferencias conciliadas quedan fuera, porque mover dinero entre tus cuentas no es " +
      "gasto. El paréntesis dice de qué meses sale.",
  },
  "expenses.income_avg": {
    title: "Ingreso promedio",
    body:
      "Media de tus ingresos en la ventana elegida, con el mismo denominador que el gasto " +
      "promedio: solo los meses con movimientos reales, sin contar el mes que estás mirando y " +
      "sin las transferencias conciliadas.",
  },
  "expenses.savings_transferred": {
    title: "Traspasado a ahorro",
    body:
      "Dinero que moviste a cuentas o productos de ahorro, según los movimientos marcados como " +
      "ahorro. NO es ingresos menos gastos: es solo lo que apartaste de forma explícita, así que " +
      "no tiene por qué coincidir con el ahorro del Resumen. Promediado sobre los mismos meses " +
      "reales que el gasto y el ingreso, y con la misma ventana, que no incluye el mes que " +
      "estás mirando.",
  },
  "expenses.transferred_rate": {
    title: "Porcentaje traspasado",
    body:
      "Qué parte de tus ingresos moviste a ahorro de forma explícita, sobre los mismos meses " +
      "reales que promedian las tarjetas de al lado. Ojo: no es tu tasa de ahorro — esa vive en " +
      "el Resumen y sale de ingresos menos gastos. La ventana no incluye el mes que estás " +
      "mirando.",
  },
} as const satisfies Record<string, HelpText>;

export type HelpTextId = keyof typeof HELP_TEXTS;

export function helpText(id: HelpTextId): HelpText {
  return HELP_TEXTS[id];
}
