/**
 * Perfil de jubilación en cliente (5.0.0, issue #207, **modelo v2** C1-C8).
 *
 * Cuatro cosas que este suite existe para impedir, todas silenciosas si se rompen:
 *
 *  1. **Un PATCH que no es mínimo.** Mandar el perfil entero resetea en el servidor lo que el
 *     usuario no tocó — el bug exacto que el tri-estado de `RetirementProfilePatch` existe para
 *     esquivar. Aquí se fija que solo viajan las claves cambiadas y que `null` borra de verdad,
 *     con `coast_stop_age` como el tri-estado nuevo del modelo v2.
 *  2. **Una guarda de validez que diverge de las cotas del servidor.** La tabla recorre CADA
 *     regla de `validate_retirement_profile` con su código estable: si Rust cambia una cota y
 *     nadie la trae aquí, el usuario recibe un 400 con el formulario prometiéndole «Guardado
 *     automático».
 *  3. **Un default del puente que se mueve en un lado y no en el otro.** `defaultBridgePct` es
 *     una FÓRMULA (`max(5, swr + 1)`) duplicada en Rust: si divergen, activar el puente enseña
 *     una tasa y guarda otra.
 *  4. **Una vista previa del objetivo que deja de cuadrar con el fixture compartido.** El SWR y
 *     el modo viven en el perfil; la fórmula del número FIRE clásico no se movió. Se recorre el
 *     mismo `fire-parity.json` que comparten Rust y `fire.test.ts`, pero pasando por un
 *     `RetirementProfileApi` real, que es la fontanería que la SPA usa de verdad desde 5.0.0.
 */

import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import type {
  FireNumberModeApi,
  PartialRetirementApi,
  PensionPlanApi,
  RetirementProfileApi,
  TaxBracketApi,
} from "../api/types";
import { ERROR_MESSAGES } from "./errorMessages";
import { computeFireAnnualNeedNetEur, grossUpNetAnnualFire } from "./fire";
import {
  DEFAULT_BRIDGE_YEARS,
  DEFAULT_SUCCESS_THRESHOLD_PCT,
  MAX_BRIDGE_PCT,
  MAX_BRIDGE_YEARS,
  MAX_GUARDRAIL_PCT,
  MAX_SUCCESS_THRESHOLD_PCT,
  MAX_SWR_PCT,
  MAX_WITHDRAWAL_PCT,
  MIN_BRIDGE_YEARS,
  MIN_PENSION_AGE,
  MIN_PROFILE_AGE,
  MIN_SUCCESS_THRESHOLD_PCT,
  RETIREMENT_STRATEGIES,
  buildRetirementProfilePatch,
  defaultBridgePct,
  defaultRetirementProfileApi,
  isEmptyRetirementProfilePatch,
  newPartialRetirementDraft,
  newPensionPlanDraft,
  normalizeRetirementProfile,
  parseRetirementStrategy,
  retirementProfileIssue,
  strategyRequiresTargetAge,
  withBridgeEnabled,
} from "./retirementProfile";

const base = (over: Partial<RetirementProfileApi> = {}): RetirementProfileApi => ({
  ...defaultRetirementProfileApi(),
  ...over,
});

const pensionOf = (over: Partial<PensionPlanApi> = {}): PensionPlanApi => ({
  monthly_amount_today: "1200",
  starts_at_age: 67,
  indexed: true,
  fraction_while_partial: "0",
  bridge_enabled: false,
  bridge_max_pct: null,
  bridge_max_years: null,
  ...over,
});

const partialOf = (over: Partial<PartialRetirementApi> = {}): PartialRetirementApi => ({
  mode: "at_age",
  starts_at_age: 55,
  income_monthly_today: "800",
  expense_basis: "retirement",
  ...over,
});

// ---------------------------------------------------------------------------
// Defaults y normalización — espejo de `resolve_retirement_profile`
// ---------------------------------------------------------------------------

describe("defaults del perfil", () => {
  it("un perfil por defecto es cruce por éxito al 95 %", () => {
    const p = defaultRetirementProfileApi();
    expect(p.strategy).toBe("asap");
    expect(p.swr_pct).toBe("3.5");
    expect(p.horizon_lifespan_age).toBe(90);
    expect(p.fire_number_mode).toBe("annual_expense");
    expect(p.withdrawal_rule.kind).toBe("fixed_real");
    expect(p.withdrawal_rule.spend_mode).toBe("ceiling");
    // El umbral VOLVIÓ al perfil (C3, revierte V7) y su default es 95, no 100: el 100 significa
    // cero fallos de N caminos y es una exigencia, no el punto de partida razonable.
    expect(p.success_threshold_pct).toBe(95);
    expect(p.coast_mode).toBe("fixed_retirement_age");
    expect(p.coast_stop_age).toBeNull();
  });

  it("y no arrastra ninguno de los tres ejes que el modelo v2 retiró", () => {
    // Si uno reapareciera, el PATCH lo mandaría y el servidor lo ignoraría en silencio: el
    // formulario enseñaría una decisión que no está en el plan que se simula.
    const p = defaultRetirementProfileApi();
    expect(p).not.toHaveProperty("target_basis");
    expect(p).not.toHaveProperty("bridge_discount_basis");
    expect(p).not.toHaveProperty("cash_buffer_months");
  });

  it("y se puede guardar tal cual (la guarda no bloquea el estado de partida)", () => {
    expect(retirementProfileIssue(defaultRetirementProfileApi())).toBeNull();
  });

  it("los borradores de pensión y de jornada reducida arrancan en su modo explícito", () => {
    const pen = newPensionPlanDraft("3.5");
    expect(pen.monthly_amount_today).toBe("");
    // El puente es un AJUSTE, no el estado natural de tener pensión: arranca apagado y sin
    // números, que es lo que el contrato publica para ese estado.
    expect(pen.bridge_enabled).toBe(false);
    expect(pen.bridge_max_pct).toBeNull();
    expect(pen.bridge_max_years).toBeNull();

    const par = newPartialRetirementDraft();
    expect(par.mode).toBe("at_age");
    expect(par.starts_at_age).toBe(60);
  });

  it("encender el puente pone los defaults, apagarlo no inventa números", () => {
    const on = withBridgeEnabled(newPensionPlanDraft("3.5"), true, "3.5");
    expect(on.bridge_enabled).toBe(true);
    expect(on.bridge_max_pct).toBe("5");
    expect(on.bridge_max_years).toBe(DEFAULT_BRIDGE_YEARS);
    // Y apagarlo conserva lo que había: el interruptor no destruye lo que el usuario escribió.
    const off = withBridgeEnabled(on, false, "3.5");
    expect(off.bridge_enabled).toBe(false);
    expect(off.bridge_max_pct).toBe("5");
  });
});

