/**
 * La tarjeta ANCHA del Resumen (U9, modelo v2): frase · estrategia · «Éxito del plan» ·
 * «Capital necesario hoy» · aviso.
 *
 * Lo que este test protege es sobre todo la **procedencia de cada mitad**: la frase sale de la
 * serie (la única fuente que publica edades, hitos secundarios y las fechas al 100/90) y el
 * estado, el éxito y el capital salen de `summary.plan` (la misma cache de plan que resolvió la
 * fecha). Mezclarlas mal es silencioso: la tarjeta sigue pintándose, con el hito de una
 * simulación y el veredicto de otra.
 */

import { describe, expect, it } from "vitest";
import type { SummaryPlanApi } from "../api/types";
import { formatCurrencyAmount } from "./format";
import { planCardV2, type PlanCardV2Series } from "./plan-card";

const monthLabel = (mi: number) => `M${mi}`;

function plan(over: Partial<SummaryPlanApi> = {}): SummaryPlanApi {
  return {
    strategy: "asap",
    jubilacion_month_index: 144,
    required_savings_monthly: null,
    disposable_monthly: null,
    underfunded: null,
    absent_reason: null,
    success_of_plan: null,
    success_threshold_pct: null,
    safe_date_month_index: 144,
    needed_capital_today: null,
    plan_state: "ready",
    ...over,
  };
}

