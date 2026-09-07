import { describe, expect, it } from "vitest";

import {
  CURVE_POLL_BASE_DELAY_MS,
  CURVE_POLL_MAX_ATTEMPTS,
  CURVE_POLL_MAX_DELAY_MS,
  NO_LAST_GOOD,
  curvePollDelayMs,
  nextLastGood,
  shouldPollNeededCurve,
  type LastGood,
} from "./stale-data";

type Payload = { readonly n: number };

describe("nextLastGood — la última respuesta buena se sigue pintando", () => {
  it("un dato nuevo manda siempre", () => {
    const a: Payload = { n: 1 };
    const got = nextLastGood<Payload>(NO_LAST_GOOD, a, false);
    expect(got).toEqual({ value: a, refreshing: false });
  });

  it("con carga en vuelo el dato nuevo se pinta y se marca refrescando", () => {
    const a: Payload = { n: 1 };
    // El two-phase fetch de la proyección entrega el `hybrid` con el `monthly` todavía en vuelo:
    // hay dato bueno Y sigue habiendo revalidación.
    expect(nextLastGood<Payload>(NO_LAST_GOOD, a, true)).toEqual({
      value: a,
      refreshing: true,
    });
  });

  it("un null CON carga en vuelo conserva lo último bueno (el bug del parpadeo)", () => {
    const prev: LastGood<Payload> = { value: { n: 1 }, refreshing: false };
    const got = nextLastGood<Payload>(prev, null, true);
    expect(got.value).toBe(prev.value);
    expect(got.refreshing).toBe(true);
  });

  it("un null SIN carga en vuelo suelta el dato: no es una cache", () => {
    const prev: LastGood<Payload> = { value: { n: 1 }, refreshing: true };
    expect(nextLastGood<Payload>(prev, null, false)).toEqual({
      value: null,
      refreshing: false,
    });
  });

  it("sin nada que conservar, la carga inicial se declara refrescando", () => {
    expect(nextLastGood<Payload>(NO_LAST_GOOD, null, true)).toEqual({
      value: null,
      refreshing: true,
    });
  });

  it("`undefined` se trata como ausencia, igual que `null`", () => {
    const prev: LastGood<Payload> = { value: { n: 1 }, refreshing: false };
    expect(nextLastGood<Payload>(prev, undefined, true).value).toBe(prev.value);
  });

  it("devuelve `prev` por IDENTIDAD cuando nada cambia", () => {
    // Lo consumen `useMemo`s que dependen del objeto: una copia nueva por render los
    // invalidaría todos sin que ningún dato se hubiera movido.
    const a: Payload = { n: 1 };
    const settled = nextLastGood<Payload>(NO_LAST_GOOD, a, false);
    expect(nextLastGood<Payload>(settled, a, false)).toBe(settled);

    const holding = nextLastGood<Payload>(settled, null, true);
    expect(nextLastGood<Payload>(holding, null, true)).toBe(holding);

    expect(nextLastGood<Payload>(NO_LAST_GOOD, null, false)).toBe(NO_LAST_GOOD);
  });

  it("es idempotente en los cuatro casos (seguro bajo el doble render de StrictMode)", () => {
    const a: Payload = { n: 1 };
    const prevs: LastGood<Payload>[] = [
      NO_LAST_GOOD,
      { value: { n: 9 }, refreshing: false },
      { value: { n: 9 }, refreshing: true },
    ];
    for (const prev of prevs) {
      for (const incoming of [a, null]) {
        for (const busy of [true, false]) {
          const once = nextLastGood<Payload>(prev, incoming, busy);
          const twice = nextLastGood<Payload>(once, incoming, busy);
          expect(twice).toEqual(once);
        }
      }
    }
  });

  it("secuencia real de un autosave: guardar → refetch → respuesta, sin un solo hueco", () => {
    const first: Payload = { n: 1 };
    const second: Payload = { n: 2 };
    let s = nextLastGood<Payload>(NO_LAST_GOOD, first, false);
    // PATCH del perfil: arranca el refetch y la vista todavía no tiene nada nuevo.
    s = nextLastGood<Payload>(s, first, true);
    expect(s.value).toBe(first);
    // Fase 1 (hybrid) llega; el monthly sigue en vuelo.
    s = nextLastGood<Payload>(s, second, true);
    expect(s.value).toBe(second);
    // Fase 2 aterriza y se apaga la carga.
    s = nextLastGood<Payload>(s, second, false);
    expect(s).toEqual({ value: second, refreshing: false });
  });
});

describe("sondeo del nivel 2 (needed_capital_curve_state)", () => {
  it("arranca en 2 s y crece ×1,5 hasta el techo de 15 s", () => {
    expect(curvePollDelayMs(0)).toBe(CURVE_POLL_BASE_DELAY_MS);
    expect(curvePollDelayMs(1)).toBe(3_000);
    expect(curvePollDelayMs(2)).toBe(4_500);
    expect(curvePollDelayMs(3)).toBe(6_750);
    expect(curvePollDelayMs(99)).toBe(CURVE_POLL_MAX_DELAY_MS);
  });

  it("nunca baja del primer intervalo ni sube del techo", () => {
    for (let n = -3; n < 40; n += 1) {
      const d = curvePollDelayMs(n);
      expect(d).toBeGreaterThanOrEqual(CURVE_POLL_BASE_DELAY_MS);
      expect(d).toBeLessThanOrEqual(CURVE_POLL_MAX_DELAY_MS);
    }
  });

  it("es monótona: cada intento espera al menos lo que el anterior", () => {
    for (let n = 0; n < 20; n += 1) {
      expect(curvePollDelayMs(n + 1)).toBeGreaterThanOrEqual(curvePollDelayMs(n));
    }
  });

  it("solo sondea con `computing`: `ready` y `unavailable` son finales", () => {
    const base = { attempts: 0, active: true };
    expect(shouldPollNeededCurve({ ...base, state: "computing" })).toBe(true);
    expect(shouldPollNeededCurve({ ...base, state: "ready" })).toBe(false);
    expect(shouldPollNeededCurve({ ...base, state: "unavailable" })).toBe(false);
    expect(shouldPollNeededCurve({ ...base, state: null })).toBe(false);
    expect(shouldPollNeededCurve({ ...base, state: undefined })).toBe(false);
  });

  it("no sondea con la pestaña inactiva", () => {
    expect(
      shouldPollNeededCurve({ state: "computing", attempts: 0, active: false }),
    ).toBe(false);
  });

  it("se rinde al agotar los intentos", () => {
    expect(
      shouldPollNeededCurve({
        state: "computing",
        attempts: CURVE_POLL_MAX_ATTEMPTS - 1,
        active: true,
      }),
    ).toBe(true);
    expect(
      shouldPollNeededCurve({
        state: "computing",
        attempts: CURVE_POLL_MAX_ATTEMPTS,
        active: true,
      }),
    ).toBe(false);
  });
});
