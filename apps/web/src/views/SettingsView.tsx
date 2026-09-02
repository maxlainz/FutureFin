import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type Dispatch,
  type FormEvent,
  type SetStateAction,
} from "react";
import type {
  CategoryRow,
  CategoryScope,
  FfbackupImportPreviewResponse,
  FireSettingsApi,
  HealthResponse,
  InstallationAccess,
  UserResponse,
} from "../api/types";
import { HelpPopover } from "../components/HelpPopover";
import { healthStatusLabel, roleLabel } from "../lib/enumLabels";
import { parseDisplayDecimal, toApiDecimalString } from "../lib/format";
import { HELP_TEXTS } from "../lib/helpTexts";
import { Modal, ModalFormError } from "../components/Modal";
import { RowEditIcon, RowTrashIcon } from "../components/icons";
import { Switch } from "../components/Switch";
import { AccountCard } from "../components/AccountCard";
import { ApiTokensPanel } from "./ApiTokensPanel";
import { HistorySettingsPanel } from "./HistorySettingsPanel";
import { OAuthConnectionsPanel } from "./OAuthConnectionsPanel";
import { ThemeToggle } from "../components/ThemeToggle";
import type { ThemePref } from "../lib/theme";
import {
  DEFAULT_ES_TAX_BRACKETS_API,
  normalizeInstallationFireSettings,
  parseSavingsSource,
  savingsSourceUsesTransactions,
} from "../lib/fire";
import {
  SETTINGS_SUBTAB_LABEL,
  TAB_PATH,
  type SettingsSubTabId,
} from "../lib/navigation";
import { appUrl } from "../lib/basePath";

const CATEGORY_SCOPES: CategoryScope[] = ["asset", "liability", "income", "expense"];

const CATEGORY_SCOPE_LABEL: Record<CategoryScope, string> = {
  asset: "Activos",
  liability: "Pasivos",
  income: "Ingresos",
  expense: "Gastos",
};

