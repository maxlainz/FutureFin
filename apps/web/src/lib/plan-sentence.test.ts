/**
 * La tabla de frases (U7/U9/U10) fijada, con especial insistencia en dos cosas:
 *
 *  * **S8** — la longitud del puente es `pension_start − jubilación`, no «meses desde hoy». El
 *    caso del test es el que se veía mal en pantalla: jubilación a los 60 (mes 120) y pensión a
 *    los 72 (mes 264) son **12 años** de puente, no 22.
 *  * **Cada `null` tiene su frase.** Ninguna ausencia se rellena con un guion ni se cuela como
 *    «0»: «no cruzas el objetivo en el horizonte» es un resultado del plan y hay que leerlo así.
 */

import { describe, expect, it } from "vitest";
import {
  memberPlanSentence,
  planSentence,
  type MemberPlanSentenceMember,
  type PlanSentenceSeries,
} from "./plan-sentence";

/** Rotulador inyectado: el módulo no sabe si el eje va en fechas o en edades. */
const monthLabel = (mi: number) => `M${mi}`;

function series(over: Partial<PlanSentenceSeries> = {}): PlanSentenceSeries {
  return {
    strategy: "asap",
    jubilacion_month_index: null,
    jubilacion_age: null,
    coast_fire_month_index: null,
    partial_retirement_month_index: null,
    pension_start_month_index: null,
    underfunded: null,
    ...over,
  };
}