describe("los defaults del puente siguen al SWR", () => {
  // `max(5, swr + 1)`: el `swr + 1` mantiene la invariante «el puente es estrictamente mayor que
  // el SWR» y el suelo de 5 evita proponer un puente que no adelantaría ninguna fecha. La misma
  // fórmula está en Rust; si una se mueve sin la otra, la pantalla enseña una tasa y el servidor
  // guarda otra.
  const cases: Array<[string, string]> = [
    ["3.5", "5"],
    ["0", "5"],
    ["4", "5"],
    ["5", "6"],
    ["6", "7"],
  ];
  for (const [swr, expected] of cases) {
    it(`SWR ${swr} % → puente al ${expected} %`, () => {
      expect(defaultBridgePct(swr)).toBe(expected);
      // Y el default es siempre un puente DE VERDAD (estrictamente mayor), que es la única
      // propiedad de la que depende la guarda `bridge_max_pct_not_above_swr`.
      expect(Number(defaultBridgePct(swr))).toBeGreaterThan(Number(swr));
      expect(Number(defaultBridgePct(swr))).toBeLessThanOrEqual(MAX_BRIDGE_PCT);
    });
  }

  it("un SWR ilegible o fuera de cota no produce basura binaria", () => {
    expect(defaultBridgePct("tres")).toBe("5");
    expect(defaultBridgePct("99")).toBe("7"); // el SWR se acota a 6 antes de sumar
    expect(defaultBridgePct("2.9")).toBe("5");
    expect(defaultBridgePct("4.9")).toBe("5.9"); // no «5.900000000000001»
  });

  it("los años por defecto son 7 y caen dentro de la cota publicada", () => {
    expect(DEFAULT_BRIDGE_YEARS).toBe(7);
    expect(DEFAULT_BRIDGE_YEARS).toBeGreaterThanOrEqual(MIN_BRIDGE_YEARS);
    expect(DEFAULT_BRIDGE_YEARS).toBeLessThanOrEqual(MAX_BRIDGE_YEARS);
  });
});

describe("parseRetirementStrategy", () => {
  it("son cuatro y `pension_bridge` ya no es una de ellas", () => {
    expect([...RETIREMENT_STRATEGIES]).toEqual(["asap", "retire_at_age", "coast", "partial"]);
    expect(RETIREMENT_STRATEGIES).not.toContain("pension_bridge");
  });

  it("pliega el literal retirado `pension_bridge` a `asap`", () => {
    // Mismo precedente que `annual_expense_adjusted` en `parseFireNumberMode`: un literal que
    // sigue vivo en perfiles guardados y en backups no puede dejar el selector en blanco. Quien
    // ENCIENDE el puente al migrar es el servidor (aviso `strategy_pension_bridge_migrated`);
    // aquí solo se traduce la estrategia.
    expect(parseRetirementStrategy("pension_bridge")).toBe("asap");
    // Y por la vía que usa la SPA de verdad: un perfil guardado con el literal retirado.
    const guardado = normalizeRetirementProfile({
      ...defaultRetirementProfileApi(),
      strategy: "pension_bridge",
      pension: pensionOf({ bridge_enabled: true, bridge_max_pct: "5", bridge_max_years: 7 }),
    } as unknown as RetirementProfileApi);
    expect(guardado.strategy).toBe("asap");
    // El puente que el servidor encendió al migrar llega intacto y es guardable.
    expect(guardado.pension?.bridge_enabled).toBe(true);
    expect(retirementProfileIssue(guardado)).toBeNull();
  });

  it("cualquier otro literal desconocido también cae a `asap`", () => {
    expect(parseRetirementStrategy("monte_carlo")).toBe("asap");
    expect(parseRetirementStrategy(null)).toBe("asap");
  });
});

