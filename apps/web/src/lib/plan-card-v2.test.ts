/**
 * La tarjeta ANCHA del Resumen (U9): frase · estrategia · «Éxito del plan» · aviso.
 *
 * Lo que este test protege es sobre todo la **procedencia de cada mitad**: la frase sale de la
 * serie (la única fuente que publica el mes coast, el de media jornada y el de la pensión) y el
 * estado y el éxito salen de `summary.plan` (la fuente canónica desde WP5-2b). Mezclarlas mal es
 * silencioso: la tarjeta sigue pintándose, con el hito de una simulación y el veredicto de otra.
 */

import { describe, expect, it } from "vitest";
import type { SummaryPlanApi } from "../api/types";
import { planCardV2, type PlanCardV2Series } from "./plan-card";

const monthLabel = (mi: number) => `M${mi}`;

function plan(over: Partial<SummaryPlanApi> = {}): SummaryPlanApi {
  return {
    strategy: "asap",
    retirement_trigger: "liquid_crossing",
    jubilacion_month_index: 144,
    required_savings_monthly: null,
    disposable_monthly: null,
    underfunded: null,
    absent_reason: null,
    ...over,
  };
}

function series(over: Partial<PlanCardV2Series> = {}): PlanCardV2Series {
  return {
    strategy: "asap",
    jubilacion_month_index: 144,
    jubilacion_age: null,
    coast_fire_month_index: null,
    partial_retirement_month_index: null,
    pension_start_month_index: null,
    underfunded: null,
    warnings: [],
    ...over,
  };
}

const card = (
  over: Partial<Parameters<typeof planCardV2>[0]> = {},
): ReturnType<typeof planCardV2> =>
  planCardV2({ monthLabel, targetRetirementAge: null, ...over });

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("título y subtítulo", () => {
  it("el título ES la frase del plan", () => {
    expect(card({ series: series({ jubilacion_age: 52 }) }).title).toBe(
      "Te jubilas en M144, a los 52 · dentro de 12 años",
    );
  });

  it("el subtítulo es la estrategia sola cuando no hay hito secundario", () => {
    expect(card({ series: series() }).subtitle).toBe("Cuanto antes (FIRE clásico)");
    expect(
      card({
        series: series({ strategy: "retire_at_age", jubilacion_age: 55 }),
        targetRetirementAge: 55,
      }).subtitle,
    ).toBe("A una edad fija");
  });

  it("«Coast FIRE» añade cuándo dejas de aportar", () => {
    expect(
      card({ series: series({ strategy: "coast", coast_fire_month_index: 84 }) }).subtitle,
    ).toBe("Ahorrar ahora y dejar crecer (Coast FIRE) · dejas de aportar en M84");
  });

  it("«Media jornada» añade desde cuándo", () => {
    expect(
      card({
        series: series({ strategy: "partial", partial_retirement_month_index: 120 }),
      }).subtitle,
    ).toBe("Media jornada · desde M120");
  });

  it("«Puente hasta la pensión» añade cuándo llega la pensión", () => {
    expect(
      card({
        series: series({
          strategy: "pension_bridge",
          jubilacion_month_index: 120,
          pension_start_month_index: 264,
        }),
      }).subtitle,
    ).toBe("Puente hasta la pensión · pensión desde M264");
  });

  it("una pensión con fecha también se anuncia con estrategias que no son el puente", () => {
    expect(
      card({ series: series({ strategy: "asap", pension_start_month_index: 264 }) }).subtitle,
    ).toBe("Cuanto antes (FIRE clásico) · pensión desde M264");
  });

  it("sin estrategia (el agregado del hogar) el subtítulo lo dice, no queda vacío", () => {
    expect(card({ series: series({ strategy: null }) }).subtitle).toBe("Sin estrategia");
  });
});

describe("procedencia: la frase de la serie, el estado del plan", () => {
  it("con las dos fuentes, la FRASE sale de la serie (es la única con hitos secundarios)", () => {
    const c = card({
      plan: plan({ strategy: "coast", jubilacion_month_index: 300 }),
      series: series({
        strategy: "coast",
        coast_fire_month_index: 84,
        jubilacion_month_index: 300,
      }),
      targetRetirementAge: 60,
    });
    expect(c.title).toBe("Puedes dejar de aportar en M84 y jubilarte a los 60");
  });

  it("sin serie, la frase se arma con `plan` y sus hitos secundarios quedan vacíos", () => {
    const c = card({ plan: plan({ jubilacion_month_index: 144 }) });
    expect(c.title).toBe("Te jubilas en M144 · dentro de 12 años");
    expect(c.subtitle).toBe("Cuanto antes (FIRE clásico)");
  });

  it("sin ninguna de las dos, frase neutra en ámbar", () => {
    const c = card();
    expect(c.title).toBe("Sin plan que mostrar");
    expect(c.tone).toBe("warn");
  });

  it("un plan con `absent_reason` no se usa: manda la serie", () => {
    const c = card({
      plan: plan({ absent_reason: "household_aggregate", jubilacion_month_index: null }),
      series: series({ jubilacion_month_index: 60 }),
    });
    expect(c.title).toBe("Te jubilas en M60 · dentro de 5 años");
  });
});

