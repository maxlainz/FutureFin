/**
 * **La FRASE del plan** (5.0.0, rediseño UX U1a; decisiones U7, U9 y U10 de #207).
 *
 * El resultado de una simulación de jubilación no es un número: es un hito con fecha, edad y
 * plazo, y con la estrategia mandando sobre qué significa cada uno. La versión anterior lo
 * repartía en tres tarjetas («Jubilación», «Años», «Edad») que el usuario tenía que volver a
 * juntar en su cabeza, y las tres decían cosas distintas según la estrategia sin avisar.
 *
 * U7/U9/U10 lo resuelven igual en las tres superficies: **una sola oración**, la misma en la
 * cabecera de Jubilación, en la tarjeta del Resumen y —en tercera persona— por cada miembro del
 * hogar. Por eso vive aquí y no en una vista: tres copias de la misma frase divergen al primer
 * cambio de estrategia.
 *
 * Reglas que este módulo NO puede romper:
 *
 *  1. **Todos los `*_month_index` viven en la MISMA rejilla** (mes 0 = hoy) y jamás son
 *     posiciones de array. Los plazos se calculan restando meses de esa rejilla —nunca contando
 *     puntos de `points[]`, que con `density=hybrid` no son meses.
 *  2. **La longitud del puente es `pension_start − jubilación`** (S8), no «meses desde hoy». La
 *     tarjeta anterior enseñaba lo segundo y el número era plausible, que es lo peor que le puede
 *     pasar a un error: un puente de 12 años se leía como 22 porque contaba también los años que
 *     faltan para jubilarse.
 *  3. **Un índice `null` no es un cero.** Cada estrategia tiene su frase de ausencia y ninguna se
 *     rellena con un guion: «no cruzas el objetivo en el horizonte» es un RESULTADO del plan.
 *  4. **El rotulador de meses lo inyecta la vista** (fechas o edades, con su zona horaria). Este
 *     módulo no resuelve calendarios.
 */

import type {
  HouseholdMemberProjectionApi,
  ProjectionSeriesApi,
  RetirementStrategyApi,
} from "../api/types";
import { formatMonthSpanEs } from "./duration";

/** Tono de la frase — el mismo vocabulario de estado que `plan-card.ts` y el design system. */
export type PlanSentenceTone = "ok" | "warn" | "danger";

/** El segundo hito que la estrategia añade a la jubilación, cuando lo tiene. */
export type PlanSecondaryKind = "coast" | "partial" | "pension";

/**
 * Las piezas con las que se armó la frase, publicadas para que la vista pueda reusarlas (un
 * subtítulo, un `aria-label`, la tira de fases) **sin volver a calcularlas**: recalcular el plazo
 * o el puente en la vista es cómo se abre la divergencia que S8 documenta.
 */
export type PlanSentenceParts = {
  strategy: RetirementStrategyApi | null;
  /** Mes EFECTIVO de jubilación en la rejilla (0 = hoy). `null` = no la hay en el horizonte. */
  retirementMonthIndex: number | null;
  retirementLabel: string | null;
  /** Años cumplidos en la jubilación. `null` sin fecha de nacimiento resoluble. */
  retirementAge: number | null;
  /** Meses de HOY a la jubilación. `null` sin jubilación; `0` = ya. */
  monthsToRetirement: number | null;
  secondaryKind: PlanSecondaryKind | null;
  secondaryMonthIndex: number | null;
  secondaryLabel: string | null;
  /** **S8**: `pension_start_month_index − jubilacion_month_index`. `null` si falta cualquiera
   *  de los dos; `0` o negativo = la pensión ya está en marcha al jubilarse. */
  bridgeMonths: number | null;
  /** El rojo de D17. `null` = la pregunta no aplica a esta estrategia, nunca `false`. */
  underfunded: boolean | null;
};

export type PlanSentence = {
  text: string;
  tone: PlanSentenceTone;
  parts: PlanSentenceParts;
};

/**
 * Cómo rotula el eje la vista. En `ages` el `monthLabel` ya devuelve una edad («a los 55»), así
 * que la coletilla «, a los N» sobraría y se omite: decir la edad dos veces en la misma oración
 * la hace ilegible.
 */
export type PlanSentenceAgeMode = "dates" | "ages";

/** Los campos de la serie que deciden la frase. Un `Pick` para que un test escriba el caso
 *  mínimo sin inventarse una proyección entera. */
export type PlanSentenceSeries = Pick<
  ProjectionSeriesApi,
  | "strategy"
  | "jubilacion_month_index"
  | "jubilacion_age"
  | "coast_fire_month_index"
  | "partial_retirement_month_index"
  | "pension_start_month_index"
  | "underfunded"
