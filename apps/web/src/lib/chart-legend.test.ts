import { describe, expect, it } from "vitest";
import {
  applyLegendCollapse,
  assetOwnerNameById,
  buildAssetLegendItems,
  buildStructuralLegendItems,
  collapsedAssetLegendCap,
  legendOrderByPeakDesc,
  topAssetTooltipRows,
} from "./chart-legend";

describe("buildStructuralLegendItems", () => {
  it("base: Patrimonio neto + Capital aportado, en ese orden", () => {
    const items = buildStructuralLegendItems({
      hasNeededCapital: false,
      hasHistory: false,
    });
    expect(items.map((i) => i.label)).toEqual([
      "Patrimonio neto",
      "Capital aportado",
    ]);
    expect(items[0]!.swatch).toBe("line");
    expect(items[1]!.swatch).toBe("dashed");
  });

  // INVERTIDO por el modelo v2 (C4): «Objetivo FIRE» ya no es una entrada posible —
  // `fire_target_series` salió del contrato y no hay objetivo que cruzar—, y en su hueco van la
  // curva «Capital necesario» y la MARCA de la fecha válida.
  it("nunca vuelve «Objetivo FIRE»: en su sitio va «Capital necesario»", () => {
    const items = buildStructuralLegendItems({
      hasNeededCapital: true,
      hasHistory: false,
    });
    expect(items.map((i) => i.label)).toEqual([
      "Patrimonio neto",
      "Capital aportado",
      "Capital necesario",
    ]);
    expect(items.some((i) => i.label.includes("Objetivo"))).toBe(false);
    // Discontinua, y con el token de la curva que el chart pinta — no un color escrito aparte.
    expect(items[2]!.swatch).toBe("dashed");
    expect(items[2]!.color).toBe("var(--proj-required)");
  });

  it("la marca de la fecha válida entra con SU rótulo y muestra propia", () => {
    const items = buildStructuralLegendItems({
      hasNeededCapital: true,
      hasHistory: false,
      validDateMarkLabel: "Fecha válida · 95 de cada 100",
    });
    expect(items.map((i) => i.label)).toEqual([
      "Patrimonio neto",
      "Capital aportado",
      "Capital necesario",
      "Fecha válida · 95 de cada 100",
    ]);
    // `mark` y no `line`: es un instante, no una serie, y la muestra lo dice.
    expect(items[3]!.swatch).toBe("mark");
    expect(items[3]!.color).toBe("var(--ff-accent)");
  });

  // Sin fecha válida (`not_reachable`) el chart no pinta marca: la leyenda tampoco puede
  // reservarle hueco, o rotularía una línea que no está. La explicación va en la NOTA del pie,
  // que no es un ítem de leyenda.
  it("sin rótulo de marca (ausente, null o en blanco) no hay entrada de marca", () => {
    for (const label of [undefined, null, "", "   "]) {
      const items = buildStructuralLegendItems({
        hasNeededCapital: false,
        hasHistory: false,
        validDateMarkLabel: label,
      });
      expect(items.map((i) => i.label)).toEqual([
        "Patrimonio neto",
        "Capital aportado",
      ]);
    }
  });

  it("historyIsAssetsOnly renombra el tramo pasado a «Activos (histórico)»", () => {
    // Sin patrimonio neto histórico (el pasivo no está fotografiado entero) el chart pinta
    // activos: la leyenda tiene que decirlo, o las dos mitades de la curva se leen como la misma
    // magnitud — el error que el `net_worth: null` del servidor acaba de cerrar.
    const items = buildStructuralLegendItems({
      hasNeededCapital: false,
      hasHistory: true,
      historyIsAssetsOnly: true,
    });
    expect(items.map((i) => i.label)).toEqual([
      "Patrimonio neto",
      "Capital aportado",
      "Activos (histórico)",
    ]);
    // El color no cambia: sigue siendo el token del tramo pasado.
    expect(items[2]!.color).toBe("var(--proj-nw-past)");
    // Y lleva explicación en el title (tooltip nativo del chip).
    expect(items[2]!.title).toContain("activos");

    // Omitido o false → etiqueta de siempre, sin title propio.
    const plain = buildStructuralLegendItems({
      hasNeededCapital: false,
      hasHistory: true,
    });
    expect(plain[2]!.label).toBe("Histórico");
    expect(plain[2]!.title).toBeUndefined();
  });

  // El histórico cierra SIEMPRE el bloque estructural (los activos se concatenan detrás en el
  // componente): un orden que cambiara con las props movería la leyenda entera bajo el usuario.
  it("con todo activo, el orden es nw · cc · necesario · marca · histórico", () => {
    const items = buildStructuralLegendItems({
      hasNeededCapital: true,
      hasHistory: true,
      validDateMarkLabel: "Fecha válida · 95 de cada 100",
    });
    expect(items.map((i) => i.key)).toEqual([
      "nw",
      "cc",
      "needed_capital",
      "safe_date",
      "hist",
    ]);
  });

  it("todos los colores son tokens var(--…)", () => {
    for (const i of buildStructuralLegendItems({
      hasNeededCapital: true,
      hasHistory: true,
      validDateMarkLabel: "Fecha válida",
    })) {
      expect(i.color.startsWith("var(--")).toBe(true);
    }
  });
});

