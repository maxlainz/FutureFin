/**
 * Tarjeta «Tu plan» (D27) — estado e hito.
 *
 * El valor de estos tests está en la PRECEDENCIA: `warnings[]` es un array, puede traer más de un
 * literal a la vez y la tarjeta enseña UNA línea. Sin un test, el día que se añada el cuarto aviso
 * la regla se rompe en silencio y el usuario ve «Falta tu fecha de nacimiento» donde debería ver
 * el rojo de «no llegas».
 */

import { describe, expect, it } from "vitest";
import { planMilestone, planStatusFromWarnings } from "./plan-card";

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
