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
import { Modal, ModalFormError } from "../components/Modal";
import { RowEditIcon, RowTrashIcon } from "../components/icons";
import { AccountCard } from "../components/AccountCard";
import { HistorySettingsPanel } from "./HistorySettingsPanel";
import { ThemeToggle } from "../components/ThemeToggle";
import type { ThemePref } from "../lib/theme";
import {
  DEFAULT_ES_TAX_BRACKETS_API,
  normalizeInstallationFireSettings,
} from "../lib/fire";
import {
  SETTINGS_SUBTAB_LABEL,
  type SettingsSubTabId,
} from "../lib/navigation";

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
  currencyIso,
  calendarTz,
  onHistoryMutated,
  isOwner,
  settingsSubTab,
  navigateSettingsSubTab,
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
  saveInstallationCalendarTz: (e: FormEvent) => void;
  projectionInflationPctDraft: string;
  setProjectionInflationPctDraft: Dispatch<SetStateAction<string>>;
  showAgeModeDraft: "dates" | "ages";
  setShowAgeModeDraft: Dispatch<SetStateAction<"dates" | "ages">>;
  installationProjectionSaving: boolean;
  saveInstallationProjection: (e: FormEvent) => void;
  onSaveFire: (fs: FireSettingsApi) => Promise<void>;
  health: HealthResponse | null;
  healthError: string | null;
  categoriesError: string | null;
  hasMembership: boolean;
  canEditCategories: boolean;
  canEditHistory: boolean;
  currencyIso: string;
  /** Zona horaria (IANA) de la instalación; el panel de histórico deriva «hoy» de ella. */
  calendarTz: string;
  onHistoryMutated: () => void;
  isOwner: boolean;
  settingsSubTab: SettingsSubTabId;
  navigateSettingsSubTab: (id: SettingsSubTabId) => void;
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

  useEffect(() => {
    if (!hasMembership || !isOwner) return;
    if (skipFireTaxAutosaveRef.current) {
      skipFireTaxAutosaveRef.current = false;
      return;
    }
    const payloadJson = JSON.stringify(fireTaxDraft);
    if (payloadJson === lastSavedFireTaxPayloadRef.current) return;
    let cancelled = false;
    const timer = window.setTimeout(() => {
      setFireTaxSaving(true);
      void onSaveFire(fireTaxDraft).finally(() => {
        if (!cancelled) {
          lastSavedFireTaxPayloadRef.current = payloadJson;
          setFireTaxSaving(false);
        }
      });
    }, 420);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [fireTaxDraft, hasMembership, isOwner, onSaveFire]);

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

      {settingsSubTab === "calendar" && hasMembership ? (
        <section className="panel">
          <h3 className="panel-title">Zona horaria del calendario</h3>
          {isOwner ? (
            <form
              className="stack bordered-top"
              onSubmit={saveInstallationCalendarTz}
            >
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
              <button
                type="submit"
                className="btn primary"
                disabled={calendarTzSaving}
              >
                Guardar zona horaria
              </button>
            </form>
          ) : (
            <p className="muted bordered-top">
              <strong>{installation?.installation.calendar_tz ?? "UTC"}</strong>{" "}
              · solo lectura
            </p>
          )}
        </section>
      ) : null}

      {settingsSubTab === "projection" && hasMembership ? (
        isOwner ? (
        <section className="panel">
          <h3 className="panel-title">Proyección y modo de edad</h3>
          <form
            className="stack bordered-top"
            onSubmit={saveInstallationProjection}
          >
            <label className="field">
              <span>Inflación anual %</span>
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
            <button
              type="submit"
              className="btn primary"
              disabled={installationProjectionSaving}
            >
              Guardar proyección
            </button>
          </form>

          <div className="stack bordered-top">
            <div className="field">
              <span>Fuente del ahorro de la simulación</span>
              <div
                className="ff-segmented"
                role="group"
                aria-label="Fuente del ahorro de la simulación"
              >
                {(
                  [
                    { value: "budget", label: "Presupuesto" },
                    { value: "transactions_avg", label: "Promedio 12 meses" },
                  ] as const
                ).map((o) => {
                  const active =
                    (fireTaxDraft.savings_source ?? "budget") === o.value;
                  return (
                    <button
                      key={o.value}
                      type="button"
                      className={active ? "is-active" : ""}
                      aria-pressed={active}
                      onClick={() => {
                        if (!active)
                          setFireTaxDraft((p) => ({
                            ...p,
                            savings_source: o.value,
                          }));
                      }}
                    >
                      {o.label}
                    </button>
                  );
                })}
              </div>
              <small className="muted">
                En modo promedio, el ahorro de la simulación sale del promedio
                ponderado de los últimos 12 meses de movimientos, descontando las
                cuotas de préstamos activos (que la simulación ya modela aparte).
                Con menos de 12 meses se usan los meses con datos.
              </small>
              <p className="muted tight">
                {fireTaxSaving ? "Guardando…" : "Guardado automático."}
              </p>
            </div>
          </div>
        </section>
        ) : (
        <section className="panel muted-panel">
          <h3 className="panel-title">Proyección y modo de edad</h3>
          <p className="muted tight">Solo lectura.</p>
        </section>
        )
      ) : null}

      {settingsSubTab === "retirement" && hasMembership ? (
        isOwner ? (
          <section className="panel">
            <h3 className="panel-title">Fiscalidad (IRPF ahorro)</h3>
            <p className="muted tight">
              {fireTaxSaving ? "Guardando…" : "Guardado automático."}
            </p>
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
              <fieldset disabled={!fireTaxDraft.taxes_enabled} className="stack">
                <div className="stack tight">
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
                  <div className="table-scroll">
                    <table className="table">
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
                                  setFireTaxDraft((p) => {
                                    const next = [...p.tax_brackets];
                                    next[idx] = {
                                      ...next[idx],
                                      up_to:
                                        t === ""
                                          ? idx === p.tax_brackets.length - 1
                                            ? null
                                            : next[idx].up_to
                                          : t.replace(",", "."),
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
                </div>
              </fieldset>
            </div>
          </section>
        ) : (
          <section className="panel muted-panel">
            <h3 className="panel-title">Fiscalidad (IRPF ahorro)</h3>
            <p className="muted tight">Solo lectura.</p>
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
            <p className="muted bordered-top">Cargando categorías…</p>
          ) : (
            <ul className="category-list">
              {filteredCategories.map((c) => (
                <li key={c.id} className="category-row">
                  <span className="category-scope-tag">
                    {CATEGORY_SCOPE_LABEL[c.scope]}
                  </span>
                  <span className="category-name">{c.name}</span>
                  {canEditCategories ? (
                    <div className="category-row-actions budget-row-actions">
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
                        disabled={categorySaving}
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
            <p className="muted bordered-top">Vacío.</p>
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

      {settingsSubTab === "data" ? (
        <>
          <section className="panel">
            <div className="panel-head-row">
              <h3 className="panel-title">Apariencia</h3>
            </div>
            <p className="muted compact tight">
              Elige el tema de la interfaz. «Auto» sigue la preferencia del
              sistema.
            </p>
            <div className="bordered-top" style={{ paddingTop: "0.7rem" }}>
              <ThemeToggle value={themePref} onChange={onChangeTheme} />
            </div>
          </section>

          {hasMembership ? (
            <section className="panel">
              <h3 className="panel-title">Backup personal (.ffbackup)</h3>
              <p className="muted compact bordered-top">
                Exporta o restaura un archivo cifrado con tu contraseña que
                contiene solo tus datos: activos, pasivos, presupuesto,
                planificación, categorías usadas, fecha de nacimiento y
                preferencias UI. El archivo es portable entre instalaciones.
              </p>
              {ffbackupImportDone ? (
                <p className="muted compact bordered-top">
                  {ffbackupImportDone}
                </p>
              ) : null}
              <div className="bordered-top settings-backup-actions">
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

          <section className="panel">
            <h3 className="panel-title">Instalación</h3>
            {installationBusy ? (
              <p className="muted">Cargando…</p>
            ) : installation ? (
              <dl className="settings-meta-dl">
                <div>
                  <dt>Moneda base</dt>
                  <dd>{installation.installation.base_currency}</dd>
                </div>
                <div>
                  <dt>Tu rol</dt>
                  <dd>{installation.role}</dd>
                </div>
              </dl>
            ) : (
              <p className="muted tight">Sin acceso.</p>
            )}
          </section>

          <section className="panel dev-panel">
            <h3 className="panel-title">Estado del sistema</h3>
            {healthError ? (
              <p className="error compact">
                <code>/v1/health</code>: {healthError}
              </p>
            ) : health ? (
              <dl className="health-dl">
                <div>
                  <dt>Servicio</dt>
                  <dd>{health.service}</dd>
                </div>
                <div>
                  <dt>Versión API</dt>
                  <dd>{health.version}</dd>
                </div>
                <div>
                  <dt>Estado</dt>
                  <dd>{health.status}</dd>
                </div>
              </dl>
            ) : (
              <p className="muted">Comprobando…</p>
            )}
          </section>
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
                      <p className="hint">Sin categoría sustituta en el ámbito.</p>
                    );
                  }
                  return (
                    <label className="field">
                      <span>Reasignar a</span>
                      <select
                        value={categoryRemapToId}
                        onChange={(e) => setCategoryRemapToId(e.target.value)}
                        aria-label="Categoría destino para remap"
                      >
                        {siblings.map((s) => (
                          <option key={s.id} value={s.id}>
                            {s.name}
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
              <ul className="muted tight" style={{ paddingLeft: "1.2em" }}>
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