describe("legendOrderByPeakDesc", () => {
  it("ordena por peak descendente conservando el colorIndex del orden de pintado", () => {
    // Orden de pintado (peak asc): C(10) → A(50) → B(200)
    const painted = [
      { name: "C", peak: 10 },
      { name: "A", peak: 50 },
      { name: "B", peak: 200 },
    ];
    const out = legendOrderByPeakDesc(painted);
    expect(out.map((o) => o.item.name)).toEqual(["B", "A", "C"]);
    expect(out.map((o) => o.colorIndex)).toEqual([2, 1, 0]);
  });

  it("desempata por nombre ascendente", () => {
    const out = legendOrderByPeakDesc([
      { name: "Zeta", peak: 100 },
      { name: "Alfa", peak: 100 },
    ]);
    expect(out.map((o) => o.item.name)).toEqual(["Alfa", "Zeta"]);
  });
});

describe("buildAssetLegendItems", () => {
  const entry = (id: string, name: string, colorIndex: number) => ({
    id,
    name,
    colorIndex,
  });

  it("color = ASSET_LINE_COLORS[colorIndex % 10]; key = asset_id; swatch area", () => {
    const items = buildAssetLegendItems([
      entry("a", "Fondo", 0),
      entry("b", "Casa", 10), // módulo 10 → mismo color que colorIndex 0
    ]);
    expect(items[0]!.color).toBe("var(--proj-area-1)");
    expect(items[1]!.color).toBe("var(--proj-area-1)");
    expect(items[0]!.key).toBe("a");
    expect(items.every((i) => i.swatch === "area")).toBe(true);
  });

  it("sin mapa de owners → sin sufijos", () => {
    const items = buildAssetLegendItems(
      [entry("a", "Cuenta", 0), entry("b", "Cuenta", 1)],
      null,
    );
    expect(items.map((i) => i.label)).toEqual(["Cuenta", "Cuenta"]);
  });

  it("nombre único → sin sufijo aunque el owner sea resoluble", () => {
    const items = buildAssetLegendItems([entry("a", "Fondo", 0)], { a: "Max" });
    expect(items[0]!.label).toBe("Fondo");
  });

  it("duplicado con todos los owners resolubles → sufijo en todos", () => {
    const items = buildAssetLegendItems(
      [entry("a", "Cuenta corriente", 0), entry("b", "Cuenta corriente", 1)],
      { a: "Max", b: "Ana" },
    );
    expect(items.map((i) => i.label)).toEqual([
      "Cuenta corriente · Max",
      "Cuenta corriente · Ana",
    ]);
  });

  it("duplicado con un activo ACTUAL sin owner resoluble → el grupo entero sin sufijo (todo-o-nada)", () => {
    const items = buildAssetLegendItems(
      [entry("a", "Cuenta", 0), entry("b", "Cuenta", 1), entry("c", "Fondo", 2)],
      { a: "Max", b: null, c: "Max" }, // b existe en /v1/assets pero sin owner
    );
    expect(items.map((i) => i.label)).toEqual(["Cuenta", "Cuenta", "Fondo"]);
  });

  it("las series solo-históricas (ausentes del mapa) ni sufijan ni vetan", () => {
    // b viene de snapshots: no existe en /v1/assets. El activo actual conserva su sufijo.
    const items = buildAssetLegendItems(
      [entry("a", "Cuenta", 0), entry("b", "Cuenta", 1)],
      { a: "Max" },
    );
    expect(items.map((i) => i.label)).toEqual(["Cuenta · Max", "Cuenta"]);
  });

  it("la agrupación de duplicados ignora mayúsculas y diacríticos", () => {
    const items = buildAssetLegendItems(
      [entry("a", "Café", 0), entry("b", "cafe", 1)],
      { a: "Max", b: "Ana" },
    );
    expect(items.map((i) => i.label)).toEqual(["Café · Max", "cafe · Ana"]);
  });
});

