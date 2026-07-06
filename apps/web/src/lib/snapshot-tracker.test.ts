import { describe, expect, it } from "vitest";
import {
  type EditLog,
  SNAPSHOT_EDIT_WINDOW_MS,
  liquidCoverageComplete,
  pruneEditLog,
} from "./snapshot-tracker";

const NOW = 10_000_000;

describe("SNAPSHOT_EDIT_WINDOW_MS", () => {
  it("es una hora", () => {
    expect(SNAPSHOT_EDIT_WINDOW_MS).toBe(3_600_000);
  });
});

describe("liquidCoverageComplete", () => {
  it("conjunto de líquidos vacío → false", () => {
    expect(liquidCoverageComplete(new Map(), [], NOW)).toBe(false);
  });

  it("todos los líquidos editados dentro de la ventana → true", () => {
    const log: EditLog = new Map([
      ["a", NOW - 1000],
      ["b", NOW - 500],
    ]);
    expect(liquidCoverageComplete(log, ["a", "b"], NOW)).toBe(true);
  });

  it("un id nuevo en el set sin edición todavía → false", () => {
    const log: EditLog = new Map([["a", NOW - 1000]]);
    expect(liquidCoverageComplete(log, ["a", "c"], NOW)).toBe(false);
  });

  it("una entrada caducada cuenta como no cubierta → false", () => {
    const log: EditLog = new Map([
      ["a", NOW - 1000],
      ["b", NOW - (SNAPSHOT_EDIT_WINDOW_MS + 1)],
    ]);
    expect(liquidCoverageComplete(log, ["a", "b"], NOW)).toBe(false);
  });
});

describe("pruneEditLog", () => {
  it("elimina las entradas más viejas que la ventana y no muta el original", () => {
    const stale = NOW - (SNAPSHOT_EDIT_WINDOW_MS + 1);
    const log: EditLog = new Map([
      ["a", NOW - 1000],
      ["b", stale],
    ]);
    const pruned = pruneEditLog(log, NOW);
    // Copia: el original conserva ambas entradas.
    expect(log.size).toBe(2);
    expect(pruned).not.toBe(log);
    expect(pruned.has("a")).toBe(true);
    expect(pruned.has("b")).toBe(false);
  });

  it("tras prune, un líquido cuya edición caducó deja la cobertura incompleta", () => {
    const stale = NOW - (SNAPSHOT_EDIT_WINDOW_MS + 1);
    const log: EditLog = new Map([
      ["a", NOW - 1000],
      ["b", stale],
    ]);
    const pruned = pruneEditLog(log, NOW);
    expect(liquidCoverageComplete(pruned, ["a", "b"], NOW)).toBe(false);
  });

  it("conserva las entradas justo en el borde de la ventana", () => {
    const log: EditLog = new Map([["a", NOW - SNAPSHOT_EDIT_WINDOW_MS]]);
    const pruned = pruneEditLog(log, NOW);
    expect(pruned.has("a")).toBe(true);
  });
});