describe("normalizeRetirementProfile", () => {
  it("null / basura → los defaults", () => {
    expect(normalizeRetirementProfile(null)).toEqual(defaultRetirementProfileApi());
    expect(normalizeRetirementProfile(undefined)).toEqual(defaultRetirementProfileApi());
  });

  it("clampa en LECTURA lo que llegue fuera de rango, nunca lo rechaza", () => {
    const p = normalizeRetirementProfile({
      ...defaultRetirementProfileApi(),
      swr_pct: "99",
      horizon_lifespan_age: 200,
      target_retirement_age: 3,
    });
    expect(p.swr_pct).toBe(String(MAX_SWR_PCT));
    expect(p.horizon_lifespan_age).toBe(105);
    expect(p.target_retirement_age).toBe(MIN_PROFILE_AGE);
  });

  it("acota el umbral a 80..100 y lo pone en 95 cuando falta", () => {
    // Un backend anterior a C3 no publica el campo, y el 95 tiene que salir de aquí: un
    // `undefined` haría que la guarda devolviera `success_threshold_out_of_range` sobre un
    // perfil que nadie ha tocado.
    const sin = { ...defaultRetirementProfileApi() } as Partial<RetirementProfileApi>;
    delete sin.success_threshold_pct;
    expect(
      normalizeRetirementProfile(sin as RetirementProfileApi).success_threshold_pct,
    ).toBe(DEFAULT_SUCCESS_THRESHOLD_PCT);

    const bajo = normalizeRetirementProfile(base({ success_threshold_pct: 50 }));
    expect(bajo.success_threshold_pct).toBe(MIN_SUCCESS_THRESHOLD_PCT);
    const alto = normalizeRetirementProfile(base({ success_threshold_pct: 200 }));
    expect(alto.success_threshold_pct).toBe(MAX_SUCCESS_THRESHOLD_PCT);
    // Y se queda entero: «95,7 % de los caminos» no significa nada contra un conteo.
    expect(
      normalizeRetirementProfile(base({ success_threshold_pct: 88.7 })).success_threshold_pct,
    ).toBe(88);
  });

  it("acota la edad de parada de coast a la jubilación que la espera", () => {
    // Con edad objetivo, ella es el techo: dejar de aportar DESPUÉS de jubilarte no describe
    // ningún plan.
    expect(
      normalizeRetirementProfile(
        base({ target_retirement_age: 60, coast_stop_age: 70 }),
      ).coast_stop_age,
    ).toBe(60);
    // Sin edad objetivo el techo es el horizonte.
    expect(
      normalizeRetirementProfile(
        base({ horizon_lifespan_age: 90, coast_stop_age: 200 }),
      ).coast_stop_age,
    ).toBe(90);
    expect(normalizeRetirementProfile(base({ coast_stop_age: 3 })).coast_stop_age).toBe(
      MIN_PROFILE_AGE,
    );
    // `null` se conserva: es «no la he fijado», no una edad por defecto.
    expect(normalizeRetirementProfile(base({ coast_stop_age: null })).coast_stop_age).toBeNull();
  });

  it("rellena los números del puente cuando está encendido y faltan", () => {
    // El servidor hace exactamente lo mismo al resolver el perfil. Sin este relleno, encender el
    // puente dejaría el formulario en un estado que la guarda rechaza y el autosave no podría
    // guardar nunca.
    const p = normalizeRetirementProfile(
      base({
        pension: pensionOf({ bridge_enabled: true, bridge_max_pct: null, bridge_max_years: null }),
      }),
    );
    expect(p.pension?.bridge_max_pct).toBe("5");
    expect(p.pension?.bridge_max_years).toBe(DEFAULT_BRIDGE_YEARS);
    expect(retirementProfileIssue(p)).toBeNull();
  });

  it("sube a la tasa por defecto un puente que no supera al SWR, y acota el techo", () => {
    const flojo = normalizeRetirementProfile(
      base({
        swr_pct: "5",
        pension: pensionOf({ bridge_enabled: true, bridge_max_pct: "4" }),
      }),
    );
    expect(flojo.pension?.bridge_max_pct).toBe("6");

    const enorme = normalizeRetirementProfile(
      base({ pension: pensionOf({ bridge_enabled: true, bridge_max_pct: "99" }) }),
    );
    expect(enorme.pension?.bridge_max_pct).toBe(String(MAX_BRIDGE_PCT));

    const anios = normalizeRetirementProfile(
      base({ pension: pensionOf({ bridge_enabled: true, bridge_max_years: 99 }) }),
    );
    expect(anios.pension?.bridge_max_years).toBe(MAX_BRIDGE_YEARS);
  });

  it("con el puente apagado los números son inertes y se conservan", () => {
    // Apagado, el motor no los mira. Borrarlos aquí destruiría lo que alguien fijó por API o
    // dejó preparado antes de encender el interruptor.
    const p = normalizeRetirementProfile(
      base({ pension: pensionOf({ bridge_enabled: false, bridge_max_pct: "1", bridge_max_years: 3 }) }),
    );
    expect(p.pension?.bridge_enabled).toBe(false);
    expect(p.pension?.bridge_max_pct).toBe("1");
    expect(p.pension?.bridge_max_years).toBe(3);
    expect(retirementProfileIssue(p)).toBeNull();
  });

  it("la jornada reducida conserva su modo y su edad ausente", () => {
    const asap = normalizeRetirementProfile(
      base({ partial_retirement: partialOf({ mode: "asap", starts_at_age: null }) }),
    );
    expect(asap.partial_retirement?.mode).toBe("asap");
    // `null` se conserva: con `asap` la edad la resuelve el servidor y rellenarla aquí
    // inventaría una decisión que el usuario no tomó.
    expect(asap.partial_retirement?.starts_at_age).toBeNull();

    const raro = normalizeRetirementProfile(
      base({
        partial_retirement: partialOf({ mode: "cuando_sea" as PartialRetirementApi["mode"] }),
      }),
    );
    expect(raro.partial_retirement?.mode).toBe("at_age");
  });

  it("un enumerado desconocido cae a su default, no revienta la vista", () => {
    const p = normalizeRetirementProfile({
      ...defaultRetirementProfileApi(),
      strategy: "monte_carlo",
      coast_mode: "vibes",
      withdrawal_rule: { ...defaultRetirementProfileApi().withdrawal_rule, kind: "magia" },
    } as unknown as RetirementProfileApi);
    expect(p.strategy).toBe("asap");
    expect(p.coast_mode).toBe("fixed_retirement_age");
    expect(p.withdrawal_rule.kind).toBe("fixed_real");
  });

  it("las claves del modelo viejo se caen solas", () => {
    // Es el mecanismo por el que un JSONB de 4.15.x deja de existir sin migrar nada en cliente:
    // el objeto que devuelve `normalize` es un literal cerrado.
    const p = normalizeRetirementProfile({
      ...defaultRetirementProfileApi(),
      target_basis: "perpetuity",
      bridge_discount_basis: "expected_return",
      cash_buffer_months: 24,
    } as unknown as RetirementProfileApi);
    expect(p).not.toHaveProperty("target_basis");
    expect(p).not.toHaveProperty("bridge_discount_basis");
    expect(p).not.toHaveProperty("cash_buffer_months");
  });
});