describe("assetOwnerNameById", () => {
  const members = [
    { user_id: "u1", username: "Max" },
    { user_id: "u2", username: "Ana" },
  ];

  it("todo activo actual tiene entrada; null cuando el owner no es resoluble", () => {
    const map = assetOwnerNameById(
      [
        { id: "a", owner_user_id: "u1" },
        { id: "b", owner_user_id: null },
        { id: "c" },
        { id: "d", owner_user_id: "u-borrado" },
      ],
      members,
    );
    expect(map).toEqual({ a: "Max", b: null, c: null, d: null });
  });

  it("sin assets → mapa vacío; sin members → todo null (nada resoluble)", () => {
    expect(assetOwnerNameById([], members)).toEqual({});
    expect(assetOwnerNameById([{ id: "a", owner_user_id: "u1" }], [])).toEqual({
      a: null,
    });
  });
});

describe("collapsedAssetLegendCap", () => {
  it("breakpoints canónicos: ≤640 → 3, ≤720 → 4, resto → 6", () => {
    expect(collapsedAssetLegendCap(360)).toBe(3);
    expect(collapsedAssetLegendCap(640)).toBe(3);
    expect(collapsedAssetLegendCap(641)).toBe(4);
    expect(collapsedAssetLegendCap(720)).toBe(4);
    expect(collapsedAssetLegendCap(721)).toBe(6);
    expect(collapsedAssetLegendCap(1440)).toBe(6);
  });
});

describe("applyLegendCollapse", () => {
  it("total ≤ cap → todo visible", () => {
    expect(applyLegendCollapse(3, 4)).toEqual({ visibleCount: 3, hiddenCount: 0 });
    expect(applyLegendCollapse(0, 4)).toEqual({ visibleCount: 0, hiddenCount: 0 });
  });

  it("total === cap+1 → todo visible (no escondas uno solo)", () => {
    expect(applyLegendCollapse(5, 4)).toEqual({ visibleCount: 5, hiddenCount: 0 });
  });

  it("total > cap+1 → visible = cap, hidden = resto", () => {
    expect(applyLegendCollapse(26, 6)).toEqual({
      visibleCount: 6,
      hiddenCount: 20,
    });
  });

  it("cap < 1 se trata como 1", () => {
    expect(applyLegendCollapse(5, 0)).toEqual({ visibleCount: 1, hiddenCount: 4 });
  });
});

describe("topAssetTooltipRows", () => {
  const row = (id: string, value: number | null | undefined) => ({
    id,
    label: id,
    value,
  });

  it("ordena por |valor| descendente y corta en el límite", () => {
    const { shown, hiddenCount, hiddenTotal } = topAssetTooltipRows(
      [row("a", 10), row("b", -300), row("c", 50), row("d", 200)],
      2,
    );
    expect(shown.map((r) => r.id)).toEqual(["b", "d"]);
    expect(hiddenCount).toBe(2);
    // Suma CRUDA de los ocultos (no absolutos): 50 + 10.
    expect(hiddenTotal).toBe(60);
  });

  it("descarta 0, null, undefined y NaN (relleno de solo-históricos)", () => {
    const { shown, hiddenCount } = topAssetTooltipRows([
      row("a", 0),
      row("b", null),
      row("c", undefined),
      row("d", Number.NaN),
      row("e", 5),
    ]);
    expect(shown.map((r) => r.id)).toEqual(["e"]);
    expect(hiddenCount).toBe(0);
  });

  it("menos filas que el límite → hiddenCount 0 y sin fila «Otros»", () => {
    const out = topAssetTooltipRows([row("a", 1), row("b", 2)]);
    expect(out.shown).toHaveLength(2);
    expect(out.hiddenCount).toBe(0);
    expect(out.hiddenTotal).toBe(0);
  });

  it("límite por defecto = 5", () => {
    const rows = Array.from({ length: 8 }, (_, i) => row(`r${i}`, i + 1));
    const out = topAssetTooltipRows(rows);
    expect(out.shown).toHaveLength(5);
    expect(out.hiddenCount).toBe(3);
    expect(out.hiddenTotal).toBe(1 + 2 + 3);
  });
});