describe("estado y aviso — la precedencia de siempre", () => {
  it("sin avisos no hay fila de aviso y el tono es «ok»", () => {
    const c = card({ plan: plan(), series: series() });
    expect(c.warning).toBeNull();
    expect(c.tone).toBe("ok");
  });

  it("`underfunded` del plan gana: rojo y enlace a Jubilación", () => {
    const c = card({
      plan: plan({ strategy: "retire_at_age", underfunded: true }),
      series: series({ strategy: "retire_at_age", warnings: ["birth_date_missing"] }),
      targetRetirementAge: 55,
    });
    expect(c.tone).toBe("danger");
    expect(c.warning).toEqual({
      text: "Con tu ahorro actual no llegas a tu edad objetivo",
      actionLabel: "Revisar tu plan",
      target: "retirement",
    });
  });

  it("`underfunded: null` NO se colapsa con `false`: nadie ha evaluado, y no se pinta verde falso", () => {
    const c = card({
      plan: plan({ underfunded: null }),
      series: series({ warnings: ["birth_date_missing"] }),
    });
    // El aviso que gana es el hueco de configuración, no un «En plan» inventado.
    expect(c.warning?.target).toBe("account");
    expect(c.tone).toBe("warn");
  });

  it("la fecha de nacimiento gana a la edad objetivo", () => {
    const c = card({
      plan: plan(),
      series: series({
        warnings: ["target_retirement_age_missing", "birth_date_missing"],
      }),
    });
    expect(c.warning).toEqual({
      text: "Falta tu fecha de nacimiento",
      actionLabel: "Tu cuenta",
      target: "account",
    });
  });

  it("los avisos textuales solo viven en la serie: sin ella, no hay fila", () => {
    expect(card({ plan: plan() }).warning).toBeNull();
  });

  it("un literal desconocido no deja la tarjeta sin estado", () => {
    const c = card({ plan: plan(), series: series({ warnings: ["algo_nuevo_del_servidor"] }) });
    expect(c.warning).toBeNull();
    expect(c.tone).toBe("ok");
  });

  it("el tono de la TARJETA es el del estado, no el de la frase", () => {
    // La frase de «media jornada sin jubilación total» es roja por su cuenta; el estado dice
    // «En plan» y no hay aviso — la tarjeta se queda con el estado (D17: manda el plan).
    const c = card({
      series: series({
        strategy: "partial",
        partial_retirement_month_index: 120,
        jubilacion_month_index: null,
      }),
    });
    expect(c.title).toContain("sin jubilación total en el horizonte");
    expect(c.tone).toBe("ok");
  });
});

describe("«Éxito del plan» — se rotula, jamás se recalcula", () => {
  it("sin bloque de éxito no hay KPI (ni un guion, que diría otra cosa)", () => {
    expect(card({ plan: plan(), series: series() }).success).toBeNull();
    expect(card({ series: series() }).success).toBeNull();
  });

  it("con probabilidad, el KPI es una FRASE y lleva el umbral en el paréntesis", () => {
    const c = card({
      plan: plan({
        success_probability: "0.870000",
        success_threshold_pct: 95,
        success_verdict: "amber",
      }),
      series: series(),
    });
    expect(c.success).toEqual({
      label: "Éxito del plan",
      value: "87 de cada 100 escenarios se jubilan y no agotan el capital",
      tone: "warn",
      parenthetical: "umbral 95 %",
      detail: undefined,
    });
  });

  it("el veredicto del SERVIDOR decide el tono: aquí no se recalcula el semáforo", () => {
    const tone = (verdict: "green" | "amber" | "red") =>
      card({
        plan: plan({ success_probability: "0.870000", success_verdict: verdict }),
        series: series(),
      }).success?.tone;
    expect(tone("green")).toBe("default");
    expect(tone("amber")).toBe("warn");
    expect(tone("red")).toBe("danger");
  });

  it("los escenarios que no llegan a jubilarse van al segundo slot", () => {
    const c = card({
      plan: plan({
        success_probability: "0.700000",
        never_retired_probability: "0.120000",
        success_verdict: "red",
      }),
      series: series(),
    });
    expect(c.success?.detail).toBe("12 de cada 100 no llegan a jubilarse");
  });

  it("sin sorteo pero CON razón, el KPI existe y explica el hueco", () => {
    const c = card({
      plan: plan({ success_absent_reason: "bands_unavailable", success_threshold_pct: 95 }),
      series: series(),
    });
    expect(c.success?.value).toBe("—");
    expect(c.success?.detail).toBe("no se pudieron sortear los escenarios");
    expect(c.success?.tone).toBe("default");
  });
});

describe("modo de eje «edades»", () => {
  it("se propaga a la frase: la edad no se dice dos veces", () => {
    const c = card({
      series: series({ jubilacion_age: 52 }),
      ageMode: "ages",
    });
    expect(c.title).toBe("Te jubilas en M144 · dentro de 12 años");
  });
});
