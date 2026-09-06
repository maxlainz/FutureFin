/**
 * La tabla de frases del modelo v2 (§4 del documento del modelo), con insistencia en tres cosas:
 *
 *  * **Una por estrategia × estado.** Fecha resuelta / `not_reachable` / bloque ausente son TRES
 *    respuestas distintas del servidor y las tres tienen que leerse distintas; un plan que no
 *    llega no puede parecerse a uno del que no sabemos nada. (Eran cuatro hasta A12: el `pending`
 *    describía un literal que el servidor nunca emitió — el nivel 1 del solve se resuelve en
 *    línea.)
 *  * **B5 — nunca se rotula una edad que el motor no leyó.** Sin fecha de nacimiento no hay plan
 *    (C5) y la frase lo dice; la edad GUARDADA del perfil solo aparece como «lo que pediste»,
 *    jamás pegada a un mes.
 *  * **Cada `null` tiene su frase.** Una fecha al 100 % que no llega es «nunca», no un hueco; y
 *    `not_reachable` publica lo más cerca que se estuvo en vez de callarse.
 */

import { describe, expect, it } from "vitest";
import { formatCurrencyAmount } from "./format";
import {
  memberPlanSentence,
  planSentence,
  type MemberPlanSentenceMember,
  type PlanSentenceSeries,
} from "./plan-sentence";

/** Rotulador inyectado: el módulo no sabe si el eje va en fechas o en edades. */
const monthLabel = (mi: number) => `M${mi}`;
/** Resolutor de edades inyectado (el de la vista usa el calendario del eje). */
const ageAt = (mi: number) => 25 + Math.floor(mi / 12);

function series(over: Partial<PlanSentenceSeries> = {}): PlanSentenceSeries {
  return {
    strategy: "asap",
    jubilacion_month_index: null,
    jubilacion_age: null,
    partial_retirement_month_index: null,
    pension_start_month_index: null,
    retirement_date_basis: "success_threshold",
    success_threshold_pct: 95,
    safe_date_month_index: null,
    safe_date_age: null,
    safe_date_at_100_month_index: null,
    safe_date_at_90_month_index: null,
    success_of_plan: null,
    success_wilson_low: null,
    contribution_required_monthly: null,
    contribution_underfunded: null,
    coast_stop_month_index: null,
    partial_start_month_index: null,
    success_by_retirement_year: null,
    plan_absent_reason: null,
    horizon_lifespan_age: 90,
    warnings: [],
    ...over,
  };
}

function sentence(
  over: Partial<PlanSentenceSeries> = {},
  extra: Partial<Parameters<typeof planSentence>[0]> = {},
) {
  return planSentence({
    series: series(over),
    targetRetirementAge: null,
    monthLabel,
    currencyIso: "EUR",
    ...extra,
  });
}

