/**
 * La cabecera de resultados del modelo v2: **tres tarjetas, una cifra por tarjeta**.
 *
 * Tres cosas se fijan aquí porque son decisiones del contrato (§4), no accidentes de
 * implementación:
 *
 *  1. **Las dos primeras son FIJAS** — «Capital necesario hoy» y «Éxito del plan» — y la tercera
 *     la elige la estrategia. Sin test, el día que alguien reordene un `if` la cabecera cambiará
 *     de contenido sin que nada falle.
 *  2. **Un plan que no está no se rellena**: `birth_date_missing` dice qué falta, y
 *     `not_reachable` dice «Nunca» en la tercera tarjeta y «no hay éxito que medir» en la del
 *     éxito. Son estados distintos y un guion mudo los haría indistinguibles. (Había un tercero,
 *     `retirement_date_basis: "pending"` → «calculando…», retirado en A12 con el literal: el
 *     servidor nunca lo emitió.)
 *  3. **«Capital necesario hoy» está siempre en euros de hoy** y este módulo no acepta deflactor
 *     alguno: la cifra responde a «¿cuánto necesitaría si me jubilara YA?», y «ya» es hoy.
 */

import { describe, expect, it } from "vitest";
import {
  buildRetirementTilesV2,
  retirementDetailRows,
  RETIREMENT_TILES_V2_CAP,
  type RetirementTileV2Series,
  type RetirementTilesV2Input,
} from "./retirement-tiles";

const EUR = "EUR";
const monthLabel = (mi: number) => `M${mi}`;
/** Edad de juguete: 30 años hoy, uno más por cada 12 meses de la rejilla. */
const monthAge = (mi: number) => 30 + Math.floor(mi / 12);
/** Lo que emite `Intl` en es-ES: espacio DURO antes del símbolo y miles solo a partir de
 *  10.000. Se construye aquí para que el test no dependa de cómo se teclea un NBSP. */
const eur = (digits: string) => `${digits}\u00a0\u20ac`;

function series(over: Partial<RetirementTileV2Series> = {}): RetirementTileV2Series {
  return {
    strategy: "asap",
    retirement_date_basis: "success_threshold",
    success_threshold_pct: 95,
    safe_date_month_index: 204,
    safe_date_date_ymd: "2043-09-01",
    safe_date_age: 55,
    safe_date_at_100_month_index: null,
    safe_date_at_90_month_index: null,
    success_of_plan: 0.952,
    success_wilson_low: 0.944,
    success_sampling_error_pp: "1.2000",
    paths_used: 2500,
    seed: "12345678901234567890",
    needed_capital_today: "620000.0000",
    contribution_required_monthly: null,
    contribution_required_search_ceiling: null,
    contribution_underfunded: null,
    coast_stop_month_index: null,
    partial_start_month_index: null,
    plan_absent_reason: null,
    fire_number_classic_today: null,
    warnings: [],
    ...over,
  };
}

function input(
  over: Partial<RetirementTileV2Series> = {},
  rest: Partial<Omit<RetirementTilesV2Input, "series">> = {},
): RetirementTilesV2Input {
  return {
    series: series(over),
    currencyIso: EUR,
    monthLabel,
    monthAge,
    targetRetirementAge: 55,
    ...rest,
  };
}

