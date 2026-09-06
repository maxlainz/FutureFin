/**
 * Los AVISOS de la vista Jubilación, fijados (5.0.0, modelo v2 §2.4/§3).
 *
 * Sin este test la traducción de `warnings[]` vive solo en un `if` dentro de una vista de 1.700
 * líneas, y el fallo que produce no rompe nada: un plan que el motor da por imposible se pinta
 * como uno normal, o al revés.
 *
 * Tres cosas se fijan aquí porque son decisiones y no accidentes:
 *
 *  1. **La precedencia**: primero lo que invalida el plan (rojo), después lo que lo degrada.
 *  2. **`contribution_underfunded` es un BOOLEANO de tres valores**: `true` es rojo, `false` es
 *     «llegas», y `null` es «la pregunta no aplica a esta estrategia». Colapsar los dos últimos
 *     pinta de verde un plan que nadie ha evaluado.
 *  3. **`no_volatility_declared` avisa del INDICADOR, no del plan**: sin σ el sorteo no dispersa y
 *     el éxito sale 0 % o 100 % por construcción. Ese 100 % es la lectura más cara de la pantalla.
 *
 * La cabecera de tarjetas y el «Detalle» se prueban en `retirement-tiles-v2.test.ts`.
 */

import { describe, expect, it } from "vitest";
import {
  buildRetirementNotices,
  type RetirementNoticeSeries,
} from "./retirement-tiles";

function series(over: Partial<RetirementNoticeSeries> = {}): RetirementNoticeSeries {
  return { contribution_underfunded: null, warnings: [], ...over };
}

const notices = (
  over: Partial<RetirementNoticeSeries> = {},
  targetAge: number | null = 55,
) => buildRetirementNotices(series(over), targetAge);

const codes = (
  over: Partial<RetirementNoticeSeries> = {},
  targetAge: number | null = 55,
) => notices(over, targetAge).map((n) => n.code);

describe("infra-financiado — el único rojo", () => {
  it("sale del BOOLEANO, no de un literal de `warnings[]`", () => {
    // El aviso `retire_at_age_underfunded` murió con el objetivo; lo que queda es
    // `contribution_underfunded`, que el solve de aportación mínima publica.
    expect(codes({ contribution_underfunded: true })).toEqual(["contribution_underfunded"]);
    expect(notices({ contribution_underfunded: true })[0]!.tone).toBe("danger");
  });

  it("nombra la edad objetivo cuando se conoce", () => {
    expect(notices({ contribution_underfunded: true }, 55)[0]!.text).toContain("55 años");
  });

  it("sin edad conocida el rojo sigue saliendo, sin inventarse una cifra", () => {
    const n = notices({ contribution_underfunded: true }, null)[0]!;
    expect(n.text).toContain("tu edad objetivo");
    expect(n.text).not.toMatch(/\d/);
  });

  it("dice que ni ahorrándolo todo se llega, que es lo que el booleano significa", () => {
    expect(notices({ contribution_underfunded: true })[0]!.text).toContain(
      "ni invirtiendo todo tu sobrante",
    );
  });

  it("`false` NO es un aviso: es «llegas»", () => {
    expect(codes({ contribution_underfunded: false })).toEqual([]);
  });

  it("`null` tampoco: es «la pregunta no aplica a esta estrategia»", () => {
    expect(codes({ contribution_underfunded: null })).toEqual([]);
  });
});

describe("los fallos de solve de cada estrategia", () => {
  it("coast inalcanzable", () => {
    const n = notices({ warnings: ["coast_not_reachable"] });
    expect(n.map((x) => x.code)).toEqual(["coast_not_reachable"]);
    expect(n[0]!.tone).toBe("warn");
    expect(n[0]!.text).toContain("No hay mes coast");
  });

  it("la jornada reducida que no empieza nunca", () => {
    const n = notices({ warnings: ["partial_never_starts"] });
    expect(n.map((x) => x.code)).toEqual(["partial_never_starts"]);
    expect(n[0]!.text).toContain("no empieza en ningún mes");
  });

  it("la jornada reducida que empieza y de la que no se sale — caso DISTINTO", () => {
    // `partial_never_starts` y `partial_never_fully_retires` describen cosas opuestas: no arrancar
    // nunca, o arrancar y no jubilarse jamás. Compartir copy borraría la diferencia.
    const n = notices({ warnings: ["partial_never_fully_retires"] });
    expect(n.map((x) => x.code)).toEqual(["partial_never_fully_retires"]);
    expect(n[0]!.text).toContain("nunca te jubilas del todo");
    expect(n[0]!.text).not.toContain("no empieza");
  });

  it("los dos avisos de la fase parcial pueden convivir sin fundirse", () => {
    expect(
      codes({ warnings: ["partial_never_fully_retires", "partial_never_starts"] }),
    ).toEqual(["partial_never_starts", "partial_never_fully_retires"]);
  });
});

