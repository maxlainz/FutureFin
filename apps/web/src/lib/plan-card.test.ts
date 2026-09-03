/**
 * El ESTADO del plan (D27) — precedencia de avisos y fecha/edad del hito.
 *
 * El valor de estos tests está en la PRECEDENCIA: `warnings[]` es un array, puede traer más de un
 * literal a la vez y la tarjeta enseña UNA línea. Sin un test, el día que se añada el cuarto aviso
 * la regla se rompe en silencio y el usuario ve «Falta tu fecha de nacimiento» donde debería ver
 * el rojo de «no llegas».
 *
 * El modelo de la tarjeta ANCHA (`planCardV2`, U9) compone estas mismas piezas con la ORACIÓN de
 * `lib/plan-sentence.ts` — sin test propio aquí porque cada mitad (estado, fecha/edad, frase,
 * KPI de éxito) ya está probada por separado y `planCardV2` es solo su ensamblaje.
 */

import { describe, expect, it } from "vitest";
import {
  planStatusFromPlan,
  planStatusFromWarnings,
  resolvePlanMilestoneCivil,
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