const keys = (i: RetirementTilesV2Input) => buildRetirementTilesV2(i).map((t) => t.key);
const tile = (i: RetirementTilesV2Input, key: string) =>
  buildRetirementTilesV2(i).find((t) => t.key === key);

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("las dos tarjetas FIJAS", () => {
  it("el tope es 3 y NUNCA se pasa, con ninguna estrategia", () => {
    expect(RETIREMENT_TILES_V2_CAP).toBe(3);
    for (const strategy of ["asap", "retire_at_age", "coast", "partial"] as const) {
      const rich = input({
        strategy,
        contribution_required_monthly: "300.0000",
        contribution_required_search_ceiling: "1800.0000",
        coast_stop_month_index: 84,
        partial_start_month_index: 120,
      });
      expect(buildRetirementTilesV2(rich).length, strategy).toBeLessThanOrEqual(3);
    }
  });

  it("«Capital necesario hoy» y «Éxito del plan» son las dos primeras, siempre y en ese orden", () => {
    for (const strategy of ["asap", "retire_at_age", "coast", "partial"] as const) {
      expect(keys(input({ strategy })).slice(0, 2), strategy).toEqual([
        "needed_capital",
        "success",
      ]);
    }
  });

  it("«Capital necesario hoy» va en euros de HOY y lo dice, con el umbral como recuento", () => {
    const t = tile(input(), "needed_capital")!;
    expect(t.label).toBe("Capital necesario hoy");
    expect(t.value).toBe(eur("620.000"));
    expect(t.subtitle).toBe(
      "en euros de hoy · para que aguanten 95 de cada 100 escenarios",
    );
    expect(t.helpId).toBe("retirement.needed_capital");
  });

  it("el toggle «en dinero de hoy» NO puede tocarla: aquí no entra ningún deflactor", () => {
    // La garantía es estructural — `RetirementTilesV2Input` no tiene deflactor —, así que lo que
    // se fija es el efecto: la cifra es exactamente la que publica el servidor, sin escalar.
    expect(tile(input({ needed_capital_today: "620000.0000" }), "needed_capital")?.value).toBe(
      eur("620.000"),
    );
    expect(Object.keys(input())).not.toContain("deflator");
  });

  // El plan SÍ está resuelto (`plan_absent_reason: null`) y aun así falta esta CIFRA sola: el
  // subtítulo tiene que decir por qué en vez de repetir «en euros de hoy» junto a un guion mudo.
  it("sin capital necesario, PERO con plan resuelto, el subtítulo dice el motivo de la CIFRA", () => {
    const reason = (r: RetirementTileV2Series["needed_capital_absent_reason"]) =>
      tile(
        input({ needed_capital_today: null, needed_capital_absent_reason: r }),
        "needed_capital",
      )!;
    expect(reason("no_liquid_assets").value).toBe("—");
    expect(reason("no_liquid_assets").subtitle).toBe("sin activos líquidos que escalar");
    expect(reason("threshold_unreachable").subtitle).toBe("ningún capital alcanza tu umbral");
    expect(reason("month_beyond_horizon").subtitle).toBe("la fecha cae fuera del horizonte");
    // Un literal que este backend no reconoce (o su ausencia) cae al «no disponible» genérico de
    // la casa, nunca a la frase de «en euros de hoy» que prometía una cifra que no llegó.
    expect(reason(undefined).subtitle).toBe("no disponible");
    expect(reason(undefined).subtitle).not.toContain("en euros de hoy");
  });

  // `already_covered` es la ÚNICA de las cuatro ausencias que es una RESPUESTA y no un fallo de
  // medición: ni dividiendo la cartera por 256 el plan incumple el umbral, así que no hace falta
  // capital adicional hoy. Enseñarla con el mismo «—» que «sin activos líquidos» tira la única de
  // las cuatro que es una buena noticia — y era el remate del bug de la curva, que publicaba el
  // ahorro acumulado de la nómina donde no había necesidad ninguna.
  it("«ya cubierto» ocupa el sitio de la cifra en vez de un guion mudo", () => {
    const t = tile(
      input({
        needed_capital_today: null,
        needed_capital_absent_reason: "already_covered",
      }),
      "needed_capital",
    )!;
    expect(t.value).toBe("Ya cubierto");
    expect(t.value).not.toBe("—");
    expect(t.subtitle).toBe("con lo que tienes hoy tu plan cumple el umbral");
    // Sigue siendo la primera tarjeta y sigue apuntando a la misma ayuda: lo que cambia es lo que
    // dice, no dónde está.
    expect(t.label).toBe("Capital necesario hoy");
    expect(t.helpId).toBe("retirement.needed_capital");
  });

  // Sin plan (`plan_absent_reason`) manda la ausencia del BLOQUE: ahí sí va el guion, porque no es
  // que no haga falta capital, es que no hay plan del que hablar.
  it("sin plan, «ya cubierto» no se cuela: manda la ausencia del bloque", () => {
    const t = tile(
      input({
        plan_absent_reason: "birth_date_missing",
        needed_capital_today: null,
        needed_capital_absent_reason: "already_covered",
      }),
      "needed_capital",
    )!;
    expect(t.value).toBe("—");
    expect(t.subtitle).not.toBe("con lo que tienes hoy tu plan cumple el umbral");
  });

  it("«Éxito del plan» lleva el umbral y la precisión del sorteo en el subtítulo", () => {
    const t = tile(input(), "success")!;
    expect(t.label).toBe("Éxito del plan");
    // Los topes anti-mentira de `scenariosPerHundred` cuantizan a unidades de «de cada 100»:
    // 0,952 se imprime «95,0 %», y la precisión real la declara el «±1,2 pp» de al lado.
    expect(t.value).toBe("95,0 %");
    expect(t.subtitle).toBe("umbral 95,0 % · ±1,2 pp");
    expect(t.helpId).toBe("retirement.success");
  });

  it("sin precisión publicada el subtítulo se queda en el umbral, sin «±0,0 pp»", () => {
    expect(tile(input({ success_sampling_error_pp: null }), "success")?.subtitle).toBe(
      "umbral 95,0 %",
    );
  });

  // A12 — el caso que rotulaba un porcentaje junto a un plan que no ocurre. Con `not_reachable`
  // la serie SÍ publica `success_of_plan`, pero mide la MEJOR observación del solve (el mes que
  // más cerca se quedó), no «el éxito de tu plan», que es lo que esta tarjeta promete.
  describe("sin fecha alcanzable no hay éxito que rotular", () => {
    const noDate = (over: Partial<RetirementTileV2Series> = {}) =>
      input({
        retirement_date_basis: "not_reachable",
        safe_date_month_index: null,
        safe_date_date_ymd: null,
        safe_date_age: null,
        // La mejor observación del solve: un 68 % perfectamente respetable... de otro mes.
        success_of_plan: 0.68,
        success_wilson_low: 0.66,
        ...over,
      });

    it("el valor es un guion, nunca la mejor observación del solve", () => {
      const t = tile(noDate(), "success")!;
      expect(t.value).toBe("—");
      expect(t.value).not.toMatch(/%/);
      expect(t.value).not.toContain("68");
    });

    it("el subtítulo dice por qué falta, y conserva el umbral del perfil", () => {
      const t = tile(noDate(), "success")!;
      expect(t.subtitle).toBe(
        "no hay ninguna fecha que llegue a tu umbral, así que no hay éxito que medir · umbral 95,0 %",
      );
    });

    it("sin color: el semáforo de un plan sin fecha es la ausencia de semáforo", () => {
      expect(tile(noDate(), "success")!.tone).toBe("default");
    });

    it("«Capital necesario hoy» SÍ sigue publicando su cifra: contesta a otra pregunta", () => {
      // «¿Cuánto necesitaría si me jubilara YA?» tiene respuesta aunque ningún mes del horizonte
      // llegue al umbral — y es justo la cifra que dice cuánto falta.
      const t = tile(noDate(), "needed_capital")!;
      expect(t.value).toBe(eur("620.000"));
      expect(t.subtitle).toContain("en euros de hoy");
    });
  });

  it("sin serie no hay tarjetas", () => {
    expect(buildRetirementTilesV2({ ...input(), series: null })).toEqual([]);
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("the_third_tile_follows_the_strategy", () => {
  it("cada estrategia trae SU tercera tarjeta y ninguna otra", () => {
    expect(keys(input({ strategy: "asap" }))).toEqual([
      "needed_capital",
      "success",
      "safe_date",
    ]);
    expect(
      keys(
        input({
          strategy: "retire_at_age",
          contribution_required_monthly: "300.0000",
          contribution_required_search_ceiling: "1800.0000",
        }),
      ),
    ).toEqual(["needed_capital", "success", "required_contribution"]);
    expect(keys(input({ strategy: "coast", coast_stop_month_index: 84 }))).toEqual([
      "needed_capital",
      "success",
      "coast_month",
    ]);
    expect(keys(input({ strategy: "partial", partial_start_month_index: 120 }))).toEqual([
      "needed_capital",
      "success",
      "partial_start",
    ]);
  });

  it("las tarjetas muertas del objetivo NO vuelven por ninguna puerta", () => {
    // `target`, `coast_number`, `partial_gap`, `disposable` y `bridge` servían a un objetivo que
    // el modelo v2 retiró del contrato: si alguna reaparece, es que alguien resucitó la escuela
    // del objetivo sin decirlo.
    const all = new Set(
      (["asap", "retire_at_age", "coast", "partial"] as const).flatMap((strategy) =>
        keys(
          input({
            strategy,
            contribution_required_monthly: "300.0000",
            coast_stop_month_index: 84,
            partial_start_month_index: 120,
          }),
        ),
      ),
    );
    for (const dead of ["target", "coast_number", "partial_gap", "disposable", "bridge"]) {
      expect(all.has(dead), dead).toBe(false);
    }
  });

  it("una cifra que el servidor NO publica no se pinta con guion: la cabecera se queda en dos", () => {
    // `null` ≠ 0: la estrategia no resolvió su hito, y una tercera tarjeta con «—» diría que el
    // dato existe y hoy falta.
    expect(keys(input({ strategy: "retire_at_age" }))).toEqual(["needed_capital", "success"]);
    expect(keys(input({ strategy: "coast" }))).toEqual(["needed_capital", "success"]);
    expect(keys(input({ strategy: "partial" }))).toEqual(["needed_capital", "success"]);
  });

  it("en Hogar (sin estrategia) tampoco hay tercera", () => {
    expect(keys(input({ strategy: null }))).toEqual(["needed_capital", "success"]);
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("«Fecha válida» (asap)", () => {
  it("el AÑO como cifra y la edad + el tramo en el subtítulo", () => {
    const t = tile(input({ strategy: "asap" }), "safe_date")!;
    expect(t.label).toBe("Fecha válida");
    expect(t.value).toBe("2043");
    expect(t.subtitle).toBe("a los 55 años · dentro de 17 años");
    expect(t.helpId).toBe("retirement.safe_date");
  });

  it("not_reachable_prints_nunca", () => {
    const t = tile(
      input({
        strategy: "asap",
        retirement_date_basis: "not_reachable",
        safe_date_month_index: null,
        safe_date_date_ymd: null,
        safe_date_age: null,
      }),
      "safe_date",
    )!;
    // «Nunca» y no un 0, ni un guion: ningún mes del horizonte cumple el umbral, y eso es una
    // respuesta.
    expect(t.value).toBe("Nunca");
    expect(t.subtitle).toContain("ningún mes del horizonte");
    expect(t.tone).toBe("danger");
  });

  it("sin fecha civil se apoya en el rotulador del eje, no se queda muda", () => {
    expect(
      tile(input({ strategy: "asap", safe_date_date_ymd: null }), "safe_date")?.value,
    ).toBe("M204");
  });

  it("sin edad resoluble no la inventa", () => {
    expect(
      tile(input({ strategy: "asap", safe_date_age: null }), "safe_date")?.subtitle,
    ).toBe("dentro de 17 años");
  });

  it("una fecha válida que ya llegó no se anuncia como «dentro de 0 meses»", () => {
    expect(
      tile(
        input({ strategy: "asap", safe_date_month_index: 0, safe_date_age: 38 }),
        "safe_date",
      )?.subtitle,
    ).toBe("a los 38 años · ya puedes jubilarte");
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("«Aportación mínima» (retire_at_age)", () => {
  const contrib = (over: Partial<RetirementTileV2Series> = {}) =>
    tile(
      input({
        strategy: "retire_at_age",
        contribution_required_monthly: "300.0000",
        contribution_required_search_ceiling: "1800.0000",
        contribution_underfunded: false,
        ...over,
      }),
      "required_contribution",
    );

  it("el importe, con el sobrante del que sale (sin denominador no se sabe si es mucho)", () => {
    const t = contrib()!;
    expect(t.label).toBe("Aportación mínima");
    expect(t.value).toBe(eur("300"));
    expect(t.subtitle).toBe(
      `al mes, además de lo que ya aportas · de ${eur("1800")}/mes de sobrante`,
    );
    expect(t.tone).toBe("default");
    expect(t.helpId).toBe("retirement.required_contribution");
  });

  it("cero es «ya llegas», nunca un «0 €» que se leería como «no ahorres»", () => {
    const t = contrib({ contribution_required_monthly: "0.0000" })!;
    expect(t.value).toBe("Ya llegas");
    expect(t.subtitle).toContain("con lo que ya aportas");
    expect(t.tone).toBe("default");
  });

  it("infra-financiado es «ni ahorrándolo todo», en rojo y con palabras", () => {
    // El importe SERÍA el techo entero, y pintarlo diría «ahorra esto y llegas» — lo contrario.
    const t = contrib({
      contribution_underfunded: true,
      contribution_required_monthly: "1800.0000",
    })!;
    expect(t.value).toBe("Ni ahorrándolo todo");
    expect(t.value).not.toContain("1800");
    expect(t.tone).toBe("danger");
    expect(t.subtitle).toContain("ni invirtiendo cada euro");
  });

  it("`contribution_underfunded: false` es «llegas», no «no aplica»", () => {
    expect(contrib({ contribution_underfunded: false })?.tone).toBe("default");
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("«Mes coast» (coast)", () => {
  it("el mes del eje, con la edad y el tramo", () => {
    const t = tile(input({ strategy: "coast", coast_stop_month_index: 84 }), "coast_month")!;
    expect(t.label).toBe("Mes coast");
    expect(t.value).toBe("M84");
    expect(t.subtitle).toBe("a los 37 años · dentro de 7 años");
    expect(t.helpId).toBe("retirement.coast_month");
  });

  it("un mes coast ya alcanzado no se anuncia como un plazo futuro", () => {
    expect(
      tile(input({ strategy: "coast", coast_stop_month_index: 0 }), "coast_month")?.subtitle,
    ).toBe("a los 30 años · ya puedes dejar de aportar");
  });

  it("sin edad resoluble el subtítulo se queda en el tramo", () => {
    expect(
      tile(
        input({ strategy: "coast", coast_stop_month_index: 84 }, { monthAge: undefined }),
        "coast_month",
      )?.subtitle,
    ).toBe("dentro de 7 años");
  });

  it("`coast_not_reachable` dice «No puedes parar nunca», no un guion", () => {
    const t = tile(
      input({
        strategy: "coast",
        coast_stop_month_index: null,
        warnings: ["coast_not_reachable"],
      }),
      "coast_month",
    )!;
    expect(t.value).toBe("No puedes parar nunca");
    expect(t.subtitle).toContain("ni aportando todos los meses");
    expect(t.tone).toBe("danger");
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("«Inicio de la jornada reducida» (partial)", () => {
  it("el mes de arranque de la fase, con edad y tramo", () => {
    const t = tile(
      input({ strategy: "partial", partial_start_month_index: 120 }),
      "partial_start",
    )!;
    expect(t.label).toBe("Inicio de la jornada reducida");
    expect(t.value).toBe("M120");
    expect(t.subtitle).toBe("a los 40 años · dentro de 10 años");
    expect(t.helpId).toBe("retirement.partial_mode");
  });

  it("`partial_never_starts` es «Nunca»: la fase no arranca en ningún mes", () => {
    const t = tile(
      input({
        strategy: "partial",
        partial_start_month_index: null,
        warnings: ["partial_never_starts"],
      }),
      "partial_start",
    )!;
    expect(t.value).toBe("Nunca");
    expect(t.tone).toBe("danger");
  });

  it("`partial_never_fully_retires` conserva el mes y avisa de que de ahí no se sale", () => {
    // Es el caso PEOR y distinto del anterior: la fase empieza y la jubilación total nunca llega.
    const t = tile(
      input({
        strategy: "partial",
        partial_start_month_index: 120,
        warnings: ["partial_never_fully_retires"],
      }),
      "partial_start",
    )!;
    expect(t.value).toBe("M120");
    expect(t.subtitle).toContain("nunca llegas a jubilarte del todo");
    expect(t.tone).toBe("danger");
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("un plan que no está no se rellena", () => {
  it("una razón de ausencia gana aunque las cifras viejas sigan pegadas a la respuesta", () => {
    const t = tile(
      input({
        plan_absent_reason: "birth_date_missing",
        needed_capital_today: "620000.0000",
      }),
      "needed_capital",
    )!;
    expect(t.value).toBe("—");
    expect(t.value).not.toContain("620");
  });

  it("a_missing_birth_date_shows_why", () => {
    const tiles = buildRetirementTilesV2(
      input({
        plan_absent_reason: "birth_date_missing",
        needed_capital_today: null,
        success_of_plan: null,
      }),
    );
    expect(tiles.map((t) => t.key)).toEqual(["needed_capital", "success"]);
    for (const t of tiles) expect(t.subtitle, t.key).toBe("falta tu fecha de nacimiento");
  });

  it("cada razón de ausencia tiene su frase, y una desconocida no se inventa", () => {
    const reason = (r: RetirementTileV2Series["plan_absent_reason"]) =>
      tile(input({ plan_absent_reason: r }), "needed_capital")?.subtitle;
    expect(reason("months_override")).toBe("esta vista fija un horizonte propio");
    expect(reason("household_aggregate")).toBe("el hogar no resuelve un plan común");
    expect(reason("algo_nuevo" as never)).toBe("no disponible");
    // A12: `no_liquid_assets` NO es una razón de `plan_absent_reason` (vive en
    // `needed_capital_absent_reason`), así que cae a la genérica como cualquier otro literal que
    // el servidor no emite por este campo.
    expect(reason("no_liquid_assets" as never)).toBe("no disponible");
  });

  it("sin plan no hay tercera tarjeta: repetir la razón por tercera vez no añade nada", () => {
    expect(
      keys(
        input({
          strategy: "coast",
          coast_stop_month_index: 84,
          plan_absent_reason: "birth_date_missing",
        }),
      ),
    ).toEqual(["needed_capital", "success"]);
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("retirementDetailRows — lo que la cabecera ya no lleva", () => {
  it("sin serie no hay filas", () => {
    expect(retirementDetailRows({ ...input(), series: null })).toEqual([]);
  });

  it("el número FIRE clásico, con su rótulo completo y su ayuda", () => {
    const r = retirementDetailRows(
      input({ fire_number_classic_today: "900000.0000" }),
    ).find((x) => x.key === "fire_number_classic")!;
    expect(r.label).toBe(
      "Número FIRE clásico (tu gasto anual ÷ tu tasa + lo que te queda de deuda, sin pensión)",
    );
    expect(r.value).toBe(eur("900.000"));
    expect(r.helpId).toBe("retirement.fire_number_classic");
  });

  it("sin número clásico no hay fila con guion", () => {
    expect(
      retirementDetailRows(input()).map((r) => r.key),
    ).not.toContain("fire_number_classic");
  });

  it("las dos cotas de la fecha: al 100 % y al 90 %", () => {
    const by = Object.fromEntries(
      retirementDetailRows(
        input({ safe_date_at_100_month_index: 300, safe_date_at_90_month_index: 168 }),
      ).map((r) => [r.key, r]),
    );
    expect(by["safe_date_100"]!.label).toBe("Fecha al 100 %");
    expect(by["safe_date_100"]!.value).toBe("M300");
    expect(by["safe_date_90"]!.label).toBe("Fecha al 90 %");
    expect(by["safe_date_90"]!.value).toBe("M168");
  });

  it("con el plan resuelto, una cota ausente es «nunca» — un resultado, no un hueco", () => {
    const by = Object.fromEntries(
      retirementDetailRows(input()).map((r) => [r.key, r.value]),
    );
    expect(by["safe_date_100"]).toBe("nunca");
    expect(by["safe_date_90"]).toBe("nunca");
  });

  it("con el plan SIN resolver las cotas no se pintan: ahí `null` sí es «todavía no se sabe»", () => {
    for (const over of [{ plan_absent_reason: "birth_date_missing" as const }]) {
      const ks = retirementDetailRows(input(over)).map((r) => r.key);
      expect(ks, JSON.stringify(over)).not.toContain("safe_date_100");
      expect(ks, JSON.stringify(over)).not.toContain("safe_date_90");
    }
  });

  it("semilla y caminos: sin ellos el éxito no tiene precisión declarada ni se reproduce", () => {
    const r = retirementDetailRows(input()).find((x) => x.key === "seed_and_paths")!;
    expect(r.label).toBe("Semilla y caminos");
    // La semilla es un u64 y viaja como STRING: pasarla por `Number` perdería dígitos y el sorteo
    // dejaría de reproducirse.
    expect(r.value).toBe("2500 caminos · semilla 12345678901234567890");
  });

  it("sin sorteo (fecha por edad) no hay fila de semilla", () => {
    expect(
      retirementDetailRows(input({ seed: null, paths_used: null })).map((r) => r.key),
    ).not.toContain("seed_and_paths");
  });

  it("los avisos bajan aquí, con su tono y en su orden de precedencia", () => {
    const rows = retirementDetailRows(
      input({
        strategy: "retire_at_age",
        contribution_underfunded: true,
        warnings: ["no_volatility_declared", "coast_not_reachable"],
      }),
    );
    const notices = rows.filter((r) => r.key.startsWith("notice:"));
    expect(notices.map((n) => n.key)).toEqual([
      "notice:contribution_underfunded",
      "notice:coast_not_reachable",
      "notice:no_volatility_declared",
    ]);
    expect(notices[0]!.tone).toBe("danger");
    expect(notices[1]!.tone).toBe("warn");
    expect(notices[0]!.value).toContain("55");
  });

  it("las filas muertas del objetivo NO reaparecen en el Detalle", () => {
    const ks = retirementDetailRows(
      input({ fire_number_classic_today: "900000.0000" }),
    ).map((r) => r.key);
    for (const dead of [
      "target_nominal",
      "liquid_crossing",
      "disposable_today",
      "bridge_discount",
      "pension_coverage",
    ]) {
      expect(ks, dead).not.toContain(dead);
    }
  });

  it("todas las claves son únicas (son keys de React)", () => {
    const rows = retirementDetailRows(
      input({
        fire_number_classic_today: "900000.0000",
        safe_date_at_100_month_index: 300,
        safe_date_at_90_month_index: 168,
        contribution_underfunded: true,
        warnings: ["coast_not_reachable", "no_volatility_declared"],
      }),
    );
    expect(new Set(rows.map((r) => r.key)).size).toBe(rows.length);
  });
});
