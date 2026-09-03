/**
 * Tarjeta «Tu plan» (D27) — estado e hito.
 *
 * El valor de estos tests está en la PRECEDENCIA: `warnings[]` es un array, puede traer más de un
 * literal a la vez y la tarjeta enseña UNA línea. Sin un test, el día que se añada el cuarto aviso
 * la regla se rompe en silencio y el usuario ve «Falta tu fecha de nacimiento» donde debería ver
 * el rojo de «no llegas».
 */

import { describe, expect, it } from "vitest";
import type { SummaryPlanApi } from "../api/types";
import {
  ownPlanCard,
  planMilestone,
  planStatusFromPlan,
  planStatusFromWarnings,
  resolvePlanMilestoneCivil,
  type PlanSeriesFallback,
} from "./plan-card";

describe("planStatusFromWarnings", () => {
  it("sin avisos, el plan está en plan y no ofrece acción", () => {
    const s = planStatusFromWarnings([]);
    expect(s).toEqual({
      warning: null,
      tone: "ok",
      label: "En plan",
      action: null,
    });
    expect(planStatusFromWarnings(undefined).tone).toBe("ok");
    expect(planStatusFromWarnings(null).tone).toBe("ok");
  });

  it("falta la fecha de nacimiento ⇒ aviso con enlace a Tu cuenta", () => {
    const s = planStatusFromWarnings(["birth_date_missing"]);
    expect(s.warning).toBe("birth_date_missing");
    expect(s.tone).toBe("warn");
    expect(s.label).toBe("Falta tu fecha de nacimiento");
    expect(s.action).toEqual({ label: "Tu cuenta", target: "account" });
  });

  it("falta la edad objetivo ⇒ aviso con enlace a Jubilación", () => {
    const s = planStatusFromWarnings(["target_retirement_age_missing"]);
    expect(s.tone).toBe("warn");
    expect(s.action?.target).toBe("retirement");
  });

  it("infra-financiado gana a todo y va en ROJO (D17)", () => {
    // El motor aún no emite este literal (llega con los solves): el mapeo existe hoy para que
    // el día que llegue no se pinte como nada.
    const s = planStatusFromWarnings([
      "birth_date_missing",
      "retire_at_age_underfunded",
      "target_retirement_age_missing",
    ]);
    expect(s.warning).toBe("retire_at_age_underfunded");
    expect(s.tone).toBe("danger");
    expect(s.action?.target).toBe("retirement");
  });

  it("entre los dos avisos de dato ausente manda la fecha de nacimiento", () => {
    const s = planStatusFromWarnings([
      "target_retirement_age_missing",
      "birth_date_missing",
    ]);
    expect(s.warning).toBe("birth_date_missing");
  });

  it("un literal desconocido no deja la tarjeta sin estado", () => {
    const s = planStatusFromWarnings(["algo_que_no_existe_todavia"]);
    expect(s.tone).toBe("ok");
    expect(s.label).toBe("En plan");
  });
});

describe("planMilestone", () => {
  it("con jubilación, la fecha civil del servidor y la edad como apoyo", () => {
    expect(
      planMilestone({
        jubilacionMonthIndex: 235,
        jubilacionDateYmd: "2045-11-17",
        jubilacionAge: 58,
      }),
    ).toEqual({ value: "17/11/2045", detail: "a los 58 años", reached: true });
  });

  it("sin edad resoluble, el hito sigue siendo la fecha y el detalle queda vacío", () => {
    const m = planMilestone({
      jubilacionMonthIndex: 235,
      jubilacionDateYmd: "2045-11-17",
      jubilacionAge: null,
    });
    expect(m.detail).toBeNull();
    expect(m.reached).toBe(true);
  });

  it("mes sin fecha civil (agregado del hogar) cae al número de mes, no inventa una fecha", () => {
    expect(
      planMilestone({ jubilacionMonthIndex: 200, jubilacionDateYmd: null }).value,
    ).toBe("Mes 200");
  });

  it("null SIN razón = hay plan y no se jubila dentro del horizonte", () => {
    const m = planMilestone({ jubilacionMonthIndex: null });
    expect(m).toEqual({
      value: "Sin cruce en el horizonte",
      detail: null,
      reached: false,
    });
  });

  it("null CON razón = la pregunta no aplica en esta respuesta, y se dice cuál", () => {
    expect(
      planMilestone({
        jubilacionMonthIndex: null,
        jubilacionAbsentReason: "no_retirement_trigger",
      }).value,
    ).toBe("Sin objetivo ni edad objetivo: este plan no se jubila");
    expect(
      planMilestone({
        jubilacionMonthIndex: null,
        jubilacionAbsentReason: "household_aggregate",
      }).value,
    ).toBe("El hogar suma varios planes: mira la tarjeta de cada persona");
  });

  it("una razón desconocida no deja la tarjeta en blanco", () => {
    const m = planMilestone({
      jubilacionMonthIndex: null,
      jubilacionAbsentReason: "motivo_futuro",
    });
    expect(m.value).toBe("Sin hito que mostrar");
    expect(m.reached).toBe(false);
  });

  it("el mes 0 es «ya jubilado», no un hueco (0 es falsy en JS)", () => {
    const m = planMilestone({
      jubilacionMonthIndex: 0,
      jubilacionDateYmd: "2026-09-03",
      jubilacionAge: 39,
    });
    expect(m.reached).toBe(true);
    expect(m.value).toBe("03/09/2026");
  });
});

// ── 5.0.0 WP5-2b · la tarjeta lee `summary.plan` ───────────────────────────────────────────
//
// El objeto `plan` del Resumen NO trae `warnings[]`: el rojo viaja como el booleano
// `underfunded`, y la fecha del hito hay que resolverla del índice de mes con el ancla de la
// proyección. Las dos cosas son formas nuevas de romper la tarjeta en silencio — un plan que no
// llega pintado de verde, y una fecha inventada un mes por encima o por debajo.