describe("strategyRequiresTargetAge", () => {
  it("coast solo pide la edad de jubilación en su modo A", () => {
    // En el modo B el usuario fija la edad a la que deja de aportar y la fecha es el RESULTADO:
    // pedirle además la edad de jubilación sería pedirle la respuesta.
    expect(strategyRequiresTargetAge("retire_at_age", "fixed_retirement_age")).toBe(true);
    expect(strategyRequiresTargetAge("retire_at_age", "fixed_stop_age")).toBe(true);
    expect(strategyRequiresTargetAge("coast", "fixed_retirement_age")).toBe(true);
    expect(strategyRequiresTargetAge("coast", "fixed_stop_age")).toBe(false);
    expect(strategyRequiresTargetAge("asap", "fixed_retirement_age")).toBe(false);
    expect(strategyRequiresTargetAge("partial", "fixed_retirement_age")).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// PATCH mínimo y tri-estado
// ---------------------------------------------------------------------------

describe("buildRetirementProfilePatch — mínimo", () => {
  it("sin cambios, el patch está vacío (el servidor lo rechazaría con patch_empty)", () => {
    const p = base();
    const patch = buildRetirementProfilePatch(p, { ...p });
    expect(patch).toEqual({});
    expect(isEmptyRetirementProfilePatch(patch)).toBe(true);
  });

  it("solo viaja la clave que cambia: la pensión declarada NO se toca", () => {
    const before = base({ swr_pct: "3.0", pension: pensionOf({ monthly_amount_today: "1100" }) });
    const patch = buildRetirementProfilePatch(before, { ...before, swr_pct: "3.5" });
    expect(Object.keys(patch)).toEqual(["swr_pct"]);
    expect(patch.swr_pct).toBe("3.5");
    expect("pension" in patch).toBe(false);
  });

  it("un decimal reescrito con el mismo VALOR no genera patch", () => {
    // Sin esto, teclear «3,50» sobre un «3.5» guardado sería una escritura (y una invalidación
    // de la proyección) por cada pulsación de coma.
    const before = base({ swr_pct: "3.5" });
    expect(buildRetirementProfilePatch(before, { ...before, swr_pct: "3.50" })).toEqual({});
    expect(buildRetirementProfilePatch(before, { ...before, swr_pct: "3,5" })).toEqual({});
  });

  it("varios cambios a la vez viajan juntos y nada más", () => {
    const before = base();
    const patch = buildRetirementProfilePatch(
      before,
      base({ strategy: "retire_at_age", target_retirement_age: 58, horizon_lifespan_age: 95 }),
    );
    expect(Object.keys(patch).sort()).toEqual(
      ["horizon_lifespan_age", "strategy", "target_retirement_age"].sort(),
    );
  });

  it("el umbral y el modo de coast viajan como los demás escalares", () => {
    expect(
      buildRetirementProfilePatch(base(), base({ success_threshold_pct: 100 })),
    ).toEqual({ success_threshold_pct: 100 });
    expect(buildRetirementProfilePatch(base(), base({ coast_mode: "fixed_stop_age" }))).toEqual({
      coast_mode: "fixed_stop_age",
    });
  });
});

describe("buildRetirementProfilePatch — tri-estado", () => {
  it("borrar un bloque manda `null` explícito, no lo omite", () => {
    const before = base({ pension: pensionOf(), partial_retirement: partialOf() });
    const patch = buildRetirementProfilePatch(
      before,
      base({ pension: null, partial_retirement: null }),
    );
    expect(patch.pension).toBeNull();
    expect(patch.partial_retirement).toBeNull();
  });

  it("borrar la edad objetivo manda `null`, que no es 0", () => {
    const before = base({ target_retirement_age: 60 });
    const patch = buildRetirementProfilePatch(before, base({ target_retirement_age: null }));
    expect(patch.target_retirement_age).toBeNull();
    expect(patch.target_retirement_age).not.toBe(0);
  });

  it("`coast_stop_age` es mínimo y tri-estado", () => {
    // El servidor la CONSERVA aunque el modo no la use (igual que `target_retirement_age`), así
    // que soltarla tiene que ser una orden explícita y no un efecto colateral de cambiar de modo.
    const sinTocar = base({ strategy: "coast", coast_stop_age: 55 });
    expect(
      "coast_stop_age" in buildRetirementProfilePatch(sinTocar, { ...sinTocar, swr_pct: "3.2" }),
    ).toBe(false);

    const fijar = buildRetirementProfilePatch(base(), base({ coast_stop_age: 55 }));
    expect(fijar).toEqual({ coast_stop_age: 55 });

    const soltar = buildRetirementProfilePatch(sinTocar, {
      ...sinTocar,
      coast_stop_age: null,
    });
    expect("coast_stop_age" in soltar).toBe(true);
    expect(soltar.coast_stop_age).toBeNull();
  });

  it("declarar un bloque nuevo viaja entero", () => {
    const patch = buildRetirementProfilePatch(base(), base({ pension: pensionOf() }));
    expect(patch.pension).toEqual(pensionOf());
  });

  it("la pensión viaja con sus TRES campos de puente, nunca a medias", () => {
    // Qué campos son obligatorios dentro del bloque depende de otros campos del mismo bloque: un
    // merge parcial permitiría «puente encendido sin tasa», que nadie escribió.
    const before = base({ pension: pensionOf() });
    const patch = buildRetirementProfilePatch(before, {
      ...before,
      pension: withBridgeEnabled(pensionOf(), true, "3.5"),
    });
    expect(patch.pension).toEqual(
      pensionOf({ bridge_enabled: true, bridge_max_pct: "5", bridge_max_years: 7 }),
    );
  });

  it("apagar el puente manda los dos números a `null`, no números inertes", () => {
    const before = base({
      pension: pensionOf({ bridge_enabled: true, bridge_max_pct: "8", bridge_max_years: 10 }),
    });
    const patch = buildRetirementProfilePatch(before, {
      ...before,
      pension: { ...before.pension!, bridge_enabled: false },
    });
    expect(patch.pension?.bridge_enabled).toBe(false);
    expect(patch.pension?.bridge_max_pct).toBeNull();
    expect(patch.pension?.bridge_max_years).toBeNull();
  });

  it("una tasa de puente vaciada viaja RESUELTA, nunca como cadena vacía", () => {
    // La API solo acepta decimales. Y lo que se manda es lo que la pantalla estaba enseñando: el
    // mismo default que el servidor habría puesto.
    const before = base({ pension: pensionOf() });
    const patch = buildRetirementProfilePatch(before, {
      ...before,
      pension: pensionOf({ bridge_enabled: true, bridge_max_pct: "" }),
    });
    expect(patch.pension?.bridge_max_pct).toBe("5");
    expect(patch.pension?.bridge_max_years).toBe(DEFAULT_BRIDGE_YEARS);
  });

  it("la jornada reducida viaja con su `mode`", () => {
    const before = base({ partial_retirement: partialOf() });
    const patch = buildRetirementProfilePatch(before, {
      ...before,
      partial_retirement: partialOf({ mode: "asap", starts_at_age: null }),
    });
    expect(patch.partial_retirement?.mode).toBe("asap");
    expect(patch.partial_retirement?.starts_at_age).toBeNull();
  });

  it("la regla de retirada viaja ENTERA, nunca campo a campo", () => {
    // Sus `pct` obligatorios dependen de `kind`: un merge parcial permitiría estados como
    // «guardrails con el pct del percent_of_balance anterior» que nadie escribió.
    const before = base();
    const patch = buildRetirementProfilePatch(
      before,
      base({
        withdrawal_rule: {
          kind: "guardrails",
          pct: "4",
          start_pct: null,
          end_pct: null,
          band_pct: "20",
          adjust_pct: "10",
          spend_mode: "rule_is_spend",
        },
      }),
    );
    expect(patch.withdrawal_rule).toEqual({
      kind: "guardrails",
      pct: "4",
      start_pct: null,
      end_pct: null,
      band_pct: "20",
      adjust_pct: "10",
      spend_mode: "rule_is_spend",
    });
  });

  it("un importe vacío de bloque llega al wire como «0», nunca como cadena vacía", () => {
    const patch = buildRetirementProfilePatch(
      base(),
      base({ partial_retirement: partialOf({ income_monthly_today: "" }) }),
    );
    expect(patch.partial_retirement?.income_monthly_today).toBe("0");
  });

  it("ninguno de los tres ejes retirados puede colarse en el PATCH", () => {
    const before = base();
    const patch = buildRetirementProfilePatch(before, {
      ...before,
      target_basis: "perpetuity",
      cash_buffer_months: 24,
    } as unknown as RetirementProfileApi);
    expect(patch).toEqual({});
  });
});

// ---------------------------------------------------------------------------
// Guarda de validez — la tabla de cotas del servidor, regla por regla
// ---------------------------------------------------------------------------

describe("retirementProfileIssue — espejo de validate_retirement_profile", () => {
  const rule = defaultRetirementProfileApi().withdrawal_rule;

  const cases: Array<[string, RetirementProfileApi, string | null]> = [
    // --- Los cuatro ejes movidos conservan sus códigos de 4.15.x -------------------------
    ["SWR en la cota alta (6 %) es válido", base({ swr_pct: String(MAX_SWR_PCT) }), null],
    ["SWR por encima de la cota", base({ swr_pct: "6.1" }), "swr_out_of_range"],
    ["SWR negativo", base({ swr_pct: "-0.1" }), "swr_out_of_range"],
    ["SWR ilegible", base({ swr_pct: "tres" }), "decimal_invalid"],
    [
      "modo manual sin importe",
      base({ fire_number_mode: "manual", fire_number_manual_amount: null }),
      "fire_manual_amount_required",
    ],
    [
      "modo manual con importe 0",
      base({ fire_number_mode: "manual", fire_number_manual_amount: "0" }),
      "fire_manual_amount_not_positive",
    ],
    [
      "modo manual con importe positivo",
      base({ fire_number_mode: "manual", fire_number_manual_amount: "500000" }),
      null,
    ],
    [
      "horizonte por debajo de 85",
      base({ horizon_lifespan_age: 84 }),
      "horizon_lifespan_age_out_of_range",
    ],
    [
      "horizonte por encima de 105",
      base({ horizon_lifespan_age: 106 }),
      "horizon_lifespan_age_out_of_range",
    ],

    // --- Umbral de éxito (C3) ------------------------------------------------------------
    ["umbral en la cota baja (80)", base({ success_threshold_pct: 80 }), null],
    ["umbral en la cota alta (100)", base({ success_threshold_pct: 100 }), null],
    [
      "umbral por debajo de 80",
      base({ success_threshold_pct: 79 }),
      "success_threshold_out_of_range",
    ],
    [
      "umbral por encima de 100",
      base({ success_threshold_pct: 101 }),
      "success_threshold_out_of_range",
    ],
    [
      "umbral con decimales (no significa nada contra un conteo)",
      base({ success_threshold_pct: 95.5 }),
      "success_threshold_out_of_range",
    ],

    // --- Estrategia ---------------------------------------------------------------------
    [
      "retire_at_age sin edad",
      base({ strategy: "retire_at_age" }),
      "target_retirement_age_required",
    ],
    [
      "coast modo A sin edad de jubilación",
      base({ strategy: "coast", coast_mode: "fixed_retirement_age" }),
      "target_retirement_age_required",
    ],
    [
      "retire_at_age con edad",
      base({ strategy: "retire_at_age", target_retirement_age: 60 }),
      null,
    ],
    [
      "coast modo B NO pide edad de jubilación, pide la de parada",
      base({ strategy: "coast", coast_mode: "fixed_stop_age" }),
      "coast_stop_age_required",
    ],
    [
      "coast modo B con edad de parada",
      base({ strategy: "coast", coast_mode: "fixed_stop_age", coast_stop_age: 50 }),
      null,
    ],
    [
      "jornada reducida en modo `at_age` sin edad",
      base({
        strategy: "partial",
        partial_retirement: partialOf({ mode: "at_age", starts_at_age: null }),
      }),
      "partial_start_age_required",
    ],
    [
      "jornada reducida en modo `asap` no necesita edad",
      base({
        strategy: "partial",
        partial_retirement: partialOf({ mode: "asap", starts_at_age: null }),
      }),
      null,
    ],

    // --- Edades -------------------------------------------------------------------------
    [
      "edad objetivo por debajo del mínimo",
      base({ target_retirement_age: MIN_PROFILE_AGE - 1 }),
      "retirement_age_out_of_range",
    ],
    [
      "edad objetivo por encima del horizonte",
      base({ horizon_lifespan_age: 90, target_retirement_age: 91 }),
      "retirement_age_out_of_range",
    ],
    [
      "edad de parada por debajo del mínimo",
      base({
        strategy: "coast",
        coast_mode: "fixed_stop_age",
        coast_stop_age: MIN_PROFILE_AGE - 1,
      }),
      "coast_stop_age_out_of_range",
    ],
    [
      "edad de parada después de la jubilación fijada",
      base({
        strategy: "coast",
        coast_mode: "fixed_stop_age",
        target_retirement_age: 60,
        coast_stop_age: 61,
      }),
      "coast_stop_age_out_of_range",
    ],
    [
      "edad de parada por encima del horizonte (sin edad objetivo)",
      base({ horizon_lifespan_age: 90, coast_stop_age: 91 }),
      "coast_stop_age_out_of_range",
    ],
    [
      "pensión antes de la edad mínima",
      base({ pension: pensionOf({ starts_at_age: MIN_PENSION_AGE - 1 }) }),
      "pension_age_out_of_range",
    ],
    [
      "pensión después del horizonte",
      base({ horizon_lifespan_age: 90, pension: pensionOf({ starts_at_age: 91 }) }),
      "pension_age_out_of_range",
    ],
    [
      "pensión sin importe (recién activada)",
      base({ pension: pensionOf({ monthly_amount_today: "" }) }),
      "pension_amount_not_positive",
    ],
    [
      "pensión con importe 0",
      base({ pension: pensionOf({ monthly_amount_today: "0" }) }),
      "pension_amount_not_positive",
    ],
    [
      "fracción en media jornada fuera de [0,1]",
      base({ pension: pensionOf({ fraction_while_partial: "1.5" }) }),
      "pension_fraction_out_of_range",
    ],

    // --- Puente (C2/C7): solo cuenta si está ENCENDIDO -----------------------------------
    [
      "puente encendido sin números: hereda los defaults y es válido",
      base({ pension: pensionOf({ bridge_enabled: true }) }),
      null,
    ],
    [
      "puente encendido con tasa y años propios",
      base({
        pension: pensionOf({ bridge_enabled: true, bridge_max_pct: "8", bridge_max_years: 10 }),
      }),
      null,
    ],
    [
      "puente que no supera al SWR: no es un puente, es la misma tasa",
      base({
        swr_pct: "3.5",
        pension: pensionOf({ bridge_enabled: true, bridge_max_pct: "3.5" }),
      }),
      "bridge_max_pct_not_above_swr",
    ],
    [
      "puente por encima del techo de retirada",
      base({
        pension: pensionOf({
          bridge_enabled: true,
          bridge_max_pct: String(MAX_BRIDGE_PCT + 1),
        }),
      }),
      "bridge_max_pct_out_of_range",
    ],
    [
      "puente al 0 %",
      base({ pension: pensionOf({ bridge_enabled: true, bridge_max_pct: "0" }) }),
      "bridge_max_pct_out_of_range",
    ],
    [
      "puente de 0 años",
      base({ pension: pensionOf({ bridge_enabled: true, bridge_max_years: 0 }) }),
      "bridge_max_years_out_of_range",
    ],
    [
      "puente de más de 20 años",
      base({
        pension: pensionOf({ bridge_enabled: true, bridge_max_years: MAX_BRIDGE_YEARS + 1 }),
      }),
      "bridge_max_years_out_of_range",
    ],
    [
      "puente APAGADO con números imposibles: inertes, no bloquean el guardado",
      base({ pension: pensionOf({ bridge_max_pct: "0", bridge_max_years: 99 }) }),
      null,
    ],

    // --- Jornada reducida ---------------------------------------------------------------
    [
      "media jornada por debajo del mínimo",
      base({ partial_retirement: partialOf({ starts_at_age: MIN_PROFILE_AGE - 1 }) }),
      "partial_age_out_of_range",
    ],
    [
      "media jornada con ingreso negativo",
      base({ partial_retirement: partialOf({ income_monthly_today: "-1" }) }),
      "partial_income_negative",
    ],
    [
      "media jornada con ingreso vacío = año sabático, válido",
      base({ partial_retirement: partialOf({ income_monthly_today: "" }) }),
      null,
    ],
    [
      "media jornada que no empieza antes de la total",
      base({
        target_retirement_age: 60,
        partial_retirement: partialOf({ starts_at_age: 60 }),
      }),
      "partial_not_before_retirement",
    ],
    [
      "media jornada en modo `asap` no se compara con la edad de jubilación",
      base({
        target_retirement_age: 60,
        partial_retirement: partialOf({ mode: "asap", starts_at_age: null }),
      }),
      null,
    ],

    // --- Reglas de retirada: cada `kind` exige SUS campos --------------------------------
    ["fixed_real no pide nada", base({ withdrawal_rule: { ...rule } }), null],
    // U4 — un porcentaje ausente ya NO es un hueco: hereda `swr_pct` (3,5 % por defecto), que es
    // el punto entero de que la pantalla tenga un solo porcentaje editable.
    [
      "percent_of_balance sin pct HEREDA el SWR",
      base({ withdrawal_rule: { ...rule, kind: "percent_of_balance" } }),
      null,
    ],
    // …y la consecuencia declarada, la misma que en Rust: con el SWR a 0 lo heredado no es un
    // plan, y se dice en vez de devolver una simulación que no vende nada.
    [
      "percent_of_balance sin pct y SWR 0",
      base({ swr_pct: "0", withdrawal_rule: { ...rule, kind: "percent_of_balance" } }),
      "withdrawal_pct_out_of_range",
    ],
    [
      "percent_of_balance con pct",
      base({ withdrawal_rule: { ...rule, kind: "percent_of_balance", pct: "4" } }),
      null,
    ],
    [
      "percent_of_balance con pct 0",
      base({ withdrawal_rule: { ...rule, kind: "percent_of_balance", pct: "0" } }),
      "withdrawal_pct_out_of_range",
    ],
    [
      "percent_of_balance por encima del techo",
      base({
        withdrawal_rule: {
          ...rule,
          kind: "percent_of_balance",
          pct: String(MAX_WITHDRAWAL_PCT + 1),
        },
      }),
      "withdrawal_pct_out_of_range",
    ],
    [
      "hybrid sin start_pct hereda el SWR (3,5) y el 3 % queda por debajo",
      base({ withdrawal_rule: { ...rule, kind: "hybrid", end_pct: "3" } }),
      null,
    ],
    // El `end_pct` NO hereda nada: es el suelo del latch, no un porcentaje de retirada. Y se
    // compara contra el heredado, que es el que va a retirar el motor.
    [
      "hybrid sin start_pct y end por encima del SWR heredado",
      base({ withdrawal_rule: { ...rule, kind: "hybrid", end_pct: "3.9" } }),
      "hybrid_end_pct_not_below_start",
    ],
    [
      "hybrid sin end_pct sigue siendo un hueco",
      base({ withdrawal_rule: { ...rule, kind: "hybrid" } }),
      "withdrawal_pct_required",
    ],
    [
      "hybrid con end >= start",
      base({
        withdrawal_rule: { ...rule, kind: "hybrid", start_pct: "3", end_pct: "5" },
      }),
      "hybrid_end_pct_not_below_start",
    ],
    [
      "hybrid coherente",
      base({
        withdrawal_rule: { ...rule, kind: "hybrid", start_pct: "5", end_pct: "3" },
      }),
      null,
    ],
    [
      "guardrails sin adjust_pct",
      base({
        withdrawal_rule: { ...rule, kind: "guardrails", pct: "4", band_pct: "20" },
      }),
      "withdrawal_pct_required",
    ],
    [
      "guardrails con banda fuera de cota",
      base({
        withdrawal_rule: {
          ...rule,
          kind: "guardrails",
          pct: "4",
          band_pct: String(MAX_GUARDRAIL_PCT + 1),
          adjust_pct: "10",
        },
      }),
      "withdrawal_band_out_of_range",
    ],
    [
      "guardrails completo",
      base({
        withdrawal_rule: {
          ...rule,
          kind: "guardrails",
          pct: "4",
          band_pct: "20",
          adjust_pct: "10",
        },
      }),
      null,
    ],
  ];

  for (const [name, profile, expected] of cases) {
    it(name, () => {
      expect(retirementProfileIssue(profile)).toBe(expected);
    });
  }

  /**
   * Códigos del modelo v2 cuya frase todavía NO está en el catálogo: las escribe A11, en el
   * mismo commit que `fixtures/error-codes.json` (B8 — los mensajes tienen que nombrar rótulos
   * que existan en pantalla).
   *
   * **Esta lista se vacía sola por la fuerza**: la aserción de abajo es una IGUALDAD, así que en
   * cuanto A11 añada las frases el test se pone ROJO y obliga a borrar la fila. Un `filter` que
   * las tolerara para siempre convertiría la deuda en un archivo.
   */
  const FRASES_PENDIENTES_A11 = [
    "bridge_max_pct_not_above_swr",
    "bridge_max_pct_out_of_range",
    "bridge_max_years_out_of_range",
    "coast_stop_age_out_of_range",
    "coast_stop_age_required",
    "partial_start_age_required",
    "success_threshold_out_of_range",
  ];

  it("todo código que devuelve la guarda tiene frase en español (salvo los pendientes de A11)", () => {
    // Si la guarda inventara un código propio, el usuario vería el mensaje genérico y nadie se
    // enteraría: el catálogo es la única superficie donde se traduce lo que se lee en pantalla.
    const codes = new Set(
      cases.map(([, , code]) => code).filter((c): c is string => c !== null),
    );
    const missing = [...codes].filter((c) => !ERROR_MESSAGES[c]).sort();
    expect(missing).toEqual(FRASES_PENDIENTES_A11);
  });

  it("los códigos del modelo viejo ya no los puede devolver nadie", () => {
    const emitted = new Set(cases.map(([, , code]) => code));
    for (const dead of [
      "cash_buffer_out_of_range",
      "pension_required_for_bridge",
      "bridge_discount_out_of_range",
      "target_basis",
    ]) {
      expect(emitted.has(dead)).toBe(false);
    }
  });
});

// ---------------------------------------------------------------------------
// Vista previa del objetivo a través de la fontanería NUEVA (perfil → preview)
// ---------------------------------------------------------------------------

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const FIXTURE_PATH = path.resolve(
  __dirname,
  "../../../api/tests/fixtures/fire-parity.json",
);

type ParityCase = {
  name: string;
  fire_settings: {
    fire_number_mode: FireNumberModeApi;
    fire_number_manual_amount?: string | null;
    swr_pct: string;
    taxes_enabled: boolean;
    tax_brackets: TaxBracketApi[];
    taxable_gain_ratio?: string;
  };
  monthly: { income: string; income_retirement: string; expense_retirement: string };
  expected_target_nw: number | null;
};

describe("vista previa del número FIRE clásico desde el PERFIL (fixture compartido)", () => {
  const fixture = JSON.parse(readFileSync(FIXTURE_PATH, "utf8")) as {
    cases: ParityCase[];
  };

  for (const c of fixture.cases) {
    it(`case "${c.name}" cuadra ±1 € pasando por RetirementProfileApi`, () => {
      // Los tres ejes personales del caso se meten en un perfil de verdad —normalizado como lo
      // normaliza la SPA— y los fiscales se quedan donde siguen viviendo (el hogar). Si alguien
      // se llevara el SWR o el modo a otro sitio sin arrastrar la fórmula, esto se cae.
      const profile = normalizeRetirementProfile(
        base({
          fire_number_mode: c.fire_settings.fire_number_mode,
          fire_number_manual_amount: c.fire_settings.fire_number_manual_amount ?? null,
          swr_pct: c.fire_settings.swr_pct,
        }),
      );

      const need = computeFireAnnualNeedNetEur(
        {
          fire_number_mode: profile.fire_number_mode,
          fire_number_manual_amount: profile.fire_number_manual_amount,
        },
        c.monthly.expense_retirement,
        c.monthly.income,
        c.monthly.income_retirement,
      );
      const swr = Number(profile.swr_pct);
      const actual =
        need === null || need <= 0 || !Number.isFinite(swr) || swr <= 0
          ? null
          : grossUpNetAnnualFire(
              need,
              c.fire_settings.tax_brackets,
              c.fire_settings.taxes_enabled,
              Number(c.fire_settings.taxable_gain_ratio ?? "1"),
            ) /
            (swr / 100);

      if (c.expected_target_nw === null) {
        expect(actual).toBeNull();
        return;
      }
      expect(actual).not.toBeNull();
      expect(Math.abs((actual as number) - c.expected_target_nw)).toBeLessThanOrEqual(1);
    });
  }

  it("el SWR del perfil se clampa antes de dividir: nunca una cifra con SWR > 6 %", () => {
    // El clamp de lectura es lo que impide que un backup o una edición directa de la BD metan
    // un SWR imposible en la vista previa y publiquen una cifra que el servidor no calcula.
    const p = normalizeRetirementProfile(base({ swr_pct: "40" }));
    expect(Number(p.swr_pct)).toBe(MAX_SWR_PCT);
  });
});