function series(over: Partial<PlanCardV2Series> = {}): PlanCardV2Series {
  return {
    strategy: "asap",
    jubilacion_month_index: 144,
    jubilacion_age: null,
    partial_retirement_month_index: null,
    pension_start_month_index: null,
    retirement_date_basis: "success_threshold",
    success_threshold_pct: 95,
    safe_date_month_index: 144,
    safe_date_age: null,
    safe_date_at_100_month_index: null,
    safe_date_at_90_month_index: null,
    success_of_plan: 0.95,
    success_wilson_low: 0.951,
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

const card = (
  over: Partial<Parameters<typeof planCardV2>[0]> = {},
): ReturnType<typeof planCardV2> =>
  planCardV2({ monthLabel, targetRetirementAge: null, currencyIso: "EUR", ...over });

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("título y subtítulo", () => {
  it("el título ES la frase del plan", () => {
    expect(card({ series: series({ safe_date_age: 52, jubilacion_age: 52 }) }).title).toBe(
      "Con tu plan te jubilas en M144 (a los 52): aguantan 95 de cada 100 escenarios. " +
        "Al 100 %, nunca; al 90 %, nunca.",
    );
  });

  it("el subtítulo es la estrategia sola cuando no hay hito secundario", () => {
    expect(card({ series: series() }).subtitle).toBe("Cuanto antes (FIRE clásico)");
    expect(
      card({
        series: series({
          strategy: "retire_at_age",
          retirement_date_basis: "target_age",
          jubilacion_age: 55,
        }),
        targetRetirementAge: 55,
      }).subtitle,
    ).toBe("A una edad fija");
  });

  it("«Coast FIRE» añade cuándo dejas de aportar", () => {
    expect(
      card({ series: series({ strategy: "coast", coast_stop_month_index: 84 }) }).subtitle,
    ).toBe("Ahorrar ahora y dejar crecer (Coast FIRE) · dejas de aportar en M84");
  });

  it("«Media jornada» añade desde cuándo", () => {
    expect(
      card({ series: series({ strategy: "partial", partial_start_month_index: 120 }) }).subtitle,
    ).toBe("Jornada reducida (Barista FIRE) · desde M120");
  });

  it("una pensión con fecha se anuncia en cualquier estrategia (el puente ya no es una)", () => {
    expect(
      card({ series: series({ strategy: "asap", pension_start_month_index: 264 }) }).subtitle,
    ).toBe("Cuanto antes (FIRE clásico) · pensión desde M264");
  });

  it("sin estrategia (el agregado del hogar) el subtítulo lo dice, no queda vacío", () => {
    expect(card({ series: series({ strategy: null }) }).subtitle).toBe("Sin estrategia");
  });
});

describe("procedencia: la frase de la serie, el estado del plan", () => {
  it("con las dos fuentes, la FRASE sale de la serie (la única con hitos secundarios y edades)", () => {
    const c = card({
      plan: plan({ strategy: "coast", jubilacion_month_index: 360 }),
      series: series({
        strategy: "coast",
        retirement_date_basis: "target_age",
        coast_stop_month_index: 192,
        jubilacion_month_index: 360,
        jubilacion_age: 55,
      }),
      targetRetirementAge: 55,
    });
    expect(c.title).toBe(
      "Puedes dejar de aportar en M192 y jubilarte a los 55 con 95 de cada 100.",
    );
  });

  it("sin serie, la FORMA CORTA de `plan`: fecha y éxito, sin edad inventada (B5)", () => {
    const c = card({
      plan: plan({
        safe_date_month_index: 144,
        success_of_plan: 0.95,
        success_threshold_pct: 95,
      }),
      targetRetirementAge: 52,
    });
    expect(c.title).toBe("Con tu plan te jubilas en M144: aguantan 95 de cada 100 escenarios.");
    expect(c.title).not.toContain("52");
    expect(c.subtitle).toBe("Cuanto antes (FIRE clásico)");
  });

  it("la forma corta dice «calculando» mientras el nivel 1 del solve corre", () => {
    expect(card({ plan: plan({ plan_state: "pending" }) }).title).toBe("Calculando tu fecha…");
  });

  it("la forma corta con el plan ausente dice POR QUÉ, no un hueco", () => {
    const c = card({
      plan: plan({ plan_state: "absent", absent_reason: "household_aggregate" }),
    });
    expect(c.title).toBe("El hogar no tiene un plan propio");
    expect(c.tone).toBe("warn");
  });

  it("sin ninguna de las dos, frase neutra en ámbar", () => {
    const c = card();
    expect(c.title).toBe("Sin plan que mostrar");
    expect(c.tone).toBe("warn");
  });

  it("un plan con `absent_reason` no aporta estrategia: manda la serie", () => {
    const c = card({
      plan: plan({ absent_reason: "household_aggregate", jubilacion_month_index: null }),
      series: series({ jubilacion_month_index: 60, safe_date_month_index: 60 }),
    });
    expect(c.title).toContain("Con tu plan te jubilas en M60");
  });
});

describe("estado y aviso — la precedencia de siempre, con los literales de v2", () => {
  it("sin avisos no hay fila de aviso y el tono es «ok»", () => {
    const c = card({ plan: plan(), series: series() });
    expect(c.warning).toBeNull();
    expect(c.tone).toBe("ok");
  });

  it("`contribution_underfunded` gana: rojo y enlace a Jubilación", () => {
    const c = card({
      plan: plan({ strategy: "retire_at_age" }),
      series: series({
        strategy: "retire_at_age",
        retirement_date_basis: "target_age",
        jubilacion_age: 55,
        contribution_underfunded: true,
        warnings: ["birth_date_missing"],
      }),
      targetRetirementAge: 55,
    });
    expect(c.tone).toBe("danger");
    expect(c.warning).toEqual({
      text: "Ni ahorrando todo tu sobrante llegas a tu edad objetivo",
      actionLabel: "Revisar tu plan",
      target: "retirement",
    });
  });

  it("`contribution_underfunded: null` NO se colapsa con `false`: nadie lo ha evaluado", () => {
    const c = card({
      plan: plan(),
      series: series({ contribution_underfunded: null, warnings: ["birth_date_missing"] }),
    });
    // El aviso que gana es el que SÍ hay, no un «En plan» inventado.
    expect(c.warning?.target).toBe("account");
    expect(c.tone).toBe("danger");
  });

  it("la fecha de nacimiento gana a la edad objetivo, y en v2 es ROJA (C5: sin ella no hay plan)", () => {
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
    expect(c.tone).toBe("danger");
  });

  it("los tres fallos de solve del modelo v2 también son rojos", () => {
    for (const w of [
      "coast_not_reachable",
      "partial_never_starts",
      "partial_never_fully_retires",
    ]) {
      const c = card({ plan: plan(), series: series({ warnings: [w] }) });
      expect(c.tone, w).toBe("danger");
      expect(c.warning?.target, w).toBe("retirement");
    }
  });

  it("los avisos textuales solo viven en la serie: sin ella, no hay fila", () => {
    expect(card({ plan: plan() }).warning).toBeNull();
  });

  it("un literal desconocido no deja la tarjeta sin estado", () => {
    const c = card({ plan: plan(), series: series({ warnings: ["algo_nuevo_del_servidor"] }) });
    expect(c.warning).toBeNull();
    expect(c.tone).toBe("ok");
  });

  it("`plan_state: pending` es ámbar y sin acción: no hay nada que arreglar todavía", () => {
    const c = card({ plan: plan({ plan_state: "pending" }), series: series() });
    expect(c.tone).toBe("warn");
    expect(c.warning).toBeNull();
  });

  it("el tono de la TARJETA es el del estado, no el de la frase", () => {
    // La frase de «jornada reducida sin jubilación total» es roja por su cuenta; el estado dice
    // «En plan» y no hay aviso — la tarjeta se queda con el estado.
    const c = card({
      series: series({
        strategy: "partial",
        partial_start_month_index: 120,
        jubilacion_month_index: null,
        safe_date_month_index: null,
      }),
    });
    expect(c.title).toContain("no alcanza ninguna fecha válida");
    expect(c.tone).toBe("ok");
  });
});

describe("«Éxito del plan» — se rotula, jamás se recalcula", () => {
  it("sin bloque de éxito no hay KPI (ni un guion, que diría otra cosa)", () => {
    expect(card({ plan: plan(), series: series() }).success).toBeNull();
    expect(card({ series: series() }).success).toBeNull();
  });

  it("el KPI es un PORCENTAJE y su paréntesis lleva el UMBRAL del perfil (C3)", () => {
    const c = card({
      plan: plan({
        success_of_plan: 0.87,
        success_threshold_pct: 95,
        success_verdict: "amber",
      }),
      series: series(),
    });
    expect(c.success).toEqual({
      label: "Éxito del plan",
      value: "87,0 %",
      tone: "warn",
      parenthetical: "de los escenarios aguantan · umbral 95,0 %",
      detail: undefined,
    });
  });

  it("el veredicto del SERVIDOR decide el tono: aquí no se recalcula el semáforo", () => {
    const tone = (verdict: "green" | "amber" | "red") =>
      card({
        plan: plan({ success_of_plan: 0.87, success_threshold_pct: 95, success_verdict: verdict }),
        series: series(),
      }).success?.tone;
    expect(tone("green")).toBe("default");
    expect(tone("amber")).toBe("warn");
    expect(tone("red")).toBe("danger");
  });

  it("`plan_state: pending` no es un guion mudo: dice que está calculando", () => {
    const c = card({ plan: plan({ plan_state: "pending" }), series: series() });
    expect(c.success?.value).toBe("—");
    expect(c.success?.detail).toBe("calculando…");
  });

  it("sin sorteo pero CON razón, el KPI existe y explica el hueco", () => {
    const c = card({
      plan: plan({ success_absent_reason: "bands_unavailable" }),
      series: series(),
    });
    expect(c.success?.value).toBe("—");
    expect(c.success?.detail).toBe("no se pudieron sortear los escenarios");
    expect(c.success?.tone).toBe("default");
  });
});

describe("«Capital necesario hoy» — el segundo KPI del modelo v2", () => {
  it("sale de `plan.needed_capital_today`, con el helper de importes de la casa", () => {
    const c = card({
      plan: plan({ needed_capital_today: "412300" }),
      series: series(),
    });
    expect(c.neededCapital).toEqual({
      label: "Capital necesario hoy",
      value: formatCurrencyAmount("412300", "EUR"),
      tone: "default",
      parenthetical: "el líquido que sostendría tu plan si te jubilaras ya",
    });
  });

  it("sin cifra no se pinta el KPI: un guion se leería como «tu plan no necesita capital»", () => {
    expect(card({ plan: plan(), series: series() }).neededCapital).toBeNull();
    expect(card({ series: series() }).neededCapital).toBeNull();
  });

  it("respeta la divisa de la instalación, no un euro escrito a mano", () => {
    const c = card({
      plan: plan({ needed_capital_today: "412300" }),
      currencyIso: "USD",
    });
    expect(c.neededCapital?.value).toBe(formatCurrencyAmount("412300", "USD"));
  });
});

describe("modo de eje «edades»", () => {
  it("se propaga a la frase: la edad no se dice dos veces", () => {
    const c = card({
      series: series({ jubilacion_age: 52, safe_date_age: 52 }),
      ageMode: "ages",
    });
    expect(c.title).toContain("Con tu plan te jubilas en M144: aguantan 95 de cada 100");
    expect(c.title).not.toContain("(a los 52)");
  });
});
