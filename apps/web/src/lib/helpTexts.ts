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
      "esperada y el gasto crece con la inflación que tengas configurada. Qué gasto se usa " +
      "depende de la fuente del ahorro (Ajustes → Plan): con el presupuesto, el presupuestado; " +
      "con los movimientos reales, tu gasto real promediado. «+100 años» es un suelo, no una " +
      "medida exacta.",
  },
  "summary.net_worth": {
    title: "Patrimonio neto",
    body:
      "Activos totales menos pasivos pendientes. Los préstamos ya vencidos no se cuentan, aunque " +
      "siguen guardados en tu historial.",
  },

  // --- Jubilación -----------------------------------------------------------
  "retirement.target": {
    title: "Patrimonio objetivo",
    body:
      "Lo que necesitas acumular para vivir de tu patrimonio: tu gasto anual en jubilación, " +
      "con los impuestos por delante si los tienes activados, dividido entre la tasa segura de " +
      "retirada. La cifra grande está en euros de hoy. El paréntesis es ese mismo objetivo " +
      "llevado al mes del cruce con la inflación que tengas configurada, que es lo que de " +
      "verdad tendrás que haber reunido para entonces.",
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
      "Cuánto encarece cada año lo que compras. La simulación trabaja en euros corrientes: no " +
      "encoge tu patrimonio, pero sí hace crecer el objetivo FIRE, porque mantener tu nivel de " +
      "vida costará más.",
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