/** Un plan que cumple su umbral: éxito 0,95 y Wilson por encima del 95 %. */
const meeting = {
  success_of_plan: 0.95,
  success_wilson_low: 0.951,
  success_threshold_pct: 95,
} satisfies Partial<PlanSentenceSeries>;

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("«N de cada 100» — los topes anti-mentira los pone `risk-bands`, no esta frase", () => {
  it("un 0,999 no se redondea a 100: el plan falla en uno de cada mil", () => {
    const s = sentence({
      success_of_plan: 0.999,
      success_wilson_low: 0.99,
      success_threshold_pct: 95,
      jubilacion_month_index: 100,
      safe_date_month_index: 100,
    });
    expect(s.text).toContain("aguantan 99 de cada 100 escenarios");
    expect(s.parts.successOutOfHundred).toBe(99);
  });

  it("solo el 1 EXACTO llega a «100 de cada 100»", () => {
    const s = sentence({
      success_of_plan: 1,
      success_wilson_low: 0.998,
      success_threshold_pct: 100,
      jubilacion_month_index: 100,
      safe_date_month_index: 100,
    });
    expect(s.text).toContain("aguantan 100 de cada 100 escenarios");
  });

  it("sin sorteo la frase no inventa un recuento: dice la fecha y calla el éxito", () => {
    const s = sentence({ jubilacion_month_index: 144, safe_date_month_index: 144 });
    expect(s.text).toBe("Con tu plan te jubilas en M144. Al 100 %, nunca; al 90 %, nunca.");
    expect(s.parts.successOutOfHundred).toBeNull();
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("«Cuanto antes» — la fecha válida y sus dos alternativas", () => {
  it("la frase de §4, tal cual: fecha, edad, éxito y las fechas al 100 y al 90", () => {
    const s = sentence({
      ...meeting,
      strategy: "asap",
      jubilacion_month_index: 199,
      jubilacion_age: 55,
      safe_date_month_index: 199,
      safe_date_age: 55,
      safe_date_at_100_month_index: 295,
      safe_date_at_90_month_index: 163,
    });
    expect(s.text).toBe(
      "Con tu plan te jubilas en M199 (a los 55): aguantan 95 de cada 100 escenarios. " +
        "Al 100 % sería M295; al 90 %, M163.",
    );
    expect(s.tone).toBe("ok");
    expect(s.parts.successOutOfHundred).toBe(95);
  });

  it("una fecha al 100 % que no se alcanza es «nunca», no un hueco", () => {
    const s = sentence({
      ...meeting,
      jubilacion_month_index: 199,
      safe_date_month_index: 199,
      safe_date_at_100_month_index: null,
      safe_date_at_90_month_index: 163,
    });
    expect(s.text).toContain("Al 100 %, nunca; al 90 %, M163.");
  });

  it("sin fecha de nacimiento la edad se OMITE, no se inventa (B5)", () => {
    const s = sentence(
      {
        ...meeting,
        jubilacion_month_index: 144,
        safe_date_month_index: 144,
        safe_date_at_100_month_index: 200,
        safe_date_at_90_month_index: 120,
      },
      { targetRetirementAge: 50 },
    );
    expect(s.text).toBe(
      "Con tu plan te jubilas en M144: aguantan 95 de cada 100 escenarios. " +
        "Al 100 % sería M200; al 90 %, M120.",
    );
    expect(s.text).not.toContain("a los 50");
    expect(s.parts.retirementAge).toBeNull();
  });

  it("el mes 0 (o anterior) es «ya», no «dentro de 0 meses»", () => {
    const s = sentence({
      ...meeting,
      jubilacion_month_index: 0,
      safe_date_month_index: 0,
    });
    expect(s.text).toContain("Con tu plan ya puedes jubilarte: aguantan 95 de cada 100 escenarios.");
  });

  it("una estrategia nula (el agregado del hogar) usa la misma lectura", () => {
    const s = sentence({ ...meeting, strategy: null, jubilacion_month_index: 12 });
    expect(s.text).toContain("Con tu plan te jubilas en M12");
  });

  it("por debajo del umbral la frase no alarma pero avisa: ámbar", () => {
    const s = sentence({
      strategy: "asap",
      success_of_plan: 0.82,
      success_wilson_low: 0.8,
      success_threshold_pct: 95,
      jubilacion_month_index: 199,
      safe_date_month_index: 199,
    });
    expect(s.tone).toBe("warn");
    expect(s.parts.meetsThreshold).toBe(false);
  });

  it("con umbral 100 solo el éxito EXACTAMENTE 1 cumple (C3)", () => {
    const casi = sentence({
      success_of_plan: 0.999,
      success_wilson_low: 0.99,
      success_threshold_pct: 100,
      jubilacion_month_index: 100,
      safe_date_month_index: 100,
    });
    expect(casi.parts.meetsThreshold).toBe(false);
    const cero = sentence({
      success_of_plan: 1,
      success_wilson_low: 0.998,
      success_threshold_pct: 100,
      jubilacion_month_index: 100,
      safe_date_month_index: 100,
    });
    expect(cero.parts.meetsThreshold).toBe(true);
  });
});

describe("estados que ganan a la estrategia", () => {
  it("`not_reachable`: ninguna fecha cumple, y se publica lo más cerca que se estuvo", () => {
    const s = sentence({
      retirement_date_basis: "not_reachable",
      success_threshold_pct: 95,
      horizon_lifespan_age: 90,
      success_by_retirement_year: [
        { month_index: 240, success: 0.6 },
        { month_index: 540, success: 0.78 },
        { month_index: 600, success: 0.71 },
      ],
    });
    expect(s.text).toBe(
      "Con tu plan no hay ninguna fecha en la que aguanten 95 de cada 100 escenarios hasta " +
        "los 90 años. Lo más cerca: M540 con 78 de cada 100.",
    );
    expect(s.tone).toBe("danger");
  });

  it("`not_reachable` sin la tira del nivel 2: la primera oración sola, sin inventar un «lo más cerca»", () => {
    const s = sentence({
      retirement_date_basis: "not_reachable",
      success_by_retirement_year: null,
    });
    expect(s.text).toBe(
      "Con tu plan no hay ninguna fecha en la que aguanten 95 de cada 100 escenarios hasta los 90 años.",
    );
    expect(s.text).not.toContain("Lo más cerca");
  });

  it("`birth_date_missing`: sin fecha de nacimiento NO hay plan, y se dice en rojo (C5/B5)", () => {
    const s = sentence(
      { plan_absent_reason: "birth_date_missing", retirement_date_basis: "not_reachable" },
      { targetRetirementAge: 55 },
    );
    expect(s.text).toBe(
      "Falta tu fecha de nacimiento para situar la pensión y el horizonte: sin ella no hay fecha válida.",
    );
    expect(s.tone).toBe("danger");
    expect(s.text).not.toContain("55");
  });

  it("las otras dos ausencias del bloque «plan» se dicen distintas entre sí", () => {
    expect(sentence({ plan_absent_reason: "household_aggregate" }).text).toContain(
      "El hogar no resuelve una fecha",
    );
    expect(sentence({ plan_absent_reason: "months_override" }).text).toContain(
      "horizonte forzado",
    );
  });

  // A12 — `no_liquid_assets` no es una razón de `plan_absent_reason`: vive en
  // `needed_capital_absent_reason`. Mientras la tabla lo tradujo, esta frase parecía cubierta y no
  // podía dispararse; un literal que la tabla no conoce cae a la genérica, que es lo correcto.
  it("un literal que `plan_absent_reason` NO emite cae a la frase genérica, no a una inventada", () => {
    const s = sentence({ plan_absent_reason: "no_liquid_assets" as never });
    expect(s.text).toBe("Tu plan no tiene fecha válida.");
    expect(s.text).not.toContain("líquidos");
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("«Jubilarme a una edad» — la edad manda y el sorteo la juzga", () => {
  const atAge = {
    strategy: "retire_at_age",
    retirement_date_basis: "target_age",
    jubilacion_month_index: 240,
    jubilacion_age: 55,
    success_threshold_pct: 95,
  } satisfies Partial<PlanSentenceSeries>;

  it("la frase de §4: éxito, umbral entre paréntesis y la aportación que falta", () => {
    const s = sentence(
      {
        ...atAge,
        success_of_plan: 0.82,
        success_wilson_low: 0.8,
        contribution_required_monthly: "300",
      },
      { targetRetirementAge: 55 },
    );
    expect(s.text).toBe(
      "A los 55, como pediste: aguantan 82 de cada 100 escenarios (tu umbral es 95). " +
        `Para llegar harían falta ${formatCurrencyAmount("300", "EUR")} más al mes.`,
    );
    expect(s.tone).toBe("warn");
  });

  it("cuando ya llega, la fecha válida al lado dice cuánto margen hay", () => {
    const s = sentence(
      {
        ...atAge,
        success_of_plan: 0.97,
        success_wilson_low: 0.96,
        safe_date_month_index: 144,
        safe_date_age: 47,
      },
      { targetRetirementAge: 55 },
    );
    expect(s.text).toBe(
      "A los 55, como pediste: aguantan 97 de cada 100 escenarios (tu umbral es 95). " +
        "Ya llegas: podrías incluso a los 47.",
    );
    expect(s.tone).toBe("ok");
  });

  it("`contribution_underfunded` es rojo y no promete un importe que no existe", () => {
    const s = sentence(
      {
        ...atAge,
        success_of_plan: 0.4,
        success_wilson_low: 0.38,
        contribution_required_monthly: "9000",
        contribution_underfunded: true,
      },
      { targetRetirementAge: 55 },
    );
    expect(s.text).toBe(
      "A los 55, como pediste: aguantan 40 de cada 100 escenarios (tu umbral es 95). " +
        "Ni ahorrando todo tu sobrante llegas a los 55.",
    );
    expect(s.tone).toBe("danger");
    expect(s.text).not.toContain("9.000");
  });

  it("sin edad objetivo la frase es el hueco de configuración, en ámbar", () => {
    const s = sentence({
      strategy: "retire_at_age",
      retirement_date_basis: "target_age",
    });
    expect(s.text).toBe("Falta tu edad de jubilación objetivo.");
    expect(s.tone).toBe("warn");
  });

  it("la edad la pone el SERVIDOR; la guardada solo respalda cuando el motor no publicó ninguna", () => {
    const s = sentence(
      {
        strategy: "retire_at_age",
        retirement_date_basis: "target_age",
        success_of_plan: 0.96,
        success_wilson_low: 0.955,
      },
      { targetRetirementAge: 58 },
    );
    expect(s.text).toContain("A los 58, como pediste");
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("«Coast FIRE» — dos modos, dos frases", () => {
  it("modo A (fijo la edad de jubilación): el hito es cuándo puedes dejar de aportar", () => {
    const s = sentence(
      {
        ...meeting,
        strategy: "coast",
        retirement_date_basis: "target_age",
        coast_stop_month_index: 192,
        jubilacion_month_index: 360,
        jubilacion_age: 55,
      },
      { targetRetirementAge: 55, ageAt },
    );
    expect(s.text).toBe(
      "Puedes dejar de aportar en M192 (a los 41) y jubilarte a los 55 con 95 de cada 100.",
    );
    expect(s.tone).toBe("ok");
  });

  it("modo B (fijo la edad de parada): la fecha de jubilación sale del sorteo", () => {
    const s = sentence(
      {
        ...meeting,
        strategy: "coast",
        retirement_date_basis: "success_threshold",
        coast_stop_month_index: 192,
        jubilacion_month_index: 384,
        jubilacion_age: 57,
        safe_date_month_index: 384,
        safe_date_age: 57,
      },
      { ageAt },
    );
    expect(s.text).toBe(
      "Dejando de aportar a los 41, te jubilas en M384 (a los 57) con 95 de cada 100.",
    );
  });

  it("sin resolutor de edades no se estima ninguna: solo la fecha (B5)", () => {
    const s = sentence({
      ...meeting,
      strategy: "coast",
      retirement_date_basis: "success_threshold",
      coast_stop_month_index: 192,
      jubilacion_month_index: 384,
    });
    expect(s.text).toBe("Dejando de aportar en M192, te jubilas en M384 con 95 de cada 100.");
  });

  it("`coast_not_reachable`: no se llega ni aportando siempre, y eso es rojo", () => {
    const s = sentence(
      {
        strategy: "coast",
        retirement_date_basis: "target_age",
        jubilacion_age: 55,
        warnings: ["coast_not_reachable"],
      },
      { targetRetirementAge: 55 },
    );
    expect(s.text).toBe("Ni aportando hasta el final llegas a los 55 con tu umbral.");
    expect(s.tone).toBe("danger");
  });

  it("todavía sin mes coast resuelto: ámbar, y no se finge un «ya puedes»", () => {
    const s = sentence({ strategy: "coast", retirement_date_basis: "target_age" }, {
      targetRetirementAge: 55,
    });
    expect(s.text).toBe("Todavía no hay ningún mes en el que puedas dejar de aportar.");
    expect(s.tone).toBe("warn");
  });

  it("el modo del perfil gana a la deducción por `retirement_date_basis`", () => {
    const s = sentence(
      {
        ...meeting,
        strategy: "coast",
        retirement_date_basis: "target_age",
        coast_stop_month_index: 192,
        jubilacion_month_index: 384,
      },
      { coastMode: "fixed_stop_age", ageAt },
    );
    expect(s.text).toContain("Dejando de aportar a los 41");
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("«Jornada reducida» — dos hitos en una frase", () => {
  it("la frase de §4: cuándo empieza la fase y cuándo te jubilas del todo", () => {
    const s = sentence(
      {
        ...meeting,
        strategy: "partial",
        partial_start_month_index: 192,
        jubilacion_month_index: 360,
        jubilacion_age: 55,
        safe_date_month_index: 360,
        safe_date_age: 55,
      },
      { ageAt },
    );
    expect(s.text).toBe(
      "Puedes pasar a jornada reducida en M192 (a los 41) y jubilarte del todo en M360 (a los 55) con 95 de cada 100.",
    );
    expect(s.tone).toBe("ok");
  });

  it("`partial_never_starts`: el plan no puede permitírsela, en rojo", () => {
    const s = sentence({
      strategy: "partial",
      warnings: ["partial_never_starts"],
    });
    expect(s.text).toBe(
      "Tu plan no puede permitirse la jornada reducida en ningún mes del horizonte.",
    );
    expect(s.tone).toBe("danger");
  });

  it("`partial_never_fully_retires`: empieza la fase y no termina, y se dice con todas las letras", () => {
    const s = sentence(
      {
        strategy: "partial",
        partial_start_month_index: 192,
        warnings: ["partial_never_fully_retires"],
      },
      { ageAt },
    );
    expect(s.text).toBe(
      "Pasas a jornada reducida en M192 (a los 41), pero no llegas a jubilarte del todo dentro del horizonte.",
    );
    expect(s.tone).toBe("danger");
  });

  it("sin fase (la jubilación total se la come) lo dice y no la finge", () => {
    const s = sentence({
      ...meeting,
      strategy: "partial",
      jubilacion_month_index: 90,
      safe_date_month_index: 90,
    });
    expect(s.text).toBe("Sin fase de jornada reducida: te jubilas en M90 con 95 de cada 100.");
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("modo de eje «edades» — la edad no se dice dos veces", () => {
  it("con el rótulo ya en edades, el paréntesis «(a los N)» desaparece", () => {
    const s = sentence(
      {
        ...meeting,
        jubilacion_month_index: 144,
        jubilacion_age: 52,
        safe_date_month_index: 144,
        safe_date_age: 52,
      },
      { ageMode: "ages" },
    );
    expect(s.text).toContain("Con tu plan te jubilas en M144: aguantan 95 de cada 100");
    expect(s.text).not.toContain("(a los 52)");
    // La edad sigue publicada en `parts` para quien la quiera.
    expect(s.parts.retirementAge).toBe(52);
  });
});

describe("`parts` — las piezas se publican para no recalcularlas en la vista", () => {
  it("el hito secundario de cada estrategia va etiquetado con su tipo", () => {
    expect(
      sentence({ strategy: "coast", coast_stop_month_index: 84 }).parts.secondaryKind,
    ).toBe("coast");
    expect(
      sentence({ strategy: "partial", partial_start_month_index: 120 }).parts.secondaryKind,
    ).toBe("partial");
    expect(
      sentence({ strategy: "asap", pension_start_month_index: 264 }).parts.secondaryKind,
    ).toBe("pension");
    expect(sentence({ strategy: "asap" }).parts.secondaryKind).toBeNull();
  });

  it("el hito secundario trae su mes Y su rótulo ya resuelto", () => {
    const p = sentence({ strategy: "coast", coast_stop_month_index: 84 }).parts;
    expect(p.secondaryMonthIndex).toBe(84);
    expect(p.secondaryLabel).toBe("M84");
  });

  it("S8: el puente es jubilación→pensión, NO meses desde hoy", () => {
    // Jubilación en el mes 120 y pensión en el 264: 144 meses = 12 años, no 22.
    const p = sentence({
      jubilacion_month_index: 120,
      safe_date_month_index: 120,
      pension_start_month_index: 264,
    }).parts;
    expect(p.bridgeMonths).toBe(144);
  });

  it("el plazo hasta la jubilación nunca es negativo", () => {
    expect(sentence({ jubilacion_month_index: -3 }).parts.monthsToRetirement).toBe(0);
  });

  it("una serie ausente no revienta: frase neutra y piezas vacías", () => {
    const s = planSentence({
      series: null,
      targetRetirementAge: null,
      monthLabel,
      currencyIso: "EUR",
    });
    expect(s.text).toBe("Sin plan que mostrar");
    expect(s.tone).toBe("warn");
    expect(s.parts.retirementMonthIndex).toBeNull();
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("memberPlanSentence — tercera persona, con estado (U10 + B7)", () => {
  function member(over: Partial<MemberPlanSentenceMember> = {}): MemberPlanSentenceMember {
    return {
      username: "Max",
      strategy: "asap",
      jubilacion_month_index: null,
      jubilacion_age: null,
      partial_retirement_month_index: null,
      warnings: [],
      plan_state: "household_not_solved",
      ...over,
    };
  }

  it("estrategia por edad: la fecha está FIJADA, con el rótulo de producto de su estrategia", () => {
    const s = memberPlanSentence(
      member({ strategy: "retire_at_age", jubilacion_month_index: 420, jubilacion_age: 60 }),
      monthLabel,
    );
    expect(s.text).toBe("Max: A una edad fija — a los 60, M420.");
    expect(s.tone).toBe("ok");
  });

  it("estrategia por umbral: el hogar NO resuelve su fecha, y manda a su vista Jubilación", () => {
    const s = memberPlanSentence(member({ username: "Ada" }), monthLabel);
    expect(s.text).toBe(
      "Ada: Cuanto antes (FIRE clásico) — fecha válida: en su vista Jubilación.",
    );
    expect(s.tone).toBe("ok");
  });

  it("sin fecha de nacimiento, el sufijo lo dice y la línea va en rojo (B7)", () => {
    const s = memberPlanSentence(
      member({ username: "Ada", warnings: ["birth_date_missing"] }),
      monthLabel,
    );
    expect(s.text).toBe(
      "Ada: Cuanto antes (FIRE clásico) — fecha válida: en su vista Jubilación — falta su fecha de nacimiento.",
    );
    expect(s.tone).toBe("danger");
  });

  // El hogar nunca publicó una aportación mínima ni un margen por miembro (D9): no hay un
  // «infra-financiado» que un booleano pueda decidir aquí, así que el único estado que puede
  // pintar de rojo o de ámbar a un miembro es uno de sus `warnings` — la tabla de precedencia de
  // MEMBER_WARNING_SUFFIX sigue viva sin el atajo de `underfunded`.
  it("un aviso de configuración incompleta pinta de ámbar aunque la fecha esté fijada", () => {
    const s = memberPlanSentence(
      member({
        strategy: "retire_at_age",
        jubilacion_month_index: 420,
        jubilacion_age: 60,
        warnings: ["target_retirement_age_missing"],
      }),
      monthLabel,
    );
    expect(s.text).toBe(
      "Max: A una edad fija — a los 60, M420 — falta su edad de jubilación.",
    );
    expect(s.tone).toBe("warn");
  });

  // El mes coast por miembro (`coast_fire_month_index`) ya no viaja: el hogar no resuelve el
  // plan de nadie y ese campo era exactamente una cifra del solve. Solo queda la jornada
  // reducida, que es un hecho determinista.
  it("la jornada reducida añade su hito, sin inventar cifras (el mes coast ya no viaja por miembro)", () => {
    const s = memberPlanSentence(
      member({
        username: "Mariona",
        strategy: "coast",
        jubilacion_month_index: 216,
        jubilacion_age: 58,
        partial_retirement_month_index: 120,
      }),
      monthLabel,
    );
    expect(s.text).toBe(
      "Mariona: Ahorrar ahora y dejar crecer (Coast FIRE) — a los 58, M216 (hace jornada reducida desde M120).",
    );
  });

  it("sin fecha efectiva pero con jornada reducida, la fase sí es un hecho suyo", () => {
    const s = memberPlanSentence(
      member({ username: "Ada", strategy: "partial", partial_retirement_month_index: 60 }),
      monthLabel,
    );
    expect(s.text).toBe(
      "Ada: Jornada reducida (Barista FIRE) — fecha válida: en su vista Jubilación (hace jornada reducida desde M60).",
    );
  });

  it("sin edad publicada se rotula el MES, nunca una edad inventada (B5)", () => {
    const s = memberPlanSentence(
      member({ strategy: "retire_at_age", jubilacion_month_index: 420 }),
      monthLabel,
    );
    expect(s.text).toBe("Max: A una edad fija — M420.");
  });

  it("quien ya puede jubilarse no espera «0 meses»", () => {
    expect(
      memberPlanSentence(
        member({ strategy: "retire_at_age", jubilacion_month_index: 0 }),
        monthLabel,
      ).text,
    ).toBe("Max: A una edad fija — ya puede jubilarse.");
  });

  it("un nombre vacío no deja la frase sin sujeto", () => {
    expect(
      memberPlanSentence(
        member({ username: "  ", strategy: "retire_at_age", jubilacion_month_index: 12, jubilacion_age: 40 }),
        monthLabel,
      ).text,
    ).toBe("Esta persona: A una edad fija — a los 40, M12.");
  });

  // El struct de Rust nunca publicó `coast_fire_month_index` ni `underfunded` para un miembro del
  // hogar (`HouseholdMemberProjection`, `apps/api/src/handlers/projection.rs`); esta frase los leyó
  // de todos modos hasta que W10 encontró el hueco. Si un cliente viejo, o cualquier objeto que
  // no pase por el tipo, todavía trae esas dos claves en el JSON, la frase no puede cambiar por
  // su presencia — el día que alguien las reintroduzca sin querer, este test lo dice.
  it("`coast_fire_month_index`/`underfunded` en el JSON (cliente viejo) no cambian la frase", () => {
    const base = member({
      strategy: "coast",
      jubilacion_month_index: 216,
      jubilacion_age: 58,
      partial_retirement_month_index: 120,
    });
    const withStaleFields = {
      ...base,
      coast_fire_month_index: 96,
      underfunded: true,
    } as MemberPlanSentenceMember;
    expect(memberPlanSentence(withStaleFields, monthLabel)).toEqual(
      memberPlanSentence(base, monthLabel),
    );
  });
});