>;

export type PlanSentenceInput = {
  series: PlanSentenceSeries | null | undefined;
  /** Edad objetivo GUARDADA del perfil; se usa cuando la serie no publica `jubilacion_age`
   *  (sin fecha de nacimiento no hay edad calculada, pero la elegida sigue siendo la del plan). */
  targetRetirementAge: number | null;
  monthLabel: (monthIndex: number) => string;
  /** Default `dates`. */
  ageMode?: PlanSentenceAgeMode;
};

const EMPTY_PARTS: PlanSentenceParts = {
  strategy: null,
  retirementMonthIndex: null,
  retirementLabel: null,
  retirementAge: null,
  monthsToRetirement: null,
  secondaryKind: null,
  secondaryMonthIndex: null,
  secondaryLabel: null,
  bridgeMonths: null,
  underfunded: null,
};

function idx(v: number | null | undefined): number | null {
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

/**
 * La oración del plan, una por estrategia.
 *
 * | Estrategia | Frase | Ausencia |
 * |---|---|---|
 * | `asap` | «Te jubilas en {mes}, a los {edad} · dentro de {plazo}» | «No cruzas el objetivo en el horizonte» (danger) |
 * | `retire_at_age` | «Te jubilas en {mes}, a los {R}» (danger si `underfunded`) | «Falta tu edad de jubilación objetivo» (warn) |
 * | `coast` | «Puedes dejar de aportar en {mes} y jubilarte a los {R}» | «No hay mes coast: …» (warn) |
 * | `partial` | «Media jornada desde {mes}; jubilación total en {mes}» (danger si `underfunded`) | «… sin jubilación total en el horizonte» (danger) |
 * | `pension_bridge` | «Te jubilas en {mes} y vives del capital {plazo} hasta la pensión ({mes})» | «No cruzas el objetivo en el horizonte» (danger) |
 *
 * Una `strategy` nula (el agregado del hogar, o un backend viejo) usa la lectura genérica de
 * `asap`: el cruce es lo único que un plan sin estrategia declarada sabe decir.
 */
export function planSentence(input: PlanSentenceInput): PlanSentence {
  const s = input.series;
  if (!s) {
    return { text: "Sin plan que mostrar", tone: "warn", parts: { ...EMPTY_PARTS } };
  }
  const label = input.monthLabel;
  const ages = input.ageMode === "ages";
  const strategy = s.strategy ?? null;

  const mi = idx(s.jubilacion_month_index);
  const coastMi = idx(s.coast_fire_month_index);
  const partialMi = idx(s.partial_retirement_month_index);
  const pensionMi = idx(s.pension_start_month_index);
  const underfunded = s.underfunded ?? null;
  // La edad calculada manda sobre la elegida: con fecha de nacimiento las dos coinciden, y sin
  // ella la elegida sigue siendo el plan que el usuario pidió.
  const age = s.jubilacion_age ?? input.targetRetirementAge ?? null;

  const parts: PlanSentenceParts = {
    strategy,
    retirementMonthIndex: mi,
    retirementLabel: mi == null ? null : label(mi),
    retirementAge: age,
    monthsToRetirement: mi == null ? null : Math.max(0, mi),
    secondaryKind:
      strategy === "coast"
        ? "coast"
        : strategy === "partial"
          ? "partial"
          : pensionMi != null
            ? "pension"
            : null,
    secondaryMonthIndex: null,
    secondaryLabel: null,
    // S8: el puente es el TRAMO entre jubilación y pensión, no el plazo desde hoy.
    bridgeMonths: mi != null && pensionMi != null ? pensionMi - mi : null,
    underfunded,
  };
  if (parts.secondaryKind === "coast") parts.secondaryMonthIndex = coastMi;
  if (parts.secondaryKind === "partial") parts.secondaryMonthIndex = partialMi;
  if (parts.secondaryKind === "pension") parts.secondaryMonthIndex = pensionMi;
  if (parts.secondaryMonthIndex != null) {
    parts.secondaryLabel = label(parts.secondaryMonthIndex);
  }

  /** «, a los 55» — omitido sin edad y en modo edades (el rótulo ya la lleva). */
  const ageTail = age != null && !ages ? `, a los ${age}` : "";
  const done = (text: string, tone: PlanSentenceTone): PlanSentence => ({
    text,
    tone,
    parts,
  });
  const dangerIfUnderfunded = (t: PlanSentenceTone): PlanSentenceTone =>
    underfunded === true ? "danger" : t;

  switch (strategy) {
    case "retire_at_age": {
      if (age == null) return done("Falta tu edad de jubilación objetivo", "warn");
      const tone = dangerIfUnderfunded("ok");
      if (mi == null) return done(`Te jubilas a los ${age}`, tone);
      if (mi <= 0) return done(`Ya puedes jubilarte, a los ${age}`, tone);
      return done(`Te jubilas en ${label(mi)}, a los ${age}`, tone);
    }

    case "coast": {
      if (coastMi == null) {
        return done(
          "No hay mes coast: ni aportando todos los meses llegas al objetivo en tu edad",
          "warn",
        );
      }
      const tail =
        age != null
          ? `jubilarte a los ${age}`
          : mi != null
            ? `jubilarte en ${label(mi)}`
            : "jubilarte igual";
      if (coastMi <= 0) return done(`Ya puedes dejar de aportar y ${tail}`, "ok");
      return done(`Puedes dejar de aportar en ${label(coastMi)} y ${tail}`, "ok");
    }

    case "partial": {
      if (partialMi == null && mi == null) {
        return done("No cruzas el objetivo en el horizonte", "danger");
      }
      if (partialMi == null) {
        return done(
          `Sin fase de media jornada; te jubilas en ${label(mi as number)}${ageTail}`,
          dangerIfUnderfunded("ok"),
        );
      }
      if (mi == null) {
        return done(
          `Media jornada desde ${label(partialMi)}; sin jubilación total en el horizonte`,
          "danger",
        );
      }
      return done(
        `Media jornada desde ${label(partialMi)}; jubilación total en ${label(mi)}`,
        dangerIfUnderfunded("ok"),
      );
    }

    case "pension_bridge": {
      if (mi == null) return done("No cruzas el objetivo en el horizonte", "danger");
      if (pensionMi == null) {
        return done(
          `Te jubilas en ${label(mi)}${ageTail}; falta declarar tu pensión`,
          "warn",
        );
      }
      const bridge = pensionMi - mi;
      if (bridge <= 0) {
        return done(
          `Te jubilas en ${label(mi)}${ageTail} con la pensión ya en marcha (${label(pensionMi)})`,
          "ok",
        );
      }
      return done(
        `Te jubilas en ${label(mi)}${ageTail} y vives del capital ${formatMonthSpanEs(bridge)} hasta la pensión (${label(pensionMi)})`,
        "ok",
      );
    }

    // `asap` y el plan sin estrategia declarada comparten lectura: manda el cruce.
    default: {
      if (mi == null) return done("No cruzas el objetivo en el horizonte", "danger");
      if (mi <= 0) {
        return done("Ya puedes jubilarte: tu patrimonio ya cubre el objetivo", "ok");
      }
      return done(
        `Te jubilas en ${label(mi)}${ageTail} · dentro de ${formatMonthSpanEs(mi)}`,
        "ok",
      );
    }
  }
}

/** Los campos de un miembro del hogar que la frase en tercera persona necesita. */
export type MemberPlanSentenceMember = Pick<
  HouseholdMemberProjectionApi,
  "username" | "jubilacion_month_index" | "partial_retirement_month_index"
>;

/**
 * La misma frase, en **tercera persona**, para la vista Hogar (U10): «Max se quiere jubilar en 12
 * años. Mariona se quiere jubilar en 18 años y hacer media jornada a partir de 2039.»
 *
 * U10 pide números AGREGADOS y una oración por persona, nada más: el hogar no tiene plan propio
 * (no existe «la jubilación del hogar»), así que lo único honesto por miembro es su hito. Por eso
 * aquí no hay tarjetas por persona ni cifras al mes — solo la frase.
 *
 * El plazo va **en años desde hoy** y no en fecha porque es la única magnitud comparable entre
 * dos personas de edades distintas mirando la misma pantalla.
 */
export function memberPlanSentence(
  member: MemberPlanSentenceMember,
  monthLabel: (monthIndex: number) => string,
): string {
  const name = String(member.username ?? "").trim() || "Esta persona";
  const mi = idx(member.jubilacion_month_index);
  const partialMi = idx(member.partial_retirement_month_index);
  const partialTail =
    partialMi == null ? "" : ` y hacer media jornada a partir de ${monthLabel(partialMi)}`;

  if (mi == null) {
    // Sin jubilación en el horizonte la media jornada sigue siendo un hecho de su plan, así que
    // se enuncia igual — con «pero», que es lo que la relación entre las dos mitades dice.
    const tail =
      partialMi == null
        ? ""
        : `, pero hará media jornada a partir de ${monthLabel(partialMi)}`;
    return `${name} no cruza el objetivo en el horizonte${tail}.`;
  }
  if (mi <= 0) return `${name} ya se puede jubilar${partialTail}.`;
  return `${name} se quiere jubilar en ${formatMonthSpanEs(mi)}${partialTail}.`;
}