export function SettingsView({
  user,
  themePref,
  onChangeTheme,
  onReopenOnboarding,
  onLogout,
  onEditAccount,
  authBusy,
  installation,
  installationBusy,
  categoryModalOpen,
  categoryRenameModalOpen,
  closeCategoryModal,
  openNewCategoryModal,
  closeRenameCategoryModal,
  openRenameCategoryModal,
  calendarTzDraft,
  setCalendarTzDraft,
  calendarTzSaving,
  currencySaving,
  onChangeCurrency,
  saveInstallationCalendarTz,
  projectionInflationPctDraft,
  setProjectionInflationPctDraft,
  showAgeModeDraft,
  setShowAgeModeDraft,
  installationProjectionSaving,
  saveInstallationProjection,
  onSaveFire,
  health,
  healthError,
  categoriesError,
  hasMembership,
  canEditCategories,
  canEditHistory,
  scopeReadOnly,
  currencyIso,
  calendarTz,
  onHistoryMutated,
  isOwner,
  mcpWriteEnabled,
  mcpWriteSaving,
  onToggleMcpWrite,
  settingsSubTab,
  navigateSettingsSubTab,
  navigate,
  visibleSettingsSubTabs,
  pendingUsers,
  pendingUsersBusy,
  approveRoles,
  setApproveRoles,
  approveBusy,
  approvePendingUser,
  categories,
  categoriesBusy,
  categoryScopeFilter,
  setCategoryScopeFilter,
  newCatScope,
  setNewCatScope,
  newCatName,
  setNewCatName,
  categorySaving,
  createCategory,
  makeCategoryFallback,
  openCategoryDeleteModal,
  categoryDeleteModalOpen,
  categoryDeletePending,
  categoryRemapToId,
  setCategoryRemapToId,
  closeCategoryDeleteModal,
  confirmDeleteCategory,
  editingCategoryId,
  editCategoryName,
  setEditCategoryName,
  saveCategoryEdit,
  ffbackupExportModalOpen,
  ffbackupExportPassword,
  setFfbackupExportPassword,
  ffbackupExportBusy,
  ffbackupExportError,
  openFfbackupExportModal,
  closeFfbackupExportModal,
  runFfbackupExport,
  ffbackupImportModalOpen,
  ffbackupImportFile,
  setFfbackupImportFile,
  ffbackupImportPassword,
  setFfbackupImportPassword,
  ffbackupImportBusy,
  ffbackupImportError,
  ffbackupImportPreview,
  ffbackupImportDone,
  openFfbackupImportModal,
  closeFfbackupImportModal,
  runFfbackupImportPreview,
  runFfbackupImportApply,
}: {
  user: UserResponse;
  themePref: ThemePref;
  onChangeTheme: (next: ThemePref) => void;
  /** Vuelve a abrir el asistente de primera vez. Solo lo recibe el propietario. */
  onReopenOnboarding?: () => void;
  onLogout: () => void;
  onEditAccount: () => void;
  authBusy: boolean;
  installation: InstallationAccess | null;
  installationBusy: boolean;
  categoryModalOpen: boolean;
  categoryRenameModalOpen: boolean;
  closeCategoryModal: () => void;
  openNewCategoryModal: () => void;
  closeRenameCategoryModal: () => void;
  openRenameCategoryModal: (row: CategoryRow) => void;
  calendarTzDraft: string;
  setCalendarTzDraft: Dispatch<SetStateAction<string>>;
  calendarTzSaving: boolean;
  currencySaving?: boolean;
  /** Cambia la divisa base del hogar (owner-only). */
  onChangeCurrency?: (code: string) => void;
  saveInstallationCalendarTz: (e?: FormEvent) => void;
  projectionInflationPctDraft: string;
  setProjectionInflationPctDraft: Dispatch<SetStateAction<string>>;
  showAgeModeDraft: "dates" | "ages";
  setShowAgeModeDraft: Dispatch<SetStateAction<"dates" | "ages">>;
  installationProjectionSaving: boolean;
  saveInstallationProjection: (e?: FormEvent) => void;
  onSaveFire: (fs: FireSettingsApi) => Promise<void>;
  health: HealthResponse | null;
  healthError: string | null;
  categoriesError: string | null;
  hasMembership: boolean;
  canEditCategories: boolean;
  canEditHistory: boolean;
  /** Vista Hogar (D9/D32): agregado de solo lectura — el plan se edita desde la vista «Yo». */
  scopeReadOnly: boolean;
  currencyIso: string;
  /** Zona horaria (IANA) de la instalación; el panel de histórico deriva «hoy» de ella. */
  calendarTz: string;
  onHistoryMutated: () => void;
  isOwner: boolean;
  /** Kill-switch vivo de la escritura vía MCP (Ajustes → MCP; editable solo por el owner). */
  mcpWriteEnabled: boolean;
  mcpWriteSaving: boolean;
  onToggleMcpWrite: (enabled: boolean) => void;
  settingsSubTab: SettingsSubTabId;
  navigateSettingsSubTab: (id: SettingsSubTabId) => void;
  /** Navegación de la app (la usa el puntero a Jubilación del panel «Plan»). */
  navigate: (path: string, replace?: boolean) => void;
  visibleSettingsSubTabs: SettingsSubTabId[];
  pendingUsers: UserResponse[];
  pendingUsersBusy: boolean;
  approveRoles: Record<string, "member" | "viewer">;
  setApproveRoles: Dispatch<
    SetStateAction<Record<string, "member" | "viewer">>
  >;
  approveBusy: boolean;
  approvePendingUser: (userId: string) => void;
  categories: CategoryRow[];
  categoriesBusy: boolean;
  categoryScopeFilter: CategoryScope | "all";
  setCategoryScopeFilter: Dispatch<
    SetStateAction<CategoryScope | "all">
  >;
  newCatScope: CategoryScope;
  setNewCatScope: Dispatch<SetStateAction<CategoryScope>>;
  newCatName: string;
  setNewCatName: Dispatch<SetStateAction<string>>;
  categorySaving: boolean;
  createCategory: (e: FormEvent) => void;
  /** Traslada la marca «por defecto» del ámbito a esta categoría (`PATCH {is_fallback:true}`).
   *  Solo tiene sentido en ingreso/gasto: los otros dos ámbitos no tienen categoría por defecto. */
  makeCategoryFallback: (row: CategoryRow) => void;
  openCategoryDeleteModal: (row: CategoryRow) => void;
  categoryDeleteModalOpen: boolean;
  categoryDeletePending: CategoryRow | null;
  categoryRemapToId: string;
  setCategoryRemapToId: Dispatch<SetStateAction<string>>;
  closeCategoryDeleteModal: () => void;
  confirmDeleteCategory: () => void;
  editingCategoryId: string | null;
  editCategoryName: string;
  setEditCategoryName: Dispatch<SetStateAction<string>>;
  saveCategoryEdit: (id: string) => void;
  ffbackupExportModalOpen: boolean;
  ffbackupExportPassword: string;
  setFfbackupExportPassword: Dispatch<SetStateAction<string>>;
  ffbackupExportBusy: boolean;
  ffbackupExportError: string | null;
  openFfbackupExportModal: () => void;
  closeFfbackupExportModal: () => void;
  runFfbackupExport: (e: FormEvent) => void;
  ffbackupImportModalOpen: boolean;
  ffbackupImportFile: File | null;
  setFfbackupImportFile: Dispatch<SetStateAction<File | null>>;
  ffbackupImportPassword: string;
  setFfbackupImportPassword: Dispatch<SetStateAction<string>>;
  ffbackupImportBusy: boolean;
  ffbackupImportError: string | null;
  ffbackupImportPreview: FfbackupImportPreviewResponse | null;
  ffbackupImportDone: string | null;
  openFfbackupImportModal: () => void;
  closeFfbackupImportModal: () => void;
  runFfbackupImportPreview: (e: FormEvent) => void;
  runFfbackupImportApply: () => void;
}) {
  const renamingCat =
    editingCategoryId === null
      ? undefined
      : categories.find((x) => x.id === editingCategoryId);

  const filteredCategories =
    categoryScopeFilter === "all"
      ? categories
      : categories.filter((c) => c.scope === categoryScopeFilter);

  const settingsSubTabs = useMemo(
    () =>
      visibleSettingsSubTabs.map((id) => ({
        id,
        label: SETTINGS_SUBTAB_LABEL[id],
      })),
    [visibleSettingsSubTabs],
  );

  const [fireTaxDraft, setFireTaxDraft] = useState<FireSettingsApi>(() =>
    normalizeInstallationFireSettings(installation?.installation.fire_settings),
  );
  const [fireTaxSaving, setFireTaxSaving] = useState(false);
  const lastSavedFireTaxPayloadRef = useRef<string>("");
  const skipFireTaxAutosaveRef = useRef(true);

  useEffect(() => {
    const serverFs = normalizeInstallationFireSettings(
      installation?.installation.fire_settings,
    );
    setFireTaxDraft(serverFs);
    lastSavedFireTaxPayloadRef.current = JSON.stringify(serverFs);
    skipFireTaxAutosaveRef.current = true;
    // Re-inicializa el draft solo al cambiar de instalación; NO en cada cambio de
    // fire_settings, que clobbearía ediciones en curso (este draft autosalva).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [installation?.installation.id]);

  /**
   * El sub-tab «Plan» edita supuestos que alimentan la proyección; en la vista Hogar (agregado
   * de N miembros) no hay una sola persona a la que atribuir el cambio, así que el panel se
   * enseña en solo lectura y remite a la vista «Yo». Gatea también los autoguardados: dejarlos
   * vivos con el formulario oculto guardaría a espaldas del usuario.
   */
  const planEditable = isOwner && !scopeReadOnly;
  const planReadOnlyNote = scopeReadOnly
    ? "Solo lectura. Tu plan se edita desde la vista «Yo»."
    : "Solo lectura.";

  useEffect(() => {
    if (!hasMembership || !planEditable) return;
    if (skipFireTaxAutosaveRef.current) {
      skipFireTaxAutosaveRef.current = false;
      return;
    }
    const payloadJson = JSON.stringify(fireTaxDraft);
    if (payloadJson === lastSavedFireTaxPayloadRef.current) return;
    let cancelled = false;
    const timer = window.setTimeout(() => {
      setFireTaxSaving(true);
      void onSaveFire(fireTaxDraft)
        .then(() => {
          // La marca de «ya guardado» va en el `then`, NO en el `finally`: marcándola pasara lo
          // que pasara, un PATCH fallido quedaba registrado como guardado, el efecto no volvía a
          // intentarlo nunca (el payload ya coincidía con el ref) y el pie del panel seguía
          // prometiendo «Guardado automático». El cambio se perdía en silencio. Mismo patrón que
          // `runFireSave` en RetirementView.
          if (!cancelled) lastSavedFireTaxPayloadRef.current = payloadJson;
        })
        .catch(() => {
          // El banner de error lo pinta App.tsx (`saveFireSettingsPatch` rellena
          // `installationError` antes de relanzar). Aquí solo hay que NO marcar como guardado.
        })
        .finally(() => {
          if (!cancelled) setFireTaxSaving(false);
        });
    }, 420);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [fireTaxDraft, hasMembership, planEditable, onSaveFire]);

  // ── Autoguardado de zona horaria (4.0.6: en Ajustes todo guarda solo) ──
  // Solo se lanza el PATCH cuando el draft es una IANA VÁLIDA y difiere del servidor:
  // sin la guarda, cada pausa al teclear («Europe/Ma…») sería un 400 y un banner.
  const serverCalendarTz = (installation?.installation.calendar_tz ?? "UTC").trim();
  const calendarTzValid = useMemo(() => {
    const t = calendarTzDraft.trim();
    if (t.length < 3) return false;
    try {
      new Intl.DateTimeFormat(undefined, { timeZone: t });
      return true;
    } catch {
      return false;
    }
  }, [calendarTzDraft]);
  useEffect(() => {
    if (!hasMembership || !isOwner) return;
    if (!calendarTzValid) return;
    if (calendarTzDraft.trim() === serverCalendarTz) return;
    const timer = window.setTimeout(() => saveInstallationCalendarTz(), 700);
    return () => window.clearTimeout(timer);
  }, [
    calendarTzDraft,
    calendarTzValid,
    serverCalendarTz,
    hasMembership,
    isOwner,
    saveInstallationCalendarTz,
  ]);

  // ── Autoguardado de inflación + modo edad (mismo contrato) ──
  const serverInflationPct = parseDisplayDecimal(
    String(installation?.installation.annual_inflation_assumption_percent ?? "0"),
  );
  const serverShowAgeMode =
    installation?.installation.show_age_mode === "ages" ? "ages" : "dates";
  const draftInflationPct =
    projectionInflationPctDraft.trim() === ""
      ? 0
      : parseDisplayDecimal(projectionInflationPctDraft);
  const projectionDraftValid =
    draftInflationPct != null &&
    Number.isFinite(draftInflationPct) &&
    draftInflationPct >= -2 &&
    draftInflationPct <= 50;
  useEffect(() => {
    if (!hasMembership || !planEditable) return;
    if (!projectionDraftValid) return;
    if (
      draftInflationPct === serverInflationPct &&
      showAgeModeDraft === serverShowAgeMode
    ) {
      return;
    }
    const timer = window.setTimeout(() => saveInstallationProjection(), 700);
    return () => window.clearTimeout(timer);
  }, [
    draftInflationPct,
    projectionDraftValid,
    showAgeModeDraft,
    serverInflationPct,
    serverShowAgeMode,
    hasMembership,
    planEditable,
    saveInstallationProjection,
  ]);

  return (
    <div className="workspace">
      <div className="workspace-header">
        <h2 className="workspace-title">Ajustes</h2>
      </div>

      <AccountCard
        username={user.username}
        role={installation?.role ?? null}
        installationName={
          installation?.installation.base_currency
            ? `Moneda ${installation.installation.base_currency}`
            : null
        }
        onEditAccount={onEditAccount}
        onLogout={onLogout}
        busy={authBusy}
      />

      <nav
        className="settings-subtab-bar"
        aria-label="Subsecciones de ajustes"
      >
        {settingsSubTabs.map((t) => {
          const active = settingsSubTab === t.id;
          return (
            <button
              key={t.id}
              type="button"
              className={`ff-nav-pill ${active ? "is-active" : ""}`}
              aria-current={active ? "page" : undefined}
              onClick={() => navigateSettingsSubTab(t.id)}
            >
              {t.label}
            </button>
          );
        })}
      </nav>

      {settingsSubTab === "access" && isOwner ? (
        <section className="panel">
          <h3 className="panel-title">Aprobar acceso</h3>
          {/* Pestaña «Usuarios» (owner-only). Todo lo relacionado con MCP vive en su pestaña. */}
          {pendingUsersBusy ? (
            <p className="muted bordered-top">Cargando…</p>
          ) : pendingUsers.length === 0 ? (
            <p className="muted bordered-top">Nadie pendiente.</p>
          ) : (
            <ul className="pending-users-list">
              {pendingUsers.map((u) => (
                <li key={u.id} className="pending-user-row">
                  <span className="pending-user-name">{u.username}</span>
                  <div className="pending-user-actions">
                    <label className="field inline-role">
                      <span className="sr-only">Rol</span>
                      <select
                        value={approveRoles[u.id] ?? "member"}
                        onChange={(e) =>
                          setApproveRoles((prev) => ({
                            ...prev,
                            [u.id]: e.target.value as "member" | "viewer",
                          }))
                        }
                      >
                        <option value="member">Miembro</option>
                        <option value="viewer">Visor</option>
                      </select>
                    </label>
                    <button
                      type="button"
                      className="btn primary"
                      disabled={approveBusy}
                      onClick={() => approvePendingUser(u.id)}
                    >
                      Aprobar
                    </button>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </section>
      ) : null}

      {settingsSubTab === "integrations" && hasMembership ? (
        <section className="panel">
          <h3 className="panel-title">Servidor MCP</h3>
          <p className="muted">
            Conecta Claude u otro asistente a tus finanzas: el servidor MCP embebido vive en{" "}
            <code>https://tu-host/mcp</code> y acepta tokens de API (abajo) o el conector OAuth de
            claude.ai. Las herramientas de lectura están siempre disponibles para cualquier
            miembro; la escritura respeta el rol (los visores nunca escriben) y el interruptor de
            esta página.
          </p>
          <div className="bordered-top">
            <div className="mcp-write-toggle-row">
              <Switch
                checked={mcpWriteEnabled}
                onChange={onToggleMcpWrite}
                disabled={!isOwner || mcpWriteSaving}
                label="Permitir escritura vía MCP"
                ariaLabel="Permitir que las herramientas MCP escriban en esta instalación"
              />
              {isOwner ? (
                <p className="muted tight">
                  {mcpWriteSaving ? "Guardando…" : "Guardado automático."} Al desactivarlo, las
                  herramientas de escritura se cortan al instante (las de lectura siguen
                  funcionando).
                </p>
              ) : (
                <p className="muted tight">
                  <strong>{mcpWriteEnabled ? "Activada" : "Desactivada"}</strong> · solo el propietario
                  puede cambiarlo.
                </p>
              )}
            </div>
          </div>
        </section>
      ) : null}

      {settingsSubTab === "integrations" && hasMembership ? <ApiTokensPanel /> : null}

      {settingsSubTab === "integrations" && hasMembership ? <OAuthConnectionsPanel /> : null}

      {settingsSubTab === "general" && hasMembership ? (
        <section className="panel">
          <h3 className="panel-title">Divisa</h3>
          <p className="muted">
            Una sola por hogar: FutureFin no convierte entre divisas. Cambiarla cambia el símbolo
            con el que se muestran tus importes, no los reconvierte.
          </p>
          {isOwner && onChangeCurrency ? (
            <div className="stack bordered-top">
              <label className="field">
                <span>Divisa del hogar</span>
                <select
                  value={installation?.installation.base_currency ?? "EUR"}
                  disabled={currencySaving}
                  onChange={(e) => onChangeCurrency(e.target.value)}
                >
                  <option value="EUR">Euro (€)</option>
                  <option value="USD">Dólar estadounidense ($)</option>
                  <option value="GBP">Libra esterlina (£)</option>
                </select>
              </label>
            </div>
          ) : (
            <p className="muted bordered-top">
              <strong>{installation?.installation.base_currency ?? "EUR"}</strong> · solo el
              propietario puede cambiarlo.
            </p>
          )}
        </section>
      ) : null}

      {settingsSubTab === "general" && hasMembership ? (
        <section className="panel">
          <h3 className="panel-title">Zona horaria del calendario</h3>
          {isOwner ? (
            <div className="stack bordered-top">
              <label className="field">
                <span>IANA (p. ej. Europe/Madrid)</span>
                <input
                  value={calendarTzDraft}
                  onChange={(e) => setCalendarTzDraft(e.target.value)}
                  maxLength={64}
                  placeholder="Europe/Madrid"
                  autoComplete="off"
                />
              </label>
              <p className="muted tight">
                {calendarTzSaving
                  ? "Guardando…"
                  : !calendarTzValid && calendarTzDraft.trim() !== ""
                    ? "Zona horaria no reconocida — sin guardar."
                    : "Guardado automático."}
              </p>
            </div>
          ) : (
            <p className="muted bordered-top">
              <strong>{installation?.installation.calendar_tz ?? "UTC"}</strong>{" "}
              · solo el propietario puede cambiarlo.
            </p>
          )}
        </section>
      ) : null}

      {settingsSubTab === "plan" && hasMembership ? (
        planEditable ? (
        <section className="panel">
          <h3 className="panel-title">Proyección y modo de edad</h3>
          {/* 5.0.0 (D13/D26): estrategia, edad objetivo, SWR y edad límite del horizonte dejaron
              de ser del hogar y son de cada persona. Aquí solo queda el puntero — un ajuste que
              se editaba en dos sitios acaba divergiendo en uno de ellos. */}
          <p className="muted">
            Tu estrategia, tu edad objetivo y tu SWR se editan en{" "}
            <a
              href={appUrl(TAB_PATH.retirement)}
              onClick={(e) => {
                if (e.button !== 0 || e.metaKey || e.altKey || e.ctrlKey || e.shiftKey)
                  return;
                e.preventDefault();
                navigate(TAB_PATH.retirement);
              }}
            >
              Jubilación
            </a>
            .
          </p>
          <div className="stack bordered-top">
            <label className="field">
              <span className="label-with-help">
                Inflación anual %
                <HelpPopover
                  title={HELP_TEXTS["settings.inflation"].title}
                  body={HELP_TEXTS["settings.inflation"].body}
                />
              </span>
              <input
                value={projectionInflationPctDraft}
                onChange={(e) =>
                  setProjectionInflationPctDraft(e.target.value)
                }
                inputMode="decimal"
                placeholder="2,5"
                autoComplete="off"
              />
              <small className="muted">
                Se aplica al target FIRE para preservar tu poder adquisitivo. Los
                ingresos, gastos y aportaciones se mantienen constantes en euros
                — refleja «hacer lo que haces ahora». Usa <code>0</code> para
                desactivar.
                {!projectionDraftValid
                  ? " Número entre 0 y 50 — sin guardar."
                  : null}
              </small>
            </label>
            <label className="field">
              <span>Modo edad en la interfaz</span>
              <select
                value={showAgeModeDraft}
                onChange={(e) =>
                  setShowAgeModeDraft(
                    e.target.value === "ages" ? "ages" : "dates",
                  )
                }
              >
                <option value="dates">Fechas</option>
                <option value="ages">Edades</option>
              </select>
            </label>
          </div>

          <div className="stack bordered-top">
            {/* El bloque de ayuda va FUERA del <label> (como hermano) para que el nombre accesible
                del <select> sea solo su título y un clic en la ayuda no abra el desplegable;
                aria-describedby lo asocia igualmente para lectores de pantalla. */}
            <label className="field">
              <span className="label-with-help">
                Fuente del ahorro de la simulación
                <HelpPopover
                  title={HELP_TEXTS["settings.savings_source"].title}
                  body={HELP_TEXTS["settings.savings_source"].body}
                />
              </span>
              <select
                aria-describedby="savings-source-help"
                value={fireTaxDraft.savings_source ?? "budget"}
                onChange={(e) =>
                  setFireTaxDraft((p) => ({
                    ...p,
                    savings_source: parseSavingsSource(e.target.value),
                  }))
                }
              >
                <option value="budget">Presupuesto</option>
                <option value="transactions_avg">Movimientos reales</option>
                <option value="budget_income_real_expense">
                  Ingresos de presupuesto + gasto real
                </option>
              </select>
            </label>
            {/* Solo la descripción del modo ELEGIDO; la comparativa completa de los tres
                modos vive en el HelpPopover del selector (settings.savings_source). */}
            <div className="field" id="savings-source-help">
              {(fireTaxDraft.savings_source ?? "budget") === "budget" ? (
                <small className="muted">
                  La simulación ahorra cada mes lo que fijas en tu presupuesto
                  (ingresos − gastos presupuestados). No depende de tus
                  movimientos.
                </small>
              ) : fireTaxDraft.savings_source === "transactions_avg" ? (
                <small className="muted">
                  El ahorro sale de tus movimientos: ingreso y gasto promediados
                  por separado con las ventanas de abajo. Sin datos, ese lado cae
                  al presupuesto.
                </small>
              ) : (
                <small className="muted">
                  Ingresos del presupuesto y gasto medio real (ventana de abajo).
                  Solo acierta mientras mantengas el presupuesto de ingresos al
                  día.
                </small>
              )}
            </div>

            {savingsSourceUsesTransactions(fireTaxDraft.savings_source) ? (
              <div className="stack">
                {(
                  [
                    ["income", "Ingreso"] as const,
                    ["expense", "Gasto"] as const,
                  ] as const
                )
                  .filter(
                    ([side]) =>
                      side === "expense" ||
                      fireTaxDraft.savings_source === "transactions_avg",
                  )
                  .map(([side, label]) => (
                    <div className="field-row" key={side}>
                      <label className="field">
                        <span className="label-with-help">
                          {label}: meses
                          <HelpPopover
                            title={
                              HELP_TEXTS[
                                side === "income"
                                  ? "settings.income_window"
                                  : "settings.expense_window"
                              ].title
                            }
                            body={
                              HELP_TEXTS[
                                side === "income"
                                  ? "settings.income_window"
                                  : "settings.expense_window"
                              ].body
                            }
                          />
                        </span>
                        <input
                          inputMode="numeric"
                          value={String(
                            side === "income"
                              ? (fireTaxDraft.income_avg_window_months ?? 3)
                              : (fireTaxDraft.expense_avg_window_months ?? 12),
                          )}
                          onChange={(e) => {
                            const n = Number(e.target.value.trim());
                            if (!Number.isInteger(n) || n < 1 || n > 60) return;
                            setFireTaxDraft((p) => ({
                              ...p,
                              [side === "income"
                                ? "income_avg_window_months"
                                : "expense_avg_window_months"]: n,
                            }));
                          }}
                        />
                      </label>
                      <label className="field">
                        <span className="label-with-help">
                          {label}: cómo se cuentan
                          <HelpPopover
                            title={HELP_TEXTS["settings.window_mode"].title}
                            body={HELP_TEXTS["settings.window_mode"].body}
                          />
                        </span>
                        <select
                          value={
                            side === "income"
                              ? (fireTaxDraft.income_avg_window_mode ??
                                "calendar")
                              : (fireTaxDraft.expense_avg_window_mode ??
                                "calendar")
                          }
                          onChange={(e) =>
                            setFireTaxDraft((p) => ({
                              ...p,
                              [side === "income"
                                ? "income_avg_window_mode"
                                : "expense_avg_window_mode"]:
                                e.target.value === "data" ? "data" : "calendar",
                            }))
                          }
                        >
                          <option value="calendar">
                            Meses de calendario
                          </option>
                          <option value="data">Meses con datos</option>
                        </select>
                      </label>
                    </div>
                  ))}
              </div>
            ) : null}

            {/* La plusvalía gravable estaba ANIDADA dentro del bloque de las ventanas del
                promedio, así que en el modo «Presupuesto» —el de serie— era invisible. Y no
                depende del modo: gobierna el objetivo, el drenaje simulado y los dos umbrales
                de Autonomía siempre. Un ajuste que la ayuda describe como vivo y la pantalla no
                deja tocar es peor que no tenerlo. (El selector de edad límite compartía el
                mismo anidamiento; se fue al perfil de jubilación en 5.0.0.) */}
            <label className="field">
              <span className="label-with-help">
                Plusvalía gravable de la retirada
                <HelpPopover
                  title={HELP_TEXTS["settings.taxable_gain"].title}
                  body={HELP_TEXTS["settings.taxable_gain"].body}
                />
              </span>
              <input
                inputMode="decimal"
                value={String(fireTaxDraft.taxable_gain_ratio ?? "1")}
                onChange={(e) => {
                  const raw = e.target.value.trim().replace(",", ".");
                  setFireTaxDraft((prev) => ({
                    ...prev,
                    taxable_gain_ratio: raw,
                  }));
                }}
              />
            </label>
            <p className="muted tight">
              {fireTaxSaving || installationProjectionSaving
                ? "Guardando…"
                : "Guardado automático."}
            </p>
          </div>
        </section>
        ) : (
        <section className="panel muted-panel">
          <h3 className="panel-title">Proyección y modo de edad</h3>
          <p className="muted tight">{planReadOnlyNote}</p>
        </section>
        )
      ) : null}

      {settingsSubTab === "plan" && hasMembership ? (
        planEditable ? (
          <section className="panel">
            <h3 className="panel-title">Fiscalidad (IRPF ahorro)</h3>
            <div className="stack bordered-top">
              <label className="field checkbox-field">
                <input
                  type="checkbox"
                  checked={fireTaxDraft.taxes_enabled}
                  onChange={(e) =>
                    setFireTaxDraft((p) => ({
                      ...p,
                      taxes_enabled: e.target.checked,
                    }))
                  }
                />
                <span>Aplicar IRPF del ahorro</span>
              </label>
              <fieldset
                disabled={!fireTaxDraft.taxes_enabled}
                className="stack tight tax-brackets-fieldset"
              >
                  <div className="table-scroll">
                    <table className="tax-brackets-table">
                      <thead>
                        <tr>
                          <th>Hasta base (€)</th>
                          <th>Tipo (%)</th>
                        </tr>
                      </thead>
                      <tbody>
                        {fireTaxDraft.tax_brackets.map((row, idx) => (
                          <tr key={`tax-br-${idx}`}>
                            <td>
                              <input
                                placeholder={
                                  idx === fireTaxDraft.tax_brackets.length - 1 ? "∞" : ""
                                }
                                value={row.up_to ?? ""}
                                onChange={(e) => {
                                  const t = e.target.value.trim();
                                  // Por la función CANÓNICA de la app, no un replace a pelo:
                                  // «6.000» es SEIS MIL en escritura española y así lo leen ya
                                  // todos los importes (regla de millares de toApiDecimalString).
                                  // Con el replace anterior llegaba al servidor como 6,000 € y
                                  // la escala entera colapsaba a umbrales de céntimos EN
                                  // SILENCIO (seguía siendo creciente y pasaba la validación).
                                  // Si no parsea, se conserva el texto tal cual — el servidor
                                  // lo rechaza con banner, igual que antes.
                                  let normalized = t;
                                  try {
                                    normalized = toApiDecimalString(t);
                                  } catch {
                                    /* texto intermedio no parseable: se envía y el 400 avisa */
                                  }
                                  setFireTaxDraft((p) => {
                                    const next = [...p.tax_brackets];
                                    next[idx] = {
                                      ...next[idx],
                                      up_to:
                                        t === ""
                                          ? idx === p.tax_brackets.length - 1
                                            ? null
                                            : next[idx].up_to
                                          : normalized,
                                    };
                                    return { ...p, tax_brackets: next };
                                  });
                                }}
                              />
                            </td>
                            <td>
                              <input
                                value={row.pct}
                                onChange={(e) => {
                                  const t = e.target.value.replace(",", ".");
                                  setFireTaxDraft((p) => {
                                    const next = [...p.tax_brackets];
                                    next[idx] = { ...next[idx], pct: t };
                                    return { ...p, tax_brackets: next };
                                  });
                                }}
                              />
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                  <button
                    type="button"
                    className="btn ghost"
                    onClick={() =>
                      setFireTaxDraft((p) => ({
                        ...p,
                        tax_brackets: DEFAULT_ES_TAX_BRACKETS_API.map((b) => ({
                          up_to: b.up_to,
                          pct: b.pct,
                        })),
                      }))
                    }
                  >
                    Restaurar España
                  </button>
              </fieldset>
              <p className="muted tight">
                {fireTaxSaving ? "Guardando…" : "Guardado automático."}
              </p>
            </div>
          </section>
        ) : (
          <section className="panel muted-panel">
            <h3 className="panel-title">Fiscalidad (IRPF ahorro)</h3>
            <p className="muted tight">{planReadOnlyNote}</p>
          </section>
        )
      ) : null}

      {settingsSubTab === "categories" && hasMembership ? (
        <section className="panel">
          <div className="panel-head-row">
            <h3 className="panel-title">Categorías</h3>
            {canEditCategories ? (
              <button
                type="button"
                className="btn primary"
                onClick={() => openNewCategoryModal()}
              >
                Nueva categoría
              </button>
            ) : null}
          </div>
          <div className="category-toolbar bordered-top">
            <label className="field inline-role">
              <span className="sr-only">Filtrar por ámbito</span>
              <select
                value={categoryScopeFilter}
                onChange={(e) => {
                  const v = e.target.value;
                  setCategoryScopeFilter(
                    v === "all" ? "all" : (v as CategoryScope),
                  );
                }}
              >
                <option value="all">Todos los ámbitos</option>
                {CATEGORY_SCOPES.map((s) => (
                  <option key={s} value={s}>
                    {CATEGORY_SCOPE_LABEL[s]}
                  </option>
                ))}
              </select>
            </label>
          </div>
          {!canEditCategories ? (
            <p className="muted bordered-top">
              Solo lectura.
            </p>
          ) : null}
          {categoriesBusy ? (
            <p className="muted bordered-top">Cargando…</p>
          ) : (
            <ul className="category-list">
              {filteredCategories.map((c) => (
                <li key={c.id} className="category-row">
                  <span className="category-scope-tag">
                    {CATEGORY_SCOPE_LABEL[c.scope]}
                  </span>
                  <span className="category-name">{c.name}</span>
                  {c.is_fallback ? (
                    <span
                      className="category-default-tag"
                      title="Aquí van los ingresos y gastos que no clasifica ninguna regla"
                    >
                      Por defecto
                    </span>
                  ) : null}
                  {canEditCategories ? (
                    <div className="category-row-actions budget-row-actions">
                      {!c.is_fallback &&
                      (c.scope === "income" || c.scope === "expense") ? (
                        <button
                          type="button"
                          className="btn ghost text"
                          title="Hacer categoría por defecto de su ámbito"
                          disabled={categorySaving}
                          onClick={() => makeCategoryFallback(c)}
                        >
                          Por defecto
                        </button>
                      ) : null}
                      <button
                        type="button"
                        className="btn ghost icon-btn"
                        aria-label="Renombrar categoría"
                        disabled={categorySaving}
                        onClick={() => openRenameCategoryModal(c)}
                      >
                        <RowEditIcon />
                      </button>
                      <button
                        type="button"
                        className="btn ghost danger icon-btn"
                        aria-label="Eliminar categoría"
                        // La por defecto no se borra: es el destino de todo ingreso o gasto sin
                        // clasificar. Para retirarla hay que marcar antes otra.
                        disabled={categorySaving || c.is_fallback}
                        title={
                          c.is_fallback
                            ? "Es la categoría por defecto de su ámbito: marca antes otra como predeterminada"
                            : undefined
                        }
                        onClick={() => openCategoryDeleteModal(c)}
                      >
                        <RowTrashIcon />
                      </button>
                    </div>
                  ) : null}
                </li>
              ))}
            </ul>
          )}
          {!categoriesBusy && filteredCategories.length === 0 ? (
            <p className="muted bordered-top">Sin datos.</p>
          ) : null}
        </section>
      ) : null}

      {settingsSubTab === "history" && hasMembership ? (
        <HistorySettingsPanel
          canEdit={canEditHistory}
          currencyIso={currencyIso}
          calendarTz={calendarTz}
          onHistoryMutated={onHistoryMutated}
        />
      ) : null}

      {settingsSubTab === "general" ? (
        <>
          <section className="panel">
            <h3 className="panel-title">Apariencia</h3>
            <p className="muted">
              Elige el tema de la interfaz. «Auto» sigue la preferencia del
              sistema.
            </p>
            <div className="bordered-top">
              <ThemeToggle value={themePref} onChange={onChangeTheme} />
            </div>
          </section>

          {isOwner && onReopenOnboarding ? (
            <section className="panel">
              <h3 className="panel-title">Configuración inicial</h3>
              <p className="muted">
                Repasa la divisa, la zona horaria y los supuestos de tu plan con el asistente de
                bienvenida. No borra nada: solo vuelve a preguntarte.
              </p>
              <div className="bordered-top">
                <button type="button" className="btn" onClick={onReopenOnboarding}>
                  Abrir el asistente
                </button>
              </div>
            </section>
          ) : null}

          <section className="panel">
            <h3 className="panel-title">Instalación</h3>
            {installationBusy ? (
              <p className="muted tight">Cargando…</p>
            ) : installation ? (
              <dl className="settings-meta-dl">
                <div>
                  <dt>Moneda base</dt>
                  <dd>{installation.installation.base_currency}</dd>
                </div>
                <div>
                  <dt>Tu rol</dt>
                  <dd>{roleLabel(installation.role)}</dd>
                </div>
              </dl>
            ) : (
              <p className="muted tight">Sin acceso.</p>
            )}
          </section>

          <section className="panel">
            <h3 className="panel-title">Estado del sistema</h3>
            {healthError ? (
              <p className="error compact">
                No se puede contactar con la API. {healthError}
              </p>
            ) : health ? (
              <dl className="settings-meta-dl">
                <div>
                  <dt>Servicio</dt>
                  <dd>{health.service}</dd>
                </div>
                <div>
                  <dt>Versión</dt>
                  <dd>{health.version}</dd>
                </div>
                <div>
                  <dt>Estado</dt>
                  <dd>{healthStatusLabel(health.status)}</dd>
                </div>
              </dl>
            ) : (
              <p className="muted tight">Cargando…</p>
            )}
          </section>
        </>
      ) : null}

      {settingsSubTab === "data" ? (
        <>
          {hasMembership ? (
            <section className="panel">
              <h3 className="panel-title">Copia de seguridad personal</h3>
              <p className="muted">
                Exporta o restaura un archivo <code>.ffbackup</code> cifrado con
                tu contraseña que contiene solo tus datos: activos, pasivos,
                presupuesto, planificación, categorías usadas, fecha de
                nacimiento y preferencias UI. Portable entre instalaciones.
              </p>
              {ffbackupImportDone ? (
                <p className="muted bordered-top">{ffbackupImportDone}</p>
              ) : null}
              <div className="bordered-top">
                <button
                  type="button"
                  className="btn primary"
                  onClick={() => openFfbackupExportModal()}
                >
                  Exportar mis datos (.ffbackup)
                </button>
                <button
                  type="button"
                  className="btn"
                  onClick={() => openFfbackupImportModal()}
                >
                  Importar backup (.ffbackup)
                </button>
              </div>
            </section>
          ) : null}

        </>
      ) : null}

      {canEditCategories ? (
        <>
          <Modal
            title="Nueva categoría"
            open={categoryModalOpen}
            onClose={closeCategoryModal}
          >
            <form className="asset-form stack" onSubmit={createCategory}>
              <ModalFormError message={categoriesError} />
              <label className="field">
                <span>Ámbito</span>
                <select
                  value={newCatScope}
                  onChange={(e) =>
                    setNewCatScope(e.target.value as CategoryScope)
                  }
                >
                  {CATEGORY_SCOPES.map((s) => (
                    <option key={s} value={s}>
                      {CATEGORY_SCOPE_LABEL[s]}
                    </option>
                  ))}
                </select>
              </label>
              <label className="field">
                <span>Nombre</span>
                <input
                  value={newCatName}
                  onChange={(e) => setNewCatName(e.target.value)}
                  maxLength={200}
                  placeholder="p. ej. Efectivo"
                  autoComplete="off"
                />
              </label>
              <div className="asset-form-actions">
                <button
                  type="submit"
                  className="btn primary"
                  disabled={categorySaving}
                >
                  Añadir
                </button>
                <button
                  type="button"
                  className="btn ghost"
                  disabled={categorySaving}
                  onClick={() => closeCategoryModal()}
                >
                  Cancelar
                </button>
              </div>
            </form>
          </Modal>
          <Modal
            title="Renombrar categoría"
            open={categoryRenameModalOpen && editingCategoryId !== null}
            onClose={closeRenameCategoryModal}
          >
            {renamingCat ? (
              <form
                className="asset-form stack"
                onSubmit={(e) => {
                  e.preventDefault();
                  if (editingCategoryId) {
                    void saveCategoryEdit(editingCategoryId);
                  }
                }}
              >
                <ModalFormError message={categoriesError} />
                <p className="muted tight">
                  Ámbito:{" "}
                  <strong>{CATEGORY_SCOPE_LABEL[renamingCat.scope]}</strong>
                </p>
                <label className="field">
                  <span>Nombre</span>
                  <input
                    value={editCategoryName}
                    onChange={(e) => setEditCategoryName(e.target.value)}
                    maxLength={200}
                    aria-label="Nuevo nombre"
                    autoComplete="off"
                  />
                </label>
                <div className="asset-form-actions">
                  <button
                    type="submit"
                    className="btn primary"
                    disabled={categorySaving || !editCategoryName.trim()}
                  >
                    Guardar
                  </button>
                  <button
                    type="button"
                    className="btn ghost"
                    disabled={categorySaving}
                    onClick={() => closeRenameCategoryModal()}
                  >
                    Cancelar
                  </button>
                </div>
              </form>
            ) : (
              <p className="muted tight">Categoría no encontrada.</p>
            )}
          </Modal>
          <Modal
            title="Eliminar categoría"
            open={categoryDeleteModalOpen && categoryDeletePending !== null}
            onClose={closeCategoryDeleteModal}
          >
            {categoryDeletePending ? (
              <div className="stack">
                <ModalFormError message={categoriesError} />
                <p className="muted tight">
                  Se eliminará{" "}
                  <strong>{categoryDeletePending.name}</strong> (
                  {CATEGORY_SCOPE_LABEL[categoryDeletePending.scope]}).
                </p>
                {(() => {
                  const siblings = categories.filter(
                    (x) =>
                      x.scope === categoryDeletePending.scope &&
                      x.id !== categoryDeletePending.id,
                  );
                  if (siblings.length === 0) {
                    return (
                      <p className="muted tight">Sin categoría sustituta en el ámbito.</p>
                    );
                  }
                  // La por defecto va primero en la lista y es la preseleccionada (eso lo hace
                  // `openCategoryDeleteModal`): es el destino natural de lo que se queda huérfano.
                  const ordered = [
                    ...siblings.filter((x) => x.is_fallback),
                    ...siblings.filter((x) => !x.is_fallback),
                  ];
                  return (
                    <label className="field">
                      <span>Reasignar a</span>
                      <select
                        value={categoryRemapToId}
                        onChange={(e) => setCategoryRemapToId(e.target.value)}
                        aria-label="Categoría destino al reasignar"
                      >
                        {ordered.map((s) => (
                          <option key={s.id} value={s.id}>
                            {s.name}
                            {s.is_fallback ? " · por defecto" : ""}
                          </option>
                        ))}
                      </select>
                    </label>
                  );
                })()}
                <div className="asset-form-actions">
                  <button
                    type="button"
                    className="btn ghost danger"
                    disabled={categorySaving}
                    onClick={() => confirmDeleteCategory()}
                  >
                    Eliminar
                  </button>
                  <button
                    type="button"
                    className="btn ghost"
                    disabled={categorySaving}
                    onClick={() => closeCategoryDeleteModal()}
                  >
                    Cancelar
                  </button>
                </div>
              </div>
            ) : (
              <p className="muted tight">Nada seleccionado.</p>
            )}
          </Modal>
        </>
      ) : null}

      <Modal
        title="Exportar backup personal"
        open={ffbackupExportModalOpen}
        onClose={closeFfbackupExportModal}
      >
        <form className="stack" onSubmit={runFfbackupExport}>
          <ModalFormError message={ffbackupExportError} />
          <p className="muted tight">
            El archivo .ffbackup quedará cifrado con tu contraseña actual.
            Guárdalo en un sitio seguro y recuerda la contraseña — sin ella
            no se puede restaurar.
          </p>
          <label className="field">
            <span>Tu contraseña</span>
            <input
              type="password"
              autoComplete="current-password"
              value={ffbackupExportPassword}
              onChange={(e) => setFfbackupExportPassword(e.target.value)}
              disabled={ffbackupExportBusy}
              required
            />
          </label>
          <div className="asset-form-actions">
            <button
              type="submit"
              className="btn primary"
              disabled={ffbackupExportBusy}
            >
              {ffbackupExportBusy ? "Generando…" : "Descargar .ffbackup"}
            </button>
            <button
              type="button"
              className="btn ghost"
              disabled={ffbackupExportBusy}
              onClick={() => closeFfbackupExportModal()}
            >
              Cancelar
            </button>
          </div>
        </form>
      </Modal>

      <Modal
        title="Importar backup personal"
        open={ffbackupImportModalOpen}
        onClose={closeFfbackupImportModal}
      >
        <div className="stack">
          <ModalFormError message={ffbackupImportError} />
          {ffbackupImportPreview === null ? (
            <form className="stack" onSubmit={runFfbackupImportPreview}>
              <p className="muted tight">
                Sube un archivo .ffbackup y la contraseña con la que se
                generó. Verás un resumen antes de aplicar nada.
              </p>
              <label className="field">
                <span>Archivo .ffbackup</span>
                <input
                  type="file"
                  accept=".ffbackup"
                  disabled={ffbackupImportBusy}
                  onChange={(e) => {
                    const f = e.target.files && e.target.files[0];
                    setFfbackupImportFile(f ?? null);
                  }}
                />
              </label>
              <label className="field">
                <span>Contraseña del backup</span>
                <input
                  type="password"
                  autoComplete="off"
                  value={ffbackupImportPassword}
                  onChange={(e) =>
                    setFfbackupImportPassword(e.target.value)
                  }
                  disabled={ffbackupImportBusy}
                  required
                />
              </label>
              <div className="asset-form-actions">
                <button
                  type="submit"
                  className="btn primary"
                  disabled={ffbackupImportBusy || !ffbackupImportFile}
                >
                  {ffbackupImportBusy ? "Leyendo…" : "Previsualizar"}
                </button>
                <button
                  type="button"
                  className="btn ghost"
                  disabled={ffbackupImportBusy}
                  onClick={() => closeFfbackupImportModal()}
                >
                  Cancelar
                </button>
              </div>
            </form>
          ) : (
            <div className="stack">
              <p className="muted tight">
                Backup de <strong>{ffbackupImportPreview.username_original}</strong>{" "}
                exportado el {ffbackupImportPreview.exported_at} (app{" "}
                {ffbackupImportPreview.app_version}, schema v
                {ffbackupImportPreview.schema_version}).
              </p>
              <ul className="muted tight muted-list">
                <li>{ffbackupImportPreview.counts.assets} activos</li>
                <li>{ffbackupImportPreview.counts.liabilities} pasivos</li>
                <li>
                  {ffbackupImportPreview.counts.budget_entries} entradas de
                  presupuesto
                </li>
                <li>
                  {ffbackupImportPreview.counts.planning_flows} flujos
                  planificados
                </li>
                <li>
                  Categorías:{" "}
                  {ffbackupImportPreview.counts.categories_in_backup} (
                  {ffbackupImportPreview.counts.categories_already_present} ya
                  existen,{" "}
                  {ffbackupImportPreview.counts.categories_to_create} se
                  crearán)
                </li>
                {ffbackupImportPreview.birth_date_will_change ? (
                  <li>Tu fecha de nacimiento se actualizará.</li>
                ) : null}
                {ffbackupImportPreview.ui_preferences_present ? (
                  <li>Se restaurarán tus preferencias UI.</li>
                ) : null}
              </ul>
              <p className="error compact">
                Al continuar se eliminarán todos tus activos, pasivos,
                presupuesto y planificación actuales y serán reemplazados por
                los del backup. Operación atómica e irreversible.
              </p>
              <div className="asset-form-actions">
                <button
                  type="button"
                  className="btn ghost danger"
                  disabled={ffbackupImportBusy}
                  onClick={() => runFfbackupImportApply()}
                >
                  {ffbackupImportBusy
                    ? "Importando…"
                    : "Confirmar reemplazo"}
                </button>
                <button
                  type="button"
                  className="btn ghost"
                  disabled={ffbackupImportBusy}
                  onClick={() => closeFfbackupImportModal()}
                >
                  Cancelar
                </button>
              </div>
            </div>
          )}
        </div>
      </Modal>
    </div>
  );
}