function sentence(
  over: Partial<PlanSentenceSeries> = {},
  targetRetirementAge: number | null = null,
  ageMode: "dates" | "ages" = "dates",
) {
  return planSentence({
    series: series(over),
    targetRetirementAge,
    monthLabel,
    ageMode,
  });
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("«Cuanto antes» — manda el cruce", () => {
  it("frase completa: mes, edad y plazo", () => {
    const s = sentence({ strategy: "asap", jubilacion_month_index: 199, jubilacion_age: 52 });
    expect(s.text).toBe("Te jubilas en M199, a los 52 · dentro de 16 años y 7 meses");
    expect(s.tone).toBe("ok");
  });

  it("sin fecha de nacimiento la edad se OMITE, no se inventa", () => {
    const s = sentence({ strategy: "asap", jubilacion_month_index: 144 });
    expect(s.text).toBe("Te jubilas en M144 · dentro de 12 años");
    expect(s.parts.retirementAge).toBeNull();
  });

  it("sin cruce en el horizonte lo dice, en rojo", () => {
    const s = sentence({ strategy: "asap" });
    expect(s.text).toBe("No cruzas el objetivo en el horizonte");
    expect(s.tone).toBe("danger");
    expect(s.parts.retirementMonthIndex).toBeNull();
  });

  it("el mes 0 (o anterior) es «ya», no «dentro de 0 meses»", () => {
    expect(sentence({ strategy: "asap", jubilacion_month_index: 0 }).text).toBe(
      "Ya puedes jubilarte: tu patrimonio ya cubre el objetivo",
    );
  });

  it("una estrategia nula (el agregado del hogar) usa la misma lectura", () => {
    const s = sentence({ strategy: null, jubilacion_month_index: 12 });
    expect(s.text).toBe("Te jubilas en M12 · dentro de 1 año");
  });
});

describe("«A una edad fija» — manda la edad", () => {
  it("dice la fecha y la edad", () => {
    const s = sentence(
      { strategy: "retire_at_age", jubilacion_month_index: 240, jubilacion_age: 55 },
      55,
    );
    expect(s.text).toBe("Te jubilas en M240, a los 55");
    expect(s.tone).toBe("ok");
  });

  it("`underfunded` pinta la frase de rojo sin cambiar el hecho: te jubilas igual", () => {
    const s = sentence(
      {
        strategy: "retire_at_age",
        jubilacion_month_index: 240,
        jubilacion_age: 55,
        underfunded: true,
      },
      55,
    );
    expect(s.text).toBe("Te jubilas en M240, a los 55");
    expect(s.tone).toBe("danger");
  });

  it("`underfunded: false` NO es lo mismo que `null`, pero ninguno de los dos alarma", () => {
    expect(
      sentence({ strategy: "retire_at_age", jubilacion_month_index: 1, underfunded: false }, 55)
        .tone,
    ).toBe("ok");
    expect(
      sentence({ strategy: "retire_at_age", jubilacion_month_index: 1, underfunded: null }, 55)
        .tone,
    ).toBe("ok");
  });

  it("sin edad objetivo la frase es el hueco de configuración, en ámbar", () => {
    const s = sentence({ strategy: "retire_at_age", jubilacion_month_index: 240 }, null);
    expect(s.text).toBe("Falta tu edad de jubilación objetivo");
    expect(s.tone).toBe("warn");
  });

  it("la edad GUARDADA respalda a la calculada cuando no hay fecha de nacimiento", () => {
    const s = sentence({ strategy: "retire_at_age", jubilacion_month_index: 240 }, 58);
    expect(s.text).toBe("Te jubilas en M240, a los 58");
  });

  it("sin mes de jubilación queda la edad, que sigue siendo el plan pedido", () => {
    expect(sentence({ strategy: "retire_at_age" }, 55).text).toBe("Te jubilas a los 55");
  });
});

describe("«Coast FIRE» — manda el mes coast", () => {
  it("dice cuándo puedes dejar de aportar y a qué edad te jubilas", () => {
    const s = sentence(
      { strategy: "coast", coast_fire_month_index: 84, jubilacion_month_index: 300 },
      60,
    );
    expect(s.text).toBe("Puedes dejar de aportar en M84 y jubilarte a los 60");
    expect(s.tone).toBe("ok");
  });

  it("sin mes coast NO falta un dato: no se llega ni aportando siempre (ámbar)", () => {
    const s = sentence({ strategy: "coast", jubilacion_month_index: 300 }, 60);
    expect(s.text).toBe(
      "No hay mes coast: ni aportando todos los meses llegas al objetivo en tu edad",
    );
    expect(s.tone).toBe("warn");
  });

  it("mes coast 0 o anterior: ya puedes", () => {
    expect(
      sentence({ strategy: "coast", coast_fire_month_index: 0, jubilacion_month_index: 200 }, 60)
        .text,
    ).toBe("Ya puedes dejar de aportar y jubilarte a los 60");
  });

  it("sin edad objetivo cae a la fecha de jubilación", () => {
    expect(
      sentence({ strategy: "coast", coast_fire_month_index: 84, jubilacion_month_index: 300 })
        .text,
    ).toBe("Puedes dejar de aportar en M84 y jubilarte en M300");
  });
});

describe("«Media jornada» — dos hitos en una frase", () => {
  it("fase parcial y jubilación total", () => {
    const s = sentence({
      strategy: "partial",
      partial_retirement_month_index: 120,
      jubilacion_month_index: 264,
    });
    expect(s.text).toBe("Media jornada desde M120; jubilación total en M264");
    expect(s.tone).toBe("ok");
  });

  it("`underfunded` la pone en rojo", () => {
    const s = sentence({
      strategy: "partial",
      partial_retirement_month_index: 120,
      jubilacion_month_index: 264,
      underfunded: true,
    });
    expect(s.tone).toBe("danger");
  });

  it("sin jubilación total dentro del horizonte, rojo y dicho con todas las letras", () => {
    const s = sentence({ strategy: "partial", partial_retirement_month_index: 120 });
    expect(s.text).toBe("Media jornada desde M120; sin jubilación total en el horizonte");
    expect(s.tone).toBe("danger");
  });

  it("sin fase parcial (la jubilación total se la come) lo dice y no la finge", () => {
    const s = sentence({ strategy: "partial", jubilacion_month_index: 90 });
    expect(s.text).toBe("Sin fase de media jornada; te jubilas en M90");
  });

  it("sin ninguno de los dos, la frase de ausencia", () => {
    const s = sentence({ strategy: "partial" });
    expect(s.text).toBe("No cruzas el objetivo en el horizonte");
    expect(s.tone).toBe("danger");
  });
});

describe("«Puente hasta la pensión» — S8", () => {
  it("la longitud del puente es jubilación→pensión, NO meses desde hoy", () => {
    // Jubilación en el mes 120 (a los 60) y pensión en el 264 (a los 72): 144 meses = 12 años.
    const s = sentence({
      strategy: "pension_bridge",
      jubilacion_month_index: 120,
      jubilacion_age: 60,
      pension_start_month_index: 264,
    });
    expect(s.text).toBe(
      "Te jubilas en M120, a los 60 y vives del capital 12 años hasta la pensión (M264)",
    );
    expect(s.parts.bridgeMonths).toBe(144);
    // El bug: contar desde hoy daría 22 años, y la frase habría sonado igual de creíble.
    expect(s.text).not.toContain("22 años");
  });

  it("con la pensión ya en marcha al jubilarse no hay puente que contar", () => {
    const s = sentence({
      strategy: "pension_bridge",
      jubilacion_month_index: 200,
      pension_start_month_index: 150,
    });
    expect(s.text).toBe("Te jubilas en M200 con la pensión ya en marcha (M150)");
    expect(s.parts.bridgeMonths).toBe(-50);
  });

  it("sin pensión declarada, la estrategia lo pide", () => {
    const s = sentence({ strategy: "pension_bridge", jubilacion_month_index: 120 });
    expect(s.text).toBe("Te jubilas en M120; falta declarar tu pensión");
    expect(s.tone).toBe("warn");
  });

  it("sin cruce en el horizonte, rojo", () => {
    const s = sentence({ strategy: "pension_bridge", pension_start_month_index: 264 });
    expect(s.text).toBe("No cruzas el objetivo en el horizonte");
    expect(s.tone).toBe("danger");
    expect(s.parts.bridgeMonths).toBeNull();
  });
});

describe("modo de eje «edades» — la edad no se dice dos veces", () => {
  it("con el rótulo ya en edades, la coletilla «, a los N» desaparece", () => {
    const s = sentence(
      { strategy: "asap", jubilacion_month_index: 144, jubilacion_age: 52 },
      null,
      "ages",
    );
    expect(s.text).toBe("Te jubilas en M144 · dentro de 12 años");
    // La edad sigue publicada en `parts` para quien la quiera.
    expect(s.parts.retirementAge).toBe(52);
  });

  it("en «A una edad fija» la edad SÍ se mantiene: es el disparador, no una etiqueta del eje", () => {
    const s = sentence(
      { strategy: "retire_at_age", jubilacion_month_index: 240, jubilacion_age: 55 },
      55,
      "ages",
    );
    expect(s.text).toBe("Te jubilas en M240, a los 55");
  });
});

describe("`parts` — las piezas se publican para no recalcularlas en la vista", () => {
  it("el hito secundario de cada estrategia va etiquetado con su tipo", () => {
    expect(
      sentence({ strategy: "coast", coast_fire_month_index: 84 }).parts.secondaryKind,
    ).toBe("coast");
    expect(
      sentence({ strategy: "partial", partial_retirement_month_index: 120 }).parts.secondaryKind,
    ).toBe("partial");
    expect(
      sentence({ strategy: "asap", pension_start_month_index: 264 }).parts.secondaryKind,
    ).toBe("pension");
    expect(sentence({ strategy: "asap" }).parts.secondaryKind).toBeNull();
  });

  it("el hito secundario trae su mes Y su rótulo ya resuelto", () => {
    const p = sentence({ strategy: "coast", coast_fire_month_index: 84 }).parts;
    expect(p.secondaryMonthIndex).toBe(84);
    expect(p.secondaryLabel).toBe("M84");
  });

  it("el plazo hasta la jubilación nunca es negativo", () => {
    expect(sentence({ jubilacion_month_index: -3 }).parts.monthsToRetirement).toBe(0);
  });

  it("una serie ausente no revienta: frase neutra y piezas vacías", () => {
    const s = planSentence({
      series: null,
      targetRetirementAge: null,
      monthLabel,
    });
    expect(s.text).toBe("Sin plan que mostrar");
    expect(s.tone).toBe("warn");
    expect(s.parts.retirementMonthIndex).toBeNull();
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("memberPlanSentence — tercera persona para el hogar (U10)", () => {
  function member(over: Partial<MemberPlanSentenceMember> = {}): MemberPlanSentenceMember {
    return {
      username: "Max",
      jubilacion_month_index: null,
      partial_retirement_month_index: null,
      ...over,
    };
  }

  it("el caso de U10: un plazo en años, sin fecha", () => {
    expect(memberPlanSentence(member({ jubilacion_month_index: 144 }), monthLabel)).toBe(
      "Max se quiere jubilar en 12 años.",
    );
  });

  it("con media jornada, la segunda mitad de la frase", () => {
    expect(
      memberPlanSentence(
        member({
          username: "Mariona",
          jubilacion_month_index: 216,
          partial_retirement_month_index: 120,
        }),
        monthLabel,
      ),
    ).toBe("Mariona se quiere jubilar en 18 años y hacer media jornada a partir de M120.");
  });

  it("sin cruce en el horizonte lo dice, y no lo disfraza de plazo", () => {
    expect(memberPlanSentence(member({ username: "Ada" }), monthLabel)).toBe(
      "Ada no cruza el objetivo en el horizonte.",
    );
  });

  it("sin cruce pero con media jornada, las dos mitades y su relación", () => {
    expect(
      memberPlanSentence(
        member({ username: "Ada", partial_retirement_month_index: 60 }),
        monthLabel,
      ),
    ).toBe("Ada no cruza el objetivo en el horizonte, pero hará media jornada a partir de M60.");
  });

  it("quien ya puede jubilarse no espera «0 meses»", () => {
    expect(memberPlanSentence(member({ jubilacion_month_index: 0 }), monthLabel)).toBe(
      "Max ya se puede jubilar.",
    );
  });

  it("un nombre vacío no deja la frase sin sujeto", () => {
    expect(
      memberPlanSentence(member({ username: "  ", jubilacion_month_index: 12 }), monthLabel),
    ).toBe("Esta persona se quiere jubilar en 1 año.");
  });
});
