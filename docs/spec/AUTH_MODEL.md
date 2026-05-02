# FutureFin Server — Modelo multi-usuario (MVP)

Este documento fija el **contrato de identidad y autorización** para la línea self-hosted (Docker). El cliente macOS legacy era **monousuario / monoinstalación** con filtro por persona solo en UI; el servidor modela **cuentas** (`User`), **una instalación de producto por despliegue** y **membresías con alta controlada por el owner**. Cada cuenta que debe acceder al dato compartido se da de alta **registrándose en la pantalla de acceso**; no hay invitaciones por correo ni entidad separada de «persona cliente» para alta de miembros.

## Alcance por instalación

- **Una instalación** (un despliegue Docker / una base de datos de producto) tiene **exactamente un contexto financiero compartido**: una fila de configuración (`installation`) y datos de dominio (activos, categorías, etc.) enlazados a esa fila cuando el modelo los exponga.
- No hay **selector de varios espacios** ni creación libre de espacios paralelos por parte del usuario. Todos los miembros autorizados trabajan sobre el mismo ámbito persistido de esa instalación.

## Entidades

### User (cuenta)

- Identificador estable (`user_id`).
- Credenciales gestionadas por el stack elegido (usuario + contraseña con hash fuerte en MVP; OAuth opcional post-MVP).
- Sin datos financieros persistentes **fuera** del ámbito de la instalación salvo preferencias de cuenta (locale, tema).

### Installation (configuración del despliegue)

- **Singleton por base de datos:** una fila por instalación.
- Campos de negocio relevantes: **moneda base**, inflación de proyección, edad objetivo de horizonte, `show_age_mode`, ajustes FIRE, etc.

### InstallationMembership

- `user_id` + `installation_id` + `role`.
- Solo existe **un** `installation_id` válido para datos de producto; todos los miembros apuntan a esa instalación.
- **Alta de miembros:** cualquiera puede **registrarse** (cuenta `User`). Un usuario **no** obtiene acceso a la instalación hasta que el **`owner`** lo **apruebe** y le asigne rol (`member` o `viewer`). Los usuarios registrados sin fila en `installation_memberships` son «pendientes». Hasta la aprobación, no deben ver datos de la instalación.

### Persona en datos financieros (paridad Mac, fuera del contrato de alta)

- El cliente de referencia filtra y etiqueta por «persona» en activos, presupuesto, etc. Esa atribución de dominio se alineará cuando el backend exponga el modelo financiero completo; **no** debe confundirse con el alta de miembros: quien inicia sesión es siempre un `User`.

## Vistas individual vs conjunta (UX)

- Los datos financieros son **un solo conjunto** persistido en el servidor para esa instalación.
- La interfaz ofrece al menos dos **modos de visualización** (filtros de cliente, no muros de autorización entre cónyuges en MVP):
  - **Vista individual:** métricas y listas acotadas a lo asociado a una persona (o al usuario encaja con «su» persona cuando exista modelo).
  - **Vista conjunta:** todo el ámbito agregado (equivalente a «todo el hogar» en el Mac).
- Este comportamiento replica la idea del **PersonFilterBar** del cliente de referencia: mismo dataset compartido, distinto alcance de UI.

## Visibilidad y privacidad entre miembros

- **Por defecto (MVP):** miembros con rol `member` u `owner` trabajan sobre el **mismo universo de datos**; la separación «solo yo» es vía **filtro de vista**, no vía ocultación servidor entre miembros.
- **Ámbitos privados por persona** («solo yo veo este activo»): **fuera de MVP** salvo requisito legal explícito.

## Roles (MVP)

| Rol      | Descripción                                                                                                                                 |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `owner`  | **Aprobar** altas de otros usuarios (pendientes tras registro), quitar miembros (salvo reglas de último owner), backups/export completos, CRUD financiero. |
| `member` | CRUD financiero completo (equivalente al uso compartido del Mac).                                                                           |
| `viewer` | Solo lectura de vistas y métricas (opcional si se prioriza solo owner/member en v1).                                                      |

**Decisión MVP:** al menos `owner` y `member`; `viewer` recomendado si el esfuerzo es bajo.

## Autorización API

- El servidor resuelve el **`installation_id`** del singleton sin que el cliente elija entre varios ámbitos.
- Valida que `current_user` tenga membresía **activa** (tras aprobación por el owner) con rol suficiente.
- **viewer:** GET permitido, mutaciones denegadas.

## Sesiones y seguridad

- HTTPS terminado en reverse proxy; cookies seguras HttpOnly o tokens de corta vida según stack.
- Rate limiting en login; auditoría mínima opcional post-MVP.

## Relación con backups

- Con **una instalación por despliegue**, el backup **del ámbito financiero** coincide con el backup **de la instalación** para efectos prácticos. Ver `[BACKUP_AND_CSV_SPEC.md](./BACKUP_AND_CSV_SPEC.md)`.
- Solo `owner` (y opcionalmente `member` si el producto lo permite) ejecutan restore destructivo.

## Preguntas cerradas para implementación

1. **¿Varios ámbitos paralelos por instalación?** → **No.** Un singleton por base de datos.
2. **¿Nombre / etiqueta editable del espacio compartido?** → **No en MVP** (no hay CRM multi-tenant).
3. **¿Alta de miembros?** → Registro libre como `User`; acceso a la instalación solo tras **aprobación por el `owner`**.
4. **¿Roles mínimos?** → **owner + member**; **viewer** recomendado.
5. **¿Privacidad por persona entre miembros?** → **No en MVP** (filtros de vista individual/conjunta en cliente).
