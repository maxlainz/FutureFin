# FutureFin Server — Modelo multi-usuario (MVP)

Este documento fija el **contrato de identidad y autorización** para la línea self-hosted (Docker). El cliente macOS legacy era **monousuario / monoinstalación** con filtro por persona solo en UI; el servidor debe modelar **cuentas**, **hogares** y **membresías** de forma explícita.

## Entidades

### User (cuenta)

- Identificador estable (`user_id`).
- Credenciales gestionadas por el stack elegido (email+password con hash fuerte, OAuth opcional post-MVP).
- Sin datos financieros “propios” fuera del contexto de un hogar salvo preferencias de cuenta (locale, tema).

### Household (hogar)

- Equivale a `Household` en dominio (`Domain.swift` en el repo Swift de referencia FutureFin / FinFuture).
- Pertenece al modelo de datos compartido por miembros autorizados.
- Campos de negocio: nombre, moneda base, inflación de proyección, edad objetivo de horizonte, `show_age_mode`, etc.

### HouseholdMembership

- `user_id` + `household_id` + `role`.
- Un usuario puede pertenecer a **varios hogares** (caso típico: pareja con dos instancias self-hosted es raro; más habitual varios hogares en una misma instalación si el producto lo permite — **MVP recomendado: sí**, varios hogares por usuario con selector de contexto).

### Person (miembro del hogar)

- Entidad de dominio existente: persona **dentro** de un hogar, con `owner_person_id` en activos/pasivos/presupuesto/planning.
- **No** confundir `User` con `Person`: un `User` humano puede “ser” una `Person` del hogar opcionalmente (`person.user_id` opcional post-MVP); para MVP basta con que usuarios con rol adecuado editen todas las personas del hogar.

## Roles (MVP)

| Rol | Descripción |
|-----|-------------|
| `owner` | Invitar/quitar miembros (salvo último owner), eliminar hogar, backup/export completos, todos los CRUD financieros. |
| `member` | CRUD financiero completo (equivalente al uso actual del Mac en hogar compartido). |
| `viewer` | Solo lectura de todas las vistas y métricas (opcional si se prioriza solo owner/member en v1). |

**Decisión MVP:** incluir al menos `owner` y `member`; `viewer` recomendado si el esfuerzo es bajo.

## Visibilidad de datos financieros

- **Por defecto (MVP):** todos los miembros del hogar con rol `member` u `owner` ven **el mismo universo de datos** del hogar (activos, pasivos, presupuesto, upcoming, FIRE settings del hogar). El **filtro por persona** del Mac se replica como **filtro de visualización** en cliente, no como muro de autorización entre cónyuges.
- **Ámbitos privados por persona** (“solo yo veo este activo”): **fuera de MVP** salvo requisito legal explícito; documentar como extensión futura (`visibility` por ítem o vínculo Person↔User).

## Autorización API

- Todas las rutas llevan contexto `household_id` (o tenant equivalente) tras selección en UI.
- El servidor valida: `current_user` tiene `HouseholdMembership` para ese `household_id` con rol suficiente (lectura vs escritura).
- **viewer:** GET permitido, mutaciones denegadas.

## Sesiones y seguridad

- HTTPS terminado en reverse proxy; cookies seguras httpOnly o tokens de corta vida en header según stack.
- Rate limiting en login; auditoría mínima de accesos opcional post-MVP.

## Relación con backups

- Export monofichero y ZIP CSV deben estar **acotados al hogar** o **a la instalación** según [`BACKUP_AND_CSV_SPEC.md`](./BACKUP_AND_CSV_SPEC.md). Solo `owner` (y opcionalmente `member` si producto lo permite) ejecutan restore destructivo.

## Preguntas cerradas para implementación

1. **¿Multi-hogar por usuario?** → **Sí (MVP).**
2. **¿Roles mínimos?** → **owner + member**; **viewer** recomendado.
3. **¿Privacidad por persona entre miembros?** → **No en MVP** (solo filtro UX).
