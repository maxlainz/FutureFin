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
    const items = buildStructuralLegendItems({ hasFire: false, hasHistory: false });
    expect(items.map((i) => i.label)).toEqual([
      "Patrimonio neto",
      "Capital aportado",
    ]);
    expect(items[0]!.swatch).toBe("line");
    expect(items[1]!.swatch).toBe("dashed");
  });

  it("añade Objetivo FIRE y/o Histórico cuando aplican", () => {
    expect(
      buildStructuralLegendItems({ hasFire: true, hasHistory: false }).map(
        (i) => i.label,
      ),
    ).toEqual(["Patrimonio neto", "Capital aportado", "Objetivo FIRE"]);
    // Histórico va al final del bloque estructural (ANTES de los activos, que
    // se concatenan detrás en el componente).
    expect(
      buildStructuralLegendItems({ hasFire: true, hasHistory: true }).map(
        (i) => i.label,
      ),
    ).toEqual([
      "Patrimonio neto",
      "Capital aportado",
      "Objetivo FIRE",
      "Histórico",
    ]);
  });

  it("todos los colores son tokens var(--…)", () => {
    for (const i of buildStructuralLegendItems({ hasFire: true, hasHistory: true })) {
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
