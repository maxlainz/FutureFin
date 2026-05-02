# FutureFin Server — Modelo multi-usuario (MVP)

Este documento fija el **contrato de identidad y autorización** para la línea self-hosted (Docker). El cliente macOS legacy era **monousuario / monoinstalación** con filtro por persona solo en UI; el servidor modela **cuentas**, un **único hogar por instalación**, **membresías con alta controlada por el owner** y **personas de dominio** para atribución de datos.

## Alcance por instalación

- **Una instalación** (un despliegue Docker / una base de datos de producto) tiene **exactamente un `Household`** en el modelo lógico.
- No hay **selector de varios hogares** ni creación libre de hogares adicionales por parte del usuario. El hogar existe como contexto compartido de todos los miembros autorizados de esa instalación.
- El **nombre mostrado del hogar no es un campo configurable** por el usuario en MVP: puede ser una etiqueta fija de producto (p. ej. «FutureFin» / instalación) o derivada del entorno; no forma parte de los ajustes editables como en un CRM multi-tenant.

## Entidades

### User (cuenta)

- Identificador estable (`user_id`).
- Credenciales gestionadas por el stack elegido (usuario + contraseña con hash fuerte en MVP; OAuth opcional post-MVP).
- Sin datos financieros persistentes **fuera** del hogar de la instalación salvo preferencias de cuenta (locale, tema).

### Household (hogar)

- Equivale conceptualmente a `Household` en dominio (`Domain.swift` en el repo Swift de referencia FutureFin / FinFuture).
- **Singleton por instalación:** una fila (o equivalente) por base de datos de aplicación.
- Campos de negocio relevantes para el producto: **moneda base**, inflación de proyección, edad objetivo de horizonte, `show_age_mode`, ajustes FIRE del hogar, etc. — **no** incluye un nombre personalizado editable por el usuario en MVP.

### HouseholdMembership

- `user_id` + `household_id` + `role`.
- En esta instalación solo existe **un** `household_id` válido para datos de producto; todos los miembros apuntan a ese hogar.
- **Alta de miembros:** un usuario **no** obtiene acceso al hogar por sí solo. El **`owner`** debe **invitar** y **aceptar / aprobar** la incorporación del nuevo usuario (flujo de invitación + estado pendiente hasta confirmación del owner, o equivalente documentado en API). Hasta entonces, ese usuario no tiene membresía activa y no debe ver datos del hogar.

### Person (miembro del hogar — dominio)

- Persona **dentro** del hogar (titular familiar, cónyuge, hijos, etc.), con `owner_person_id` en activos/pasivos/presupuesto/planning en el modelo financiero.
- **No** confundir `User` (cuenta que inicia sesión) con `Person`: pueden existir varios `User` miembros y varias `Person`; el vínculo opcional `person.user_id` puede llegar post-MVP. En MVP basta con roles de membresía + personas editables por quien tenga permiso.

## Vistas individual vs conjunta (UX)

- Los datos financieros del hogar son **un solo conjunto** persistido en el servidor para esa instalación.
- La interfaz ofrece al menos dos **modos de visualización** (filtros de cliente, no muros de autorización entre cónyuges en MVP):
  - **Vista individual:** métricas y listas acotadas a lo asociado a una persona (o al usuario encaja con «su» persona cuando exista modelo).
  - **Vista conjunta:** todo el hogar agregado (equivalente a «todo el hogar» en el Mac).
- Este comportamiento replica la idea del **PersonFilterBar** del cliente de referencia: mismo dataset compartido, distinto alcance de UI.

## Visibilidad y privacidad entre miembros

- **Por defecto (MVP):** miembros con rol `member` u `owner` trabajan sobre el **mismo universo de datos** del hogar; la separación «solo yo» es vía **filtro de vista**, no vía ocultación servidor entre miembros del mismo hogar.
- **Ámbitos privados por persona** («solo yo veo este activo»): **fuera de MVP** salvo requisito legal explícito.

## Roles (MVP)


| Rol      | Descripción                                                                                                                                 |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `owner`  | **Invitar**, **aprobar** altas de otros usuarios, quitar miembros (salvo reglas de último owner), backups/export completos, CRUD financiero. |
| `member` | CRUD financiero completo (equivalente al uso compartido del Mac en hogar).                                                                   |
| `viewer` | Solo lectura de vistas y métricas (opcional si se prioriza solo owner/member en v1).                                                      |

**Decisión MVP:** al menos `owner` y `member`; `viewer` recomendado si el esfuerzo es bajo.

## Autorización API

- El servidor resuelve el **`household_id`** del singleton de la instalación (o equivalente) sin que el cliente elija entre varios hogares.
- Valida que `current_user` tenga `HouseholdMembership` **activa** (tras invitación aceptada por el owner) con rol suficiente.
- **viewer:** GET permitido, mutaciones denegadas.

## Sesiones y seguridad

- HTTPS terminado en reverse proxy; cookies seguras HttpOnly o tokens de corta vida según stack.
- Rate limiting en login; auditoría mínima opcional post-MVP.

## Relación con backups

- Con **un hogar por instalación**, el backup **del hogar** coincide con el backup **de la instalación** para efectos prácticos. Ver `[BACKUP_AND_CSV_SPEC.md](./BACKUP_AND_CSV_SPEC.md)`.
- Solo `owner` (y opcionalmente `member` si el producto lo permite) ejecutan restore destructivo.

## Preguntas cerradas para implementación

1. **¿Varios hogares por usuario o por instalación?** → **No.** Un hogar por instalación; sin selector multi-hogar.
2. **¿Nombre del hogar editable?** → **No en MVP** (etiqueta fija / no configurable por usuario).
3. **¿Alta de miembros?** → Solo tras **invitación y aceptación por el `owner`** (sin auto-incorporación al dataset compartido).
4. **¿Roles mínimos?** → **owner + member**; **viewer** recomendado.
5. **¿Privacidad por persona entre miembros?** → **No en MVP** (filtros de vista individual/conjunta en cliente).