const PLAN: SummaryPlanApi = {
  strategy: "retire_at_age",
  retirement_trigger: "target_age",
  jubilacion_month_index: 12,
  required_savings_monthly: "1200.0000",
  disposable_monthly: "600.0000",
  underfunded: false,
  absent_reason: null,
};

const SERIES: PlanSeriesFallback = {
  strategy: "asap",
  jubilacion_month_index: 235,
  jubilacion_date_ymd: "2046-04-01",
  jubilacion_age: 58,
  jubilacion_absent_reason: null,
  warnings: [],
};

describe("planStatusFromPlan", () => {
  it("`underfunded: true` es el rojo aunque no llegue ningún aviso", () => {
    const s = planStatusFromPlan({ underfunded: true });
    expect(s.tone).toBe("danger");
    expect(s.warning).toBe("retire_at_age_underfunded");
  });

  it("`underfunded: null` NO es «va bien» ni «va mal»: la pregunta no aplica", () => {
    expect(planStatusFromPlan({ underfunded: null }).tone).toBe("ok");
    expect(planStatusFromPlan({ underfunded: false }).tone).toBe("ok");
  });

  it("con avisos y sin booleano sigue valiendo la precedencia de siempre", () => {
    expect(
      planStatusFromPlan({ underfunded: null, warnings: ["birth_date_missing"] })
        .warning,
    ).toBe("birth_date_missing");
  });
});

describe("resolvePlanMilestoneCivil", () => {
  it("el índice de mes se fecha con el ANCLA de la proyección (mes 0 = ancla)", () => {
    const r = resolvePlanMilestoneCivil({
      monthIndex: 12,
      anchorDateYmd: "2026-09-03",
      birthDateIso: "1986-05-10",
    });
    expect(r.ymd).toBe("2027-09-03");
    expect(r.age).toBe(41);
  });

  it("el mes 0 es hoy, no un hueco", () => {
    expect(
      resolvePlanMilestoneCivil({ monthIndex: 0, anchorDateYmd: "2026-09-03" }).ymd,
    ).toBe("2026-09-03");
  });

  it("sin ancla no se inventa una fecha", () => {
    expect(resolvePlanMilestoneCivil({ monthIndex: 12 })).toEqual({
      ymd: null,
      age: null,
    });
  });

  it("sin fecha de nacimiento hay fecha pero no edad", () => {
    const r = resolvePlanMilestoneCivil({
      monthIndex: 6,
      anchorDateYmd: "2026-09-03",
    });
    expect(r.ymd).toBe("2027-03-03");
    expect(r.age).toBeNull();
  });
});

describe("ownPlanCard", () => {
  it("lee el plan del Resumen y copia sus dos cifras sin tocarlas", () => {
    const c = ownPlanCard({
      plan: PLAN,
      series: SERIES,
      anchorDateYmd: "2026-09-03",
      birthDateIso: "1986-05-10",
    })!;
    expect(c.strategy).toBe("retire_at_age");
    expect(c.milestone.value).toBe("03/09/2027");
    expect(c.milestone.detail).toBe("a los 41 años");
    expect(c.figures).toEqual({
      requiredSavingsMonthly: "1200.0000",
      disposableMonthly: "600.0000",
    });
    // La serie está cargada y dice otra cosa: el plan MANDA, no se mezclan las dos mitades.
    expect(c.strategy).not.toBe(SERIES.strategy);
  });

  it("`underfunded: true` pone la tarjeta en rojo", () => {
    const c = ownPlanCard({
      plan: { ...PLAN, underfunded: true },
      anchorDateYmd: "2026-09-03",
    })!;
    expect(c.status.tone).toBe("danger");
  });

  it("con `absent_reason` cae a la serie, entera (estrategia, hito y estado)", () => {
    const c = ownPlanCard({
      plan: { ...PLAN, absent_reason: "projection_unavailable" },
      series: SERIES,
      anchorDateYmd: "2026-09-03",
    })!;
    expect(c.strategy).toBe("asap");
    expect(c.milestone.value).toBe("01/04/2046");
    expect(c.milestone.detail).toBe("a los 58 años");
    // El respaldo no publica cifras: la serie del chart no es la fuente de esas dos.
    expect(c.figures).toBeUndefined();
  });

  it("sin `plan` (backend anterior a WP5-2b) también cae a la serie", () => {
    expect(ownPlanCard({ series: SERIES })!.strategy).toBe("asap");
  });

  it("sin plan y sin serie no hay tarjeta que pintar", () => {
    expect(ownPlanCard({})).toBeNull();
  });

  it("un plan sin jubilación en el horizonte lo DICE, no lo esconde", () => {
    const c = ownPlanCard({
      plan: { ...PLAN, jubilacion_month_index: null },
      anchorDateYmd: "2026-09-03",
    })!;
    expect(c.milestone.reached).toBe(false);
    expect(c.milestone.value).toBe("Sin cruce en el horizonte");
  });

  it("una estrategia por cruce no publica cifras: `null`, no ceros", () => {
    const c = ownPlanCard({
      plan: {
        ...PLAN,
        strategy: "asap",
        retirement_trigger: "liquid_crossing",
        required_savings_monthly: null,
        disposable_monthly: null,
        underfunded: null,
      },
      anchorDateYmd: "2026-09-03",
    })!;
    expect(c.figures).toEqual({
      requiredSavingsMonthly: null,
      disposableMonthly: null,
    });
    expect(c.status.tone).toBe("ok");
  });
});