describe("los avisos del modelo v2", () => {
  it("la pensión que cae dentro de la jornada reducida y no se cobra entera (B9)", () => {
    const n = notices({ warnings: ["pension_unpaid_during_partial"] });
    expect(n.map((x) => x.code)).toEqual(["pension_unpaid_during_partial"]);
    expect(n[0]!.text).toContain("jornada reducida");
    expect(n[0]!.text).toContain("fracción");
  });

  it("«sin volatilidad declarada»: avisa de que el ÉXITO no mide riesgo, no de la carencia", () => {
    // C5. Sin σ los 2.500 caminos son el mismo camino: el éxito sale 0 % o 100 % por
    // construcción, y ese 100 % es la lectura más cara de toda la pantalla. Describir solo la
    // carencia («no has declarado volatilidad») dejaría al usuario creyendo su 100 %.
    const n = notices({ warnings: ["no_volatility_declared"] })[0]!;
    expect(n.code).toBe("no_volatility_declared");
    expect(n.tone).toBe("warn");
    expect(n.text).toContain("el sorteo no dispersa");
    expect(n.text).toContain("no mide riesgo");
  });

  it("la migración de «Puente hasta la pensión» pide REVISARLO, no solo lo informa", () => {
    // C7: el literal `pension_bridge` dejó de ser estrategia y lo guardado migró solo. Callarlo
    // cambiaría el plan de alguien sin decírselo.
    const n = notices({ warnings: ["strategy_pension_bridge_migrated"] })[0]!;
    expect(n.code).toBe("strategy_pension_bridge_migrated");
    expect(n.text).toContain("«Puente hasta la pensión»");
    expect(n.text).toContain("«Cuanto antes»");
    expect(n.text).toContain("revísalo");
  });

  it("la edad objetivo que falta: el plan degrada a «Cuanto antes» y se dice", () => {
    expect(codes({ warnings: ["target_retirement_age_missing"] })).toEqual([
      "target_retirement_age_missing",
    ]);
  });
});

describe("lo que YA NO se avisa", () => {
  it("los literales del objetivo muerto no producen aviso", () => {
    // `bridge_discount_*` y `partial_phase_capital_shrinking` describían un objetivo descontado y
    // un hueco de perpetuidad que el modelo v2 retiró del contrato. Si un backend viejo los
    // mandara, traducirlos resucitaría en pantalla un modelo que ya no corre.
    expect(
      codes({
        warnings: [
          "bridge_discount_no_liquid_assets",
          "bridge_discount_clamped",
          "partial_phase_capital_shrinking",
          "retire_at_age_underfunded",
        ],
      }),
    ).toEqual([]);
  });

  it("`birth_date_missing` NO genera aviso aquí: lo dicen las tarjetas con su razón", () => {
    expect(codes({ warnings: ["birth_date_missing"] })).toEqual([]);
  });

  it("un literal desconocido del servidor no rompe nada ni inventa un aviso", () => {
    expect(codes({ warnings: ["algo_nuevo"] })).toEqual([]);
  });
});

describe("precedencia y forma", () => {
  it("el rojo va SIEMPRE primero, antes de los avisos que solo matizan", () => {
    expect(
      codes({
        contribution_underfunded: true,
        warnings: ["no_volatility_declared", "coast_not_reachable"],
      }),
    ).toEqual([
      "contribution_underfunded",
      "coast_not_reachable",
      "no_volatility_declared",
    ]);
  });

  it("el orden NO depende del orden en que lleguen los `warnings[]`", () => {
    const a = codes({ warnings: ["no_volatility_declared", "coast_not_reachable"] });
    const b = codes({ warnings: ["coast_not_reachable", "no_volatility_declared"] });
    expect(a).toEqual(b);
  });

  it("ningún aviso se duplica aunque el literal llegue repetido", () => {
    expect(codes({ warnings: ["coast_not_reachable", "coast_not_reachable"] })).toEqual([
      "coast_not_reachable",
    ]);
  });

  it("cada aviso tiene un tono declarado y un texto con sustancia", () => {
    const all = notices({
      contribution_underfunded: true,
      warnings: [
        "coast_not_reachable",
        "partial_never_starts",
        "partial_never_fully_retires",
        "pension_unpaid_during_partial",
        "no_volatility_declared",
        "strategy_pension_bridge_migrated",
        "target_retirement_age_missing",
      ],
    });
    expect(all).toHaveLength(8);
    for (const n of all) {
      expect(["danger", "warn"], n.code).toContain(n.tone);
      expect(n.text.length, n.code).toBeGreaterThan(40);
    }
    // Los códigos son keys de React en el «Detalle»: no puede haber dos iguales.
    expect(new Set(all.map((n) => n.code)).size).toBe(all.length);
  });
});

describe("sin serie", () => {
  it("no hay avisos mientras la proyección no ha llegado", () => {
    expect(buildRetirementNotices(null, 55)).toEqual([]);
    expect(buildRetirementNotices(undefined, 55)).toEqual([]);
  });
});
