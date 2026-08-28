---
name: futurefin-config-and-flags
description: >
  Catalog of every configuration axis in FutureFin: environment variables of the API binary (PORT,
  DATABASE_URL, SESSION_TTL_DAYS, COOKIE_SECURE, CORS_ORIGINS, WEB_STATIC_ROOT, RUST_LOG,
  FUTUREFIN_DB_CONNECT_TIMEOUT_SECS, FUTUREFIN_MCP_ENABLED, FUTUREFIN_PUBLIC_URL,
  FUTUREFIN_RECONCILE_SWEEP_HOURS, FUTUREFIN_BASE_PATH, FUTUREFIN_TRUSTED_PROXY_IPS,
  FUTUREFIN_TRUSTED_PROXY_AUTH, FUTUREFIN_HA_SSO_URL, FUTUREFIN_HA_ADDON,
  FUTUREFIN_API_PORT, WEB_DEV_PORT,
  TEST_DATABASE_URL) and of
  the self-contained container entrypoint (FUTUREFIN_DB_MODE, FUTUREFIN_MODE, FUTUREFIN_BACKUP_KEEP*,
  FUTUREFIN_PREMIGRATION_BACKUP, FUTUREFIN_ALLOW_EPHEMERAL_DB, FUTUREFIN_STATE_DIR, POSTGRES_*),
  the Home Assistant add-on options and how the entrypoint maps them to env
  (/data/options.json: log_level, sso, mcp, cors_origins, public_url, ha_sso_url;
  /data/pgdata + /data/state),
  deployment knobs (FUTUREFIN_IMAGE/TAG, APP_PORT), the three docker-compose files
  (prod single-service, dev standalone, local-image override), API query-parameter flags
  (?view=mine, ?months, ?density=hybrid), request-body limits, and per-installation runtime
  settings (PATCH /v1/installation: base_currency, calendar_tz, show_age_mode,
  annual_inflation_assumption_percent, the fire_settings JSONB with swr_pct and tax_brackets
  bounds, and mcp_write_enabled — the live kill-switch of the MCP write tools). Load this skill when you need to know what an option is called, its default, its
  validation bounds, WHICH FILE parses it (Rust binary vs entrypoint vs compose), whether it is
  prod or dev-only, why production needs no env var at all since 3.0.0, why setting DATABASE_URL in
  the container now makes it either ignore the variable or refuse to start (external databases were
  RETIRED in 4.0.0), why a setting change returns 400, why an env
  var "isn't taking effect" (.env precedence), why CORS panics at startup, or when ADDING a new
  env var / installation setting / query param. Do NOT load it for step-by-step environment setup
  (use futurefin-build-and-env), deployment/upgrade/backup operations (futurefin-run-and-operate),
  or the MEANING of the FIRE math these settings feed (futurefin-fire-domain-reference).
---

# FutureFin configuration and flags

Env/compose/entrypoint facts re-verified on **2026-08-16 for v3.0.0** (the self-contained-image
release), plus the **v3.1.0 additions of 2026-08-17** (`FUTUREFIN_PUBLIC_URL`, the widened
`FUTUREFIN_MCP_ENABLED` scope) and the **v4.4.0 MCP-transport hardening of 2026-08-28** (issue #85,
§1.1's `FUTUREFIN_MCP_ENABLED`/`FUTUREFIN_PUBLIC_URL`/`CORS_ORIGINS` rows, §4's body-limit table);
the query-param, body-limit and installation-settings sections were
last verified 2026-07-02 (v1.4.3) plus the v1.5.x/v1.6.0/v1.8.0/v2.x additions noted inline — plus
the **Fase 5 (issue #86) MCP-context pass, also 2026-08-28 (still 4.4.0)**, which changed §4's
`GET /v1/history/series` window default and corrected a stale `GET /v1/history/cashflow` claim
(§4 `window_months` rows), and added a new MCP-tool-pagination subsection — none of those
introduced env vars either. This skill is the single home for "what can be configured, where, with
what bounds".

**What 4.4.0 changed (no new env var — read this before trusting the 3.1.0 note below)**: no new
variable, but three existing ones changed *behavior* under the same name — the transport-hardening
pass that closed issue #85. `FUTUREFIN_MCP_ENABLED=0` **no longer unmounts routes** — `/mcp` and the
OAuth protocol routes stay mounted and answer 404 JSON `mcp_disabled` (fixing a real incident: the
published image's `ServeDir` fallback doesn't call itself for non-GET/HEAD, so the old unmount
produced a 405-empty-body or a 200-text/html that broke the claude.ai connector). `FUTUREFIN_PUBLIC_URL`
now **accepts a subpath**, validated by the same `prefix::normalize_prefix` that already validated
`FUTUREFIN_BASE_PATH` — the fix for OAuth/MCP behind a reverse-proxy subpath, which previously had
none. `CORS_ORIGINS` now feeds **two** CORS layers instead of one — the API layer keeps
`allow_credentials(true)`, the new `/mcp`-only layer has none — closing a real credential leak (any
origin added for a browser MCP client used to also get cookie access to `/v1`). Also new: explicit
1 MiB body cap on `/mcp` (§4) and `Origin`-header validation on `/mcp` (rmcp's default was "off").

**What 3.1.0 changed**: the embedded OAuth 2.1 authorization server added exactly **one** optional
env var, `FUTUREFIN_PUBLIC_URL` (§1.1) — production still needs none, because the issuer is derived
from the request headers by default. `FUTUREFIN_MCP_ENABLED` now gates OAuth too, with one
deliberate exception (§1.1). No new installation setting, no new query param.

**What 3.0.0 changed (read this before trusting any older note):** the Docker image is
**self-contained** — PostgreSQL 16 runs *inside* the single `futurefin` container over a Unix
socket (`/var/run/postgresql`, no TCP), so the compose service `futurefin-database` is gone,
**no environment variable is required in production** (an empty `.env` works), `DATABASE_URL` is
no longer composed for you (**4.0.0 retired external databases altogether** — setting it now is
either ignored or a hard startup refusal, §1.2), and a
new family of `FUTUREFIN_*` variables is parsed by the container **entrypoint**, not by the Rust
binary (§1.2). `docker-compose.split-dev.yml` was replaced by the standalone
`docker-compose.dev.yml` (§3).

Vocabulary used below:
- **Installation** — the singleton row in table `installation`; one per deployment; all financial
  data belongs to it. Its columns are the *runtime* settings (changed via API, stored in DB).
- **SWR** — Safe Withdrawal Rate: the % of net worth withdrawn per year in retirement.
  `FIRE number = annual expenses / (SWR/100)`.
- **split-dev** — the two-process dev mode: `cargo run` API on :8081 + Vite dev server on :8080,
  against the standalone dev Postgres of `docker-compose.dev.yml` on 127.0.0.1:5432.
- **Embedded / external DB** — embedded = the PostgreSQL inside the image (the only option since
  4.0.0); external = a separate Postgres reached via `DATABASE_URL` (2.x behavior, deprecated in
  3.0.0, **removed in 4.0.0**: the container no longer speaks to one). `DATABASE_URL` itself is
  alive and **required in split-dev**, where `cargo run` talks to `docker-compose.dev.yml`.
- **Nominal vs real** — nominal = future euros; real = deflated to today's purchasing power.

## When NOT to use this skill

- Recreating a working dev environment from scratch, toolchain issues, local image builds →
  `.claude/skills/futurefin-build-and-env/SKILL.md`.
- Deploying, upgrading, rollback, backups, logs, smoke tests →
  `.claude/skills/futurefin-run-and-operate/SKILL.md`.
- What `swr_pct`, gross-up, the inflation-growing FIRE target, or the allocation cascade *mean*
  and how the engine consumes them → `.claude/skills/futurefin-fire-domain-reference/SKILL.md`.
- Changing behavior behind a config axis (new semantics, migrations) → gates in
  `.claude/skills/futurefin-change-control/SKILL.md`.
- curl recipes to observe cache hits, densities, etc. →
  `.claude/skills/futurefin-diagnostics-and-tooling/SKILL.md`.

## 1. Environment variable catalog

Three different processes parse configuration, and confusing them is the usual source of "my env
var does nothing": **§1.1** the Rust binary (`apps/api/src/main.rs`), **§1.2** the container
entrypoint (`apps/api/docker-entrypoint.sh`, Docker image only), **§1.3** compose itself
(`docker-compose*.yml`, substituted before any container starts).

### 1.1 API runtime (parsed in `apps/api/src/main.rs`)

| Variable | Default | Bounds / parsing | Prod or dev | Notes |
|---|---|---|---|---|
| `DATABASE_URL` | **none — the binary still panics with `expect` if unset** | any Postgres URL | both | **Changed in 3.0.0, narrowed in 4.0.0.** In the image you never set it: the entrypoint `export`s `postgres:///$POSTGRES_DB?host=/var/run/postgresql&user=$POSTGRES_USER` (Unix socket) right before launching the binary, overwriting whatever was there. Setting it yourself no longer selects anything — **external DBs were removed in 4.0.0**: a URL not containing `/var/run/postgresql` is *ignored with a warning* when the volume already holds a cluster, and makes the entrypoint **abort** (`refuse_external_database`, exit 1) when it does not (§1.2). In split-dev you still set it by hand in `.env`: `postgres://futurefin:futurefin@127.0.0.1:5432/futurefin`. |
| `FUTUREFIN_DB_CONNECT_TIMEOUT_SECS` | `30` | u64, **1–600**; out of range or unparseable → **silently** 30 | both (new in 3.0.0) | Total budget for `db::connect_with_retry` (`apps/api/src/db.rs`), which retries with backoff 0.5s → 1s → 2s → 4s → 4s… instead of crash-looping. In the container it rarely matters — the entrypoint has already waited on `pg_isready` before starting the API. It does matter in split-dev, where `cargo run` can start before `docker-compose.dev.yml`'s Postgres is accepting connections. |
| `PORT` | `8080` | u16; unparseable → silently falls back to 8080 | both | API listen port, binds `0.0.0.0`. Use `8081` in split-dev so Vite can take 8080. Container always runs with `PORT=8080` — since 3.0.0 that comes **only** from the Dockerfile `ENV` (the prod compose no longer restates it); the host side is `APP_PORT`. |
| `SESSION_TTL_DAYS` | `30` | integer **1–400**; out-of-range or unparseable → **silently** 30 | both | Session cookie/DB row lifetime. Stored in `AppState.session_ttl_days`. |
| `COOKIE_SECURE` | `false` | true only for exact strings `1`, `true`, `TRUE`, `yes`, `YES` (`parse_bool_env`). `True`, `Yes`, `on` etc. parse as **false** | prod (behind HTTPS) | Sets the `Secure` attribute on the `ff_session` cookie. |
| `FUTUREFIN_MCP_ENABLED` | `true` | `parse_bool_env` (same quirk as `COOKIE_SECURE`: only `1/true/TRUE/yes/YES` are true — but here **unset → true**, any other string → false) | both (new in 3.0.0) | Parsed by `main.rs` into `AppState.mcp_enabled`. **Since 4.4.0 (issue #85, **doctrine D21**) the switch no longer unmounts routes**: `false` still mounts `/mcp` and the 7 root OAuth-protocol routes (`/.well-known/oauth-protected-resource[/mcp]`, `/.well-known/oauth-authorization-server[/mcp]`, `POST /oauth/register\|token\|revoke`), and every method on all of them returns **404 JSON `{code: "mcp_disabled"}`** (`mcp::MCP_DISABLED_MESSAGE`) instead of disappearing. **Why it changed — the incident**: unmounting only looked broken in the *published* image, whose final fallback is a `ServeDir` that does **not** call its own fallback for methods other than GET/HEAD — `POST /mcp` came back **405 with an empty body**, `GET /.well-known/oauth-authorization-server` came back **200 `text/html`** (the SPA shell). claude.ai's connector failed to parse JSON and reported "connection failed" with no clue why: a security control that, once tripped, is diagnosed as an outage. The old test built the router *without* the SPA mounted, so it confirmed a 404 that production never produced — it described a lab binary. **Doctrine D21** (not D18 — D18 is proxy-header trust; D21 is the one that generalises it to the router, and says so in its own text): router shape never depends on the environment — the same reasoning `mcp/mod.rs`'s own doc comment cites for `POST /v1/auth/sso` always mounting and answering `sso_disabled` when off. **Unchanged**: `GET /v1/oauth/authorize-details` and `POST /v1/oauth/authorize` still don't get mounted (they live under `/v1`, whose fallback already returned JSON), and `/v1/api-tokens` + `GET/DELETE /v1/oauth/connections[/{id}]` stay mounted unconditionally (`oauth_consent_router(mcp_enabled)` only gates the flow half) — turning MCP off must never strip your ability to revoke credentials you already granted. Default enabled: the surface is inert without credentials (everything 401s) and prod keeps its zero-required-vars story. Tested with the **real** static-file fallback mounted, not a lab router: `mcp_http.rs::mcp_disabled_answers_json_even_with_the_spa_mounted`, `oauth_flow.rs::oauth_protocol_disabled_with_mcp_but_connections_panel_survives` (both via `TestConfig::web_static_root`, §10). |
| `FUTUREFIN_RECONCILE_SWEEP_HOURS` | `24` | integer **0–168**; **0 = disabled**; out-of-range or unparseable → silently 24 (same laxity as `SESSION_TTL_DAYS`) | both (new in 3.8.1) | Horas entre **barridos de conciliación de transferencias**, la primera tarea periódica del binario (`main.rs::spawn_reconcile_sweep` → `handlers::transactions::reconcile::sweep_all_owners`). NO es el mecanismo principal: el pase automático ya corre tras **cada** mutación (alta, edición, borrado, import CSV, materialización de recurrentes). El barrido es la **red de reintento** de esos pases, que son best-effort y se tragan sus errores para no convertir una escritura ya persistida en un 5xx — sin él, un fallo puntual deja el par sin conciliar de forma permanente y silenciosa. La primera pasada corre **tras el primer intervalo, no al arrancar** (en el arranque no ha pasado nada que conciliar, y competir con migraciones y warm-up no compra nada). Se aborta ANTES de cerrar el pool en el apagado ordenado. En una instalación sana no encuentra nada y loguea a `debug`; solo sube a `info` si concilió algo o si algún owner falló. |
| `FUTUREFIN_PUBLIC_URL` | unset → **derived per request** | must parse as a URL, scheme `http`/`https`, host present; **since 4.4.0 (issue #85, finding 2) accepts a subpath**, validated by `prefix::normalize_prefix` — the SAME already-tested function that validates `FUTUREFIN_BASE_PATH`: starts with `/`, no `//`, no `.`/`..` segments, charset `[A-Za-z0-9._~/-]` (`%` deliberately excluded), ≤128 chars, one trailing slash trimmed, bare `/` ⇒ root. Query and fragment are still forbidden. Normalized to `origin + prefix` (`Url::origin().ascii_serialization()` then the normalized path, no trailing slash). Present-but-invalid → **panic at startup** (fail-loud, like `CORS_ORIGINS`; the function now has 5 panic sites, one per validation step) | prod, **optional** (new in 3.1.0, subpath since 4.4.0) | Parsed by the **Rust binary** (`main.rs::public_url()`) into `AppState.public_url`; consumed by `oauth::url::public_base_url` as the OAuth **issuer** and as the base of every URL in the `.well-known` metadata, the `iss` of the authorize redirect, the RFC 8707 `resource` (`{issuer}/mcp`), and the `resource_metadata` of the `/mcp` 401 challenge. Unset (the default and the normal case) → derived from the request: `X-Forwarded-Proto` + `X-Forwarded-Host` (first value of each) else the `Host` header, through a strict charset (`host[:port]`, bracketed IPv6, ≤255 chars, no `/ @`, spaces or control chars) → a bad host is **400 `invalid_request`**, no host at all likewise; **the request-derived path is never used as a prefix** — see below. **Set it when your reverse proxy sends neither `X-Forwarded-*` nor a public `Host`, or when it serves the app under a subpath** — either way the metadata would otherwise advertise an unreachable or unprefixed issuer and claude.ai reports "connection failed". **Why the prefix isn't composed from `prefix::request_prefix` instead** (that function exists and is trusted for asset rewriting, §1.1's `FUTUREFIN_BASE_PATH` row): (1) the issuer is an **identity**, not a decoration — `prefix.rs` requires no trusted peer for the prefix because a forged `X-Forwarded-Prefix` only deforms the *attacker's own* asset response, but the instant that text lands in a **discovery document** it stops being harmless, and only a fail-loud operator value can carry that weight — a header must never be able to move an identity; (2) under the Home Assistant Ingress the prefix is `/api/hassio_ingress/<token>`, an **ephemeral session token** — baking it into the issuer would be wrong, and moot in practice since the add-on documents MCP/OAuth going through the direct port, not the Ingress; (3) it reuses `normalize_prefix`, already tested. With a proxy prefix on the request (`X-Ingress-Path`/`X-Forwarded-Prefix`) and this var unset, `oauth::url::warn_missing_public_url_for_prefix` logs a **`warn` once per process** — the symptom without that line was a silent 404 on `/oauth/token`. Irrelevant with `FUTUREFIN_MCP_ENABLED=0`. Echoed in the startup `"server config"` line as `public_url=…` or `(derived from request)`. Tests never set it (`TestApp::spawn` passes `None`), which is what makes the forwarded-header derivation testable; the subpath case is `oauth_flow.rs::public_url_with_a_subpath_prefixes_every_advertised_url`. |
| `CORS_ORIGINS` | `http://127.0.0.1:5173,http://localhost:5173,http://127.0.0.1:8080,http://localhost:8080` | comma-separated origins, entries trimmed, empties dropped; an unparseable entry **panics at startup**; empty result panics | prod, only for cross-origin API access | Parsed once in `routes::cors_origins()` (moved out of `main.rs` in 4.4.0). **Feeds two CORS layers with different privileges since 4.4.0** (issue #85, finding 4) — until 4.3.1 there was a single layer over the whole router with `allow_credentials(true)`, so adding an origin to make a browser MCP client work (the MCP Inspector) also handed it **cookie** access to `/v1/backup/user-export`, `/v1/api-tokens` and `/v1/installation`. Now: **API layer** (`routes::api_cors_layer`, covers `/v1/*`, `/health`, `/openapi.json`, the OAuth protocol routes) — `allow_credentials(true)` (credential = the `ff_session` cookie), methods GET/POST/PATCH/DELETE/OPTIONS, headers `content-type`/`accept`/`authorization` (the last for OAuth's `client_secret_basic`). **MCP layer** (`mcp::mcp_cors_layer`, covers only `/mcp`) — **no `allow_credentials`** (credential = the `Authorization` header), methods GET/POST/DELETE/OPTIONS, headers `content-type`/`accept`/`authorization`/`mcp-session-id`/`mcp-protocol-version`/`last-event-id`/`mcp-method`/`mcp-name`, exposing `mcp-session-id`/`mcp-protocol-version`/`www-authenticate` (the last because it isn't a safelisted response header — unexposed, a browser client can never read the 401's `resource_metadata=` and discover the authorization server, RFC 9728 §5.1). The MCP-specific headers exist because `MCP-Protocol-Version` is mandatory on every non-`initialize` request since the 2025-06-18 revision, `Last-Event-ID` resumes a cut SSE stream, and `Mcp-Method`/`Mcp-Name` are SEP-2243's routing mirror — missing any of them from `allow_headers` fails the browser preflight with a CORS error that never mentions MCP, indistinguishable from "the server is down". `Mcp-Param-*` (SEP-2243) is deliberately **absent**: it's a header-name *prefix*, and `allow_headers` only takes exact names; no known client sends it today, and the alternative (`AllowHeaders::mirror_request`) would trade an auditable list for a mirror. The same origin list also feeds `/mcp`'s `Origin` validation (`StreamableHttpServerConfig::with_allowed_origins`, §1.1 has no separate row for it — see the provenance grep below). **Two axum traps that must stay documented**: (1) `Router::layer` only wraps routes **already registered**, so in `routes::app_router` the `mcp` router is `.merge()`d in **after** `.layer(api_cors_layer(...))` — move that merge earlier and `/mcp` inherits `allow_credentials(true)`; (2) inside `mcp::mcp_router`, the CORS layer is applied with **`route_layer`, never `layer`** — `Router::layer` also wraps a router's **fallback**, and `merge` drags that fallback into the destination router, so every unknown route (including `/oauth/authorize`, the SPA's consent screen) would go through MCP's Bearer auth and come back 401; caught by `oauth_flow.rs::get_oauth_authorize_is_not_handled_by_the_api`. Regression for the split itself: `mcp_http.rs::mcp_preflight_is_complete_and_grants_no_cookie_access`. Same-origin deployments (the normal Docker image) never send CORS preflights, so the default is fine either way. |
| `WEB_STATIC_ROOT` | unset | path; empty/whitespace value treated as unset; set-but-missing path → startup warning, API-only mode | prod (Docker sets `/app/web`) | When the path exists, the SPA is served from it with `index.html` fallback (single-port mode). Omit in split-dev — Vite serves the UI. |
| `RUST_LOG` | `futurefin_api=info,tower_http=info,sqlx=warn` | tracing `EnvFilter` syntax; invalid filter → the default is used | both | Default is applied in `main.rs` when the env filter can't be built from the env. |
| `FUTUREFIN_BASE_PATH` | unset → `""` (root) | normalized by `prefix::validate_base_path_env`: `""` or `/` ⇒ root; otherwise must start with `/`, ≤128 chars, charset `[A-Za-z0-9._~/-]`, no `//`, no `.`/`..` segments (one trailing slash is trimmed). Present-but-invalid → **panic at startup** (fail-loud, like `FUTUREFIN_PUBLIC_URL`) | prod, **optional** (new 2026-08-27) | Public subpath for deployments behind a reverse proxy that does **not** send `X-Forwarded-Prefix`. The server always mounts its routes at the root — the proxy strips the prefix; what needs it is what the *browser* resolves: asset refs in the HTML shell, fetch URLs, and the cookie `Path`. It is the **lowest-precedence** source: `X-Ingress-Path` > `X-Forwarded-Prefix` > `FUTUREFIN_BASE_PATH` > `""` (`prefix::request_prefix`), so one binary serves compose at `/` and HA Ingress under `/api/hassio_ingress/<token>` **at the same time**. An invalid *header* is ignored (deduped `warn`, ≤8 distinct values) and the next source is used — only the env var panics. Detection needs no trusted peer on purpose: a forged prefix only breaks the attacker's own page. |
| `FUTUREFIN_TRUSTED_PROXY_IPS` | unset → `PeerPolicy::Disabled` (**nobody is trusted**) | `any` (case-insensitive) ⇒ every peer, including an unknown one; otherwise a comma-separated IP list (entries trimmed, empties dropped). An unparseable entry **panics**; a list that resolves empty **panics** | prod, **optional** (new 2026-08-27) | `prefix::PeerPolicy`, stored in `AppState.trusted_peers`. Gates the **two** things a header must never buy on its own: relaxing anti-clickjacking (`handlers/frame.rs` → `frame-ancestors 'self'` instead of `X-Frame-Options: DENY`) and accepting identity headers (`POST /v1/auth/sso`). The peer is the **TCP peer** (`ConnectInfo`), not `X-Forwarded-For`. The HA add-on exports `172.30.32.2` (the Supervisor's ingress address); a LAN client hitting the optional direct port is therefore *not* trusted by the same process. `any` is for tests and for relaxing the frame policy behind a private-network proxy; **it is refused (startup panic) in combination with `FUTUREFIN_TRUSTED_PROXY_AUTH=1`** — identity headers require an explicit IP list. |
| `FUTUREFIN_TRUSTED_PROXY_AUTH` | `false` | `parse_bool_env` (same quirk as `COOKIE_SECURE`: only `1/true/TRUE/yes/YES`). **Set without `FUTUREFIN_TRUSTED_PROXY_IPS`, or with it as `any`, → panic at startup** | prod, **optional** (new 2026-08-27) | Enables `POST /v1/auth/sso` (identity delegated to the proxy via `X-Remote-User-Id`, optional `X-Remote-User-Display-Name` / `X-Remote-User-Name`). The route is **always mounted** — only the state changes: off ⇒ 401 `sso_disabled`; on but untrusted peer ⇒ 401 `sso_untrusted_peer`; missing/non-UUID header ⇒ 400 `sso_bad_identity`. The startup panic is deliberate: "auth on, nobody trusted" is not a half-configuration, it is a config that *reads* as enabled while being dead. Accounts created this way have `password_hash IS NULL` and are rejected by password login with `sso_account_no_password`. |

| `FUTUREFIN_HA_SSO_URL` | unset → «Entrar con Home Assistant» off | must parse as a URL, scheme `http`/`https`, host present, **bare origin** (no path, query or fragment); normalized with `Url::origin().ascii_serialization()`; blank/whitespace = unset. Present-but-invalid → **panic at startup** — the same fail-loud shape as `FUTUREFIN_PUBLIC_URL` | prod, **optional**, **add-on only** (new 4.3.1) | Public origin of the user's Home Assistant (`https://ha.example.org`, `http://homeassistant.local:8123`) — what the *browser* types, not an internal hostname. Parsed by `main.rs::ha_sso_url()` into `AppState.ha_sso` together with the real client (`ha_idp::client::HttpHaIdp`). Everything hangs off it: `{base}/auth/authorize`, `/auth/token`, `/auth/revoke` and `ws(s)://…/api/websocket` (identity is only readable over WebSocket — HA has no REST `auth/current_user`). **Set ⇒ the feature exists**: `ha_idp::ha_login_available` is `state.ha_sso.is_some()` and nothing else — no peer, no header — because the button's whole point is the origin where there is *no* trusted proxy. Drives `window.__FF_HA_LOGIN__` in the shell. Echoed in the startup `"server config"` line as `ha_sso_url=…` / `(disabled)`. |
| `FUTUREFIN_HA_ADDON` | `false` | `parse_bool_env` (only `1/true/TRUE/yes/YES`) | **internal to the add-on** (new 4.3.1) | Exported by the entrypoint whenever `/data/options.json` exists; never set it by hand. Its only job today is to authorize the row above: **`FUTUREFIN_HA_SSO_URL` set without `FUTUREFIN_HA_ADDON=1` panics at startup** instead of being silently ignored. Deliberate (D19, owner-confirmed): a compose install that configured the URL would believe it had a login that cannot work — HA accepts a `client_id` equal to *this* app's origin, and it only accepts it when both share an origin through HA's own ingress. The reverse (flag without URL) is the normal add-on state with the option left empty. |

Not env-configurable (hardcoded constants — changing them is a code change):
- DB pool: `max_connections=10, min=1, acquire_timeout=5s, idle_timeout=600s, max_lifetime=1800s` (`apps/api/src/db.rs`).
- Projection cache TTL: 60 min sliding (`PROJECTION_CACHE_TTL`, `apps/api/src/state.rs`).
- Body limits: 1 MiB default (via extractors), 16 MiB for backup import, and (since 4.4.0) 1 MiB explicit on `/mcp` — `DefaultBodyLimit` never reached that route (`apps/api/src/routes/mod.rs`, `apps/api/src/mcp/mod.rs`, see §4).
- Gzip compression for responses >1 KB (`main.rs`, `CompressionLayer`).

### 1.2 Container entrypoint (parsed in `apps/api/docker-entrypoint.sh`) — new in 3.0.0

The entrypoint is PID 1 in the image: it initializes/adopts/upgrades the cluster, takes the
automatic pre-migration backup, launches the postmaster and the API, and shuts both down in order.
Since 4.0.0 it no longer *chooses* a database — the embedded one is the only one — but it still
inspects `DATABASE_URL` to catch leftovers from 3.x (see `FUTUREFIN_DB_MODE` below). **None of these variables is required** — the defaults below
are exactly what production runs with an empty `.env`.

| Variable | Default | Values / bounds | Notes |
|---|---|---|---|
| `FUTUREFIN_DB_MODE` | `auto` | `auto` \| `embedded` (**synonyms since 4.0.0**); `external` **aborts** with a migration message; anything else aborts with `invalid FUTUREFIN_DB_MODE` | Both live values mean "the embedded cluster". `external` is still *recognized* only so a 3.x compose gets a useful abort instead of a cryptic one. What still varies is the handling of a leftover `DATABASE_URL` pointing outside `/var/run/postgresql`: **cluster present** → warn + ignore ("quítala de tu compose"); **no cluster** → `refuse_external_database` prints the boxed migration instructions (start 3.9.0 once with that same URL and volume, drop `DATABASE_URL`, come back) and exits 1 **before touching anything** — the volume is left untouched, which CI asserts. Note this check runs *before* the no-volume guard, so the watchtower-over-2.x-compose case shows the external message, not "no persistent volume". |
| `FUTUREFIN_MODE` | `serve` (or `argv[1]`) | `serve` \| `db-only` | `db-only` = rescue mode: PostgreSQL up, API **not** started, restore instructions printed. Any other `argv` is `exec`'d verbatim (`docker run … pg_dump --version`). |
| `FUTUREFIN_PREMIGRATION_BACKUP` | `on` | `on` = enabled; any other value disables it | Automatic `pg_dump` + gzip into `$FUTUREFIN_BACKUP_DIR` whenever the app version changed or migrations are pending. A **failing** backup aborts startup on purpose ("refusing to start with pending migrations and no safety net") — set it to `off` only to bypass deliberately. |
| `FUTUREFIN_BACKUP_KEEP` | `10` | integer | The newest N automatic backups are never pruned. |
| `FUTUREFIN_BACKUP_KEEP_DAYS` | `90` | integer (days) | Beyond the newest `KEEP`, files older than this are deleted. Plus an emergency prune when the volume drops under 256 MB free, which never goes below 3 files. |
| `FUTUREFIN_ALLOW_EPHEMERAL_DB` | `0` | `1` = allow | Guard against silent data loss: if `$PGDATA` is **not a real mountpoint** the container **aborts** ("no persistent volume is mounted"). `1` runs with a throwaway DB that dies with the container — never for real data. |
| `FUTUREFIN_API_STOP_TIMEOUT` | `15` | seconds | SIGTERM grace for the API before escalating to SIGKILL. |
| `FUTUREFIN_PG_STOP_TIMEOUT` | `30` | seconds | SIGINT (**fast** shutdown — never SIGTERM, which is *smart* and can hang) grace for the postmaster before SIGQUIT. Keep compose's `stop_grace_period: 60s` above `API_STOP_TIMEOUT + PG_STOP_TIMEOUT`. |
| `FUTUREFIN_STATE_DIR` | `/var/lib/futurefin` (Dockerfile `ENV`) | path | Volume `ffdata`: `state/` files, automatic backups, pg_upgrade staging. |
| `FUTUREFIN_BACKUP_DIR` | `$FUTUREFIN_STATE_DIR/backups` | path | Where `pre-migration-*` and `pre-pgupgrade-*` dumps land. (`pre-automigration-*` files only exist on volumes that migrated from an external DB under 3.x; 4.0.0 writes none but prunes them like the rest.) |
| `FUTUREFIN_PG_LISTEN` | empty = socket only | postgres `listen_addresses` | **Debug only.** Setting it opens TCP inside the container; production is socket-only by design. |
| `FUTUREFIN_PG_LOG_LEVEL` | unset | postgres `log_min_messages` (e.g. `debug1`) | Debug only. |
| `POSTGRES_USER` | `futurefin` | role name | Compat with 2.x: set it only if your 2.x install customized it, otherwise the adopted cluster's superuser won't match and startup dies with a clear message. |
| `POSTGRES_DB` | `futurefin` | database name | Same 2.x-compat rationale; created on first boot if missing. |
| `POSTGRES_PASSWORD` | unset | any string | **No longer required** (local socket, `trust`). If present it is only `ALTER ROLE … PASSWORD`-applied, for people who reach the role from outside. |

Dockerfile `ENV`s the entrypoint reads but you should not override: `PGDATA=/var/lib/postgresql/data`,
`PG_MAJOR=16`, `WEB_STATIC_ROOT=/app/web`, `PORT=8080`. The image also carries
`LABEL com.futurefin.postgres.majors="15,16"` (16 active, 15 bundled only to auto-`pg_upgrade`
older volumes) and a `HEALTHCHECK` on `/v1/ready`, and deliberately declares **no `VOLUME`** — the
mountpoint guard above depends on that.

### 1.2.1 Home Assistant add-on: `options.json` → env (same entrypoint, new 2026-08-27)

In the HA add-on the user never sees an env var: they fill a form, the Supervisor writes
`/data/options.json`, and `apps/api/docker-entrypoint.sh` translates it. **The presence of that file
IS the detection** (`HA_ADDON=1`) — there is no other reliable signal inside the container. The
option schema lives in `addon/futurefin/config.yaml` (`options:` + `schema:`); the labels the user
reads are in `addon/futurefin/translations/{en,es}.yaml`.

Two variables are overridden **unconditionally and before everything else**, because the Supervisor
mounts exactly one persistent bind (`/data`) and the Dockerfile `ENV`s point outside it — a
`${VAR:-default}` would never see HA's value and the database would die on every container recreate:

| Exported | Value under the add-on | Instead of |
|---|---|---|
| `PGDATA` | `/data/pgdata` | `/var/lib/postgresql/data` |
| `FUTUREFIN_STATE_DIR` | `/data/state` (so backups land in `/data/state/backups`) | `/var/lib/futurefin` |

Consequence for the volume guard: `$PGDATA` is now a *subdirectory* of the mountpoint, so the check
is `is_persisted` (walks ancestors, **stops before `/`** — in any container `/` is a mountpoint, so
accepting it would make the guard decorative) rather than a plain `mountpoint` on `$PGDATA`. The
compose case is unchanged: `/var/lib/postgresql/data` → `/var/lib/postgresql` → `/var/lib` → `/var`,
none mounted → still aborts.

Then the six options, read with `ha_opt` (`jq`, in the image since this change). Note `ha_opt`
checks presence explicitly instead of `.[$k] // empty`: jq's `//` treats `false` as empty, so a
boolean set to `false` would read as absent and the toggle would never apply.

| Option (`options.json`) | Default | Exports |
|---|---|---|
| `log_level` | `"info"` | `trace`/`debug` → `RUST_LOG=futurefin_api=debug,tower_http=debug,sqlx=warn`; `warn`/`error` → `…=warn,…=warn,sqlx=error`; `info`/`notice`/empty → **nothing** (any pre-existing `RUST_LOG` is respected). Schema: `list(trace\|debug\|info\|warn\|error)`. |
| `sso` | `true` | When `true`: `FUTUREFIN_TRUSTED_PROXY_AUTH=1`. The IP list `FUTUREFIN_TRUSTED_PROXY_IPS=${FUTUREFIN_TRUSTED_PROXY_IPS:-172.30.32.2}` is exported **unconditionally in add-on mode**, toggle or not — the ingress iframe needs a trusted peer for `handlers/frame.rs` to relax `X-Frame-Options: DENY` into `frame-ancestors 'self'`, or the panel renders **blank** even though the add-on works. Between the two, the §1.1 startup panic ("auth on, nobody trusted") is unreachable here. `172.30.32.2` is the Supervisor's ingress address on HA's internal network. |
| `mcp` | `true` | Only `false` does anything: `FUTUREFIN_MCP_ENABLED=0`. (MCP and OAuth are **not** reachable through the ingress; they need the optional direct port.) |
| `cors_origins` | `""` | Non-empty → `CORS_ORIGINS=<value>` verbatim (so §1.1's fail-loud parsing applies: a bad entry panics at startup). |
| `public_url` | `""` | Non-empty → `FUTUREFIN_PUBLIC_URL=<value>` verbatim (same fail-loud validation). Only needed for OAuth over a direct/tunnelled URL. |
| `ha_sso_url` (4.3.1) | `""` | Non-empty → `FUTUREFIN_HA_SSO_URL=<value>` verbatim (same fail-loud validation: a malformed value panics at startup). Turns on **«Entrar con Home Assistant»** for the login screen and the OAuth consent screen *outside* the panel. It is the **public** URL of the user's HA — the browser follows the redirect to it, and the add-on itself calls it to exchange the code and read the identity — so it needs **neither `hassio_api` nor `homeassistant_api`**. Empty (the default) ⇒ the button does not exist and everything is byte-identical to 4.3.0. |

Add-on mode also exports `FUTUREFIN_HA_ADDON=1` unconditionally (§1.1) — the signal that makes the
option above legal at all; outside the add-on the URL is a startup panic, not a no-op.

Not an option and not an env var: `FUTUREFIN_BASE_PATH` stays unset in the add-on — the ingress
sends `X-Ingress-Path` on every request, which outranks it (§1.1).

### 1.3 Compose / deployment level (substituted by `docker-compose*.yml`, never seen by the binary)

| Variable | Default | Prod or dev | Notes |
|---|---|---|---|
| `FUTUREFIN_IMAGE` | `maxlainz/futurefin` | prod | Set to `futurefin-local` for the local-image test flow (§3). |
| `FUTUREFIN_TAG` | `latest` | prod | Pin to `X.Y.Z` for stability; rollback = change tag + `up -d`. |
| `APP_PORT` | `8080` | prod | **Host** port mapped to the container's fixed internal `:8080`. This is the distinction: `APP_PORT` = host side of the mapping, `PORT` = what the binary listens on inside the container (always 8080 there). |
| `POSTGRES_USER` / `POSTGRES_DB` | `futurefin` / `futurefin` | prod + dev | Passed through to the container (§1.2) and, in `docker-compose.dev.yml`, to the dev Postgres and its `pg_isready` healthcheck. |
| `POSTGRES_PASSWORD` | dev compose defaults it to `futurefin`; prod compose does not pass it at all | dev (prod: optional) | **Changed in 3.0.0**: the old `${POSTGRES_PASSWORD:?Set POSTGRES_PASSWORD in .env}` guard is gone — production no longer needs it. It still matters in split-dev, where it must match the password inside your `DATABASE_URL`. |

### Dev-only (Vite, tests, scripts)

| Variable | Default | Consumed where | Notes |
|---|---|---|---|
| `FUTUREFIN_API_PORT` | `8081` | `apps/web/vite.config.ts` | Vite proxy target port for `/v1`, `/health`, `/openapi.json` and — since 3.1.0 — `/.well-known`, `/oauth/token`, `/oauth/register`, `/oauth/revoke`, `/mcp`. Read **without** `VITE_` prefix — the config uses `loadEnv(mode, repoRoot, "")`, i.e. all vars, from the **repo root** `.env` (not `apps/web/.env`). **Never add a bare `"/oauth"` proxy key**: keys are prefixes, so it would hijack `/oauth/authorize` — an SPA view, not a backend route — and dev would 404 instead of showing the consent screen. |
| `WEB_DEV_PORT` | `8080` | `apps/web/vite.config.ts` | Vite dev-server port. `strictPort: false` — if 8080 is busy Vite silently picks the next port; check the terminal banner. |
| `TEST_DATABASE_URL` | `postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test` | `apps/api/tests/common/mod.rs` | Postgres for integration tests (each test creates its own schema). **Desde 4.0.0 CI la define** (job `integration`, servicio `postgres:16.4-alpine` en `127.0.0.1:5432`; en local el puerto documentado sigue siendo 5433 para no chocar con el Postgres de desarrollo). Antes no existía en ningún job y la suite de integración no corría en CI — ver `.claude/skills/futurefin-validation-and-qa/SKILL.md` §3. |
| `BASE`, `SMOKE_USER`, `SMOKE_PASS` | `http://127.0.0.1:8080`, auto-registers throwaway user | `scripts/smoke-projection-cache.sh` | Owned by futurefin-diagnostics-and-tooling. |
| `ENV_FILE`, `BACKUP_DIR`, `KEEP_BACKUPS` | `.env.prod`, `./backups`, `30` | `scripts/backup-postgres.sh` | Owned by futurefin-run-and-operate. |

`.env.example` at the repo root is the canonical template: since 3.0.0 **every line in it is
commented out** — production runs with an empty `.env` or none at all. It documents the optional
prod knobs (`FUTUREFIN_TAG`, `APP_PORT`, `FUTUREFIN_IMAGE`, the two backup-retention vars, and since
3.1.0 `FUTUREFIN_MCP_ENABLED` + `FUTUREFIN_PUBLIC_URL`), the
2.x compat trio (`POSTGRES_USER`/`POSTGRES_DB`/`POSTGRES_PASSWORD`) and the dev block (`PORT=8081`,
`DATABASE_URL`, `RUST_LOG`). Since 4.0.0 the external-DB pair is gone from it, replaced by a note
saying external databases were retired and that a leftover `DATABASE_URL` makes the container stop
and explain how to migrate (via 3.9.0). The warning about not leaving the dev `DATABASE_URL` next
to the production compose stands, but keep it honest: **none of this repo's compose files pass
`DATABASE_URL` into the container** (no `env_file:`, no `DATABASE_URL:` entry), so it only reaches
the image through a compose that declares it (the 2.x one does) or `docker run -e`.

## 2. `.env` loading order and precedence

API side — `main.rs::load_env()` runs before anything else:
1. `dotenvy::from_filename({CARGO_MANIFEST_DIR}/../../.env)` — the **repo-root** `.env`, resolved
   at compile time relative to `apps/api/Cargo.toml`. This is why `cargo run` from `apps/api`
   still picks up the root `.env`.
2. `dotenvy::dotenv()` — `.env` in the current working directory (a fallback; from repo root this
   is the same file).

dotenvy never overwrites variables already set: **real environment > repo-root `.env` > CWD
`.env`**. If a change to `.env` "isn't taking effect", check for the variable exported in your
shell or injected by compose — that wins. Both loads are `let _ = ... .ok()`: a missing `.env` is
silent, so in Docker (no `.env` in the image) only real env vars apply — and since 3.0.0 the one
that matters most, `DATABASE_URL`, is `export`ed by the entrypoint immediately before launching
the binary, so inside the container it always wins.

Vite side — `apps/web/vite.config.ts` computes `repoRoot = apps/web/../..` and calls
`loadEnv(mode, repoRoot, "")`. The empty-string third argument disables the `VITE_` prefix filter,
so `FUTUREFIN_API_PORT` / `WEB_DEV_PORT` are plain names in the root `.env`. These are dev-server
settings only; they are not baked into the client bundle.

## 3. Docker Compose file matrix

Three files, but only **one** of them is an override now (3.0.0 replaced
`docker-compose.split-dev.yml` — it no longer exists — with the standalone `docker-compose.dev.yml`):

| File | Project name | Scenario | What it does |
|---|---|---|---|
| `docker-compose.yml` | `futurefin` | Production / normal run | **One** service, `futurefin`: the published image, host `${APP_PORT:-8080}` → container 8080, `restart: unless-stopped`, `stop_grace_period: 60s` (the embedded postmaster needs room to checkpoint; Watchtower ignores it — set `WATCHTOWER_TIMEOUT=60s`). Volumes `pgdata:/var/lib/postgresql/data` (**same name and path as 2.x**, so upgrading reuses the data as-is) and `ffdata:/var/lib/futurefin` (automatic backups + pg_upgrade staging). Environment: only `RUST_LOG`, `POSTGRES_USER`, `POSTGRES_DB` — `PORT`/`WEB_STATIC_ROOT`/`PGDATA` come from the Dockerfile `ENV` and **`DATABASE_URL` is deliberately absent**. Healthcheck: `["CMD-SHELL", "curl -fsS http://127.0.0.1:8080/v1/ready >/dev/null"]`, `interval 15s`, `timeout 5s`, `retries 5`, `start_period 120s` (first boot after a 2.x upgrade does chown + REINDEX + backup). CMD-SHELL is mandatory (v1.0.2 incident: the exec form doesn't resolve `curl` via PATH) and **no `</dev/tcp/…>` fallback** may be added — it would mask a 503 from `/v1/ready` and report healthy with the DB down. |
| `docker-compose.local.yml` | (inherits `futurefin`) | Test a locally built image without publishing | Unchanged: an override adding **`pull_policy: never`** to service `futurefin` — otherwise compose tries to pull `futurefin-local:dev` from Docker Hub and fails. Use with `FUTUREFIN_IMAGE=futurefin-local`, `FUTUREFIN_TAG=dev` in `.env`: `docker compose -f docker-compose.yml -f docker-compose.local.yml --env-file .env up -d`. Full recipe: CLAUDE.md "Test local con Docker Desktop". |
| `docker-compose.dev.yml` | `futurefin-dev` | split-dev (`cargo run` + `npm run dev:web`) | **Standalone, not an override** — the production file has no DB service left to override. Single service `db` (`postgres:16.4-alpine`, digest-pinned, container `futurefin-dev-db`) published on **`127.0.0.1:5432`**, volume `devdata`, `pg_isready` healthcheck, creds defaulting to `futurefin`/`futurefin`/`futurefin`. Usage: `docker compose -f docker-compose.dev.yml up -d` (no `-f docker-compose.yml`). Never in production. A comment inside explains how to keep your pre-3.0.0 dev data: replace the `devdata:` entry with `devdata: {external: true, name: futurefin_pgdata}`. |

Only `docker-compose.local.yml` is combined with the base file (`-f docker-compose.yml -f
docker-compose.local.yml`, base first). `docker-compose.dev.yml` is passed **alone** and lives in
its own compose project, so it never collides with a production stack on the same host — but note
the two projects would both want host port 5432/8080 respectively, and the dev volume is
`futurefin-dev_devdata`, *not* `futurefin_pgdata`.

The postgres image digest now appears in exactly two places: `docker-compose.dev.yml` (dev DB) and
`apps/api/Dockerfile` (the pg15/pg16 COPY source stages). It is **no longer** in
`docker-compose.yml`.

## 4. API query-parameter flags and body limits

### `?view=household|mine` — ledger scope (all ledger endpoints)

Defined in `apps/api/src/handlers/person_view.rs` (`LedgerViewQuery::resolve`). The value is
trimmed; exactly `mine` → `LedgerView::Mine` (adds `AND owner_user_id = <session user>`); **any
other value, including typos, silently means `household`** (full installation). Accepted by
assets, liabilities, summary, budget, planning, allocation-rules and projection handlers.
Non-negotiable semantics: this is a client-side display filter, **not** an authorization
boundary — every member sees household data. Handlers must build the WHERE via
`LedgerView::scope_where(alias)` + `bind_scope_as/scalar` (placeholder indices start at
`next_arg_index()`: 2 for household, 3 for mine); never hand-write the two branches.

### `GET /v1/projection/series?months=&density=&view=` (`apps/api/src/handlers/projection.rs`)

| Param | Values | Default | Semantics |
|---|---|---|---|
| `months` | u32, **must be 12–840 or the request is rejected** — since 4.4.0 out-of-range is **400 `months_out_of_range`**, NOT a silent clamp (`validate_months_override`, `handlers/projection.rs`) | omitted | Horizon override. Omitted → horizon derived from demographics: years until age 90 from ONE resolved birth date (session user's `users.birth_date`, else the first `persons` row by `is_primary DESC, sort_index ASC` — NOT the oldest member), clamped 5–70 years; no birth date at all → 30 years. `horizon_basis` in the response reports which path: `lifespan_90`, `fallback_no_demographics`, or `months_override`. (Implementation: `projection_horizon_months()`, `handlers/projection.rs` ~599–627.) |
| `density` | `monthly` \| `hybrid` (trimmed; anything else → `monthly`) | `monthly` | Serialization-only decimation: `monthly` ≈ one point per month (~841 at max horizon); `hybrid` = months 0..12 monthly + 24, 36, … annually (~82 points, ~5× smaller JSON). The engine always computes the **full** series; milestones/crossover indices are computed pre-decimation, so a `reached_month_index` may not exist as a point in a hybrid response — match by `month_index`, never by array position (the v1.4.2 chart bug). |
| `view` | as above | `household` | Also selects the cache partition. |

Cache-key implications (`apps/api/src/state.rs`): the in-memory projection cache is keyed by
`(installation_id, view, owner_user_id [Some only for mine], density)` with a 60-min sliding TTL.
**Any `?months=` override bypasses the cache entirely** (computed fresh, never stored). Adding a
new query param that changes response content requires either joining `ProjectionCacheKey` or
bypassing the cache — otherwise users get stale cross-contaminated responses. Every mutating
handler invalidates the whole installation's entries (`refresh_projection_after_mutation`).

### `GET /v1/history/*` — snapshot query params (`apps/api/src/handlers/history.rs`, v1.5.0)

| Endpoint | Param | Values | Default | Semantics |
|---|---|---|---|---|
| `GET /v1/history/snapshots` | `year` | `i32`, validated **1900–3000** (out of range → 400) | omitted → all years | Filters by a civil-date range (index-friendly), always own-user (no `?view`). |
| `GET /v1/history/snapshots` | `kind` | `asset` \| `liability` (anything else → 400 `invalid_kind`) | omitted → both | Note: **stricter than `?view`/`?density`** — an unknown `kind` here **errors 400**, it does not silently fall back. |
| `GET /v1/history/series` | `view` | `household` \| `mine` (standard `LedgerViewQuery::resolve`) | `household` | Standard scope filter (§4.1). `mine` = own series; `household` = server-side sum of every user's interpolated series. |
| `GET /v1/history/series` | `window_months` (default changed Fase 5/issue #86, 4.4.0) | `i64`, **1..=1200** (out of range → 400 `window_months_out_of_range`, shared `validate_window_months` helper — NOT a clamp) | omitted → `DEFAULT_HISTORY_WINDOW_MONTHS` = `120` (10 years) | Before 4.4.0, omitting it returned the **entire** history back to the first snapshot; a client reading a short array from a fresh install could not tell "little data" from "truncated". Now the default is bounded and the response says so: `window_months` (echo), `window_truncated` (more history exists beyond the emitted window), `first_snapshot_date_ymd`/`first_snapshot_month_index`. Ask for `window_months=1200` (`MAX_HISTORY_WINDOW_MONTHS`) to get everything — nothing can be older, since that value is also the cap. |
| `GET /v1/history/snapshots/prefill` (v1.5.1) | `kind` | `asset` \| `liability` (anything else → 400 `invalid_kind`) | **required** | Which ledger side to pre-populate the backfill modal with. Always own-user (no `?view`). |
| `GET /v1/history/snapshots/prefill` (v1.5.1) | `date` | civil date `YYYY-MM-DD`; a future date → 400 `snapshot_date_in_future` | **required** | Target date the suggested values are interpolated to (same math as `/v1/history/series`). Each item returns a `value` + `basis` ∈ `interpolated`\|`first_snapshot`\|`live`\|`not_owned`; items that didn't exist yet arrive `value:"0"`, `existed:false`. |
| `GET /v1/history/cashflow` (v1.6.0) | `view` | standard `LedgerViewQuery::resolve` | `household` | Standard scope filter (§4.1) over transactions + snapshots. |
| `GET /v1/history/cashflow` (v1.6.0) | `window_months` | `i64`, **1..=120** (out of range → 400 `window_months_out_of_range`, same shared `validate_window_months` as `/v1/history/series` — **not** a silent clamp, corrected here) | `24` (`DEFAULT_CASHFLOW_WINDOW_MONTHS`) | Months of monthly aggregate + fine-grid window. **Since Fase 5/issue #86 (4.4.0)** the **fine** curve carries an additional, separate cap: `MAX_FINE_CURVE_WINDOW_MONTHS = 36`. Above 36, `window_months` itself is still accepted (up to 120) and `months[]` (the monthly aggregate) arrives in full — only `fine` comes back `null`, with `fine_absent_reason` ∈ `not_requested`\|`window_too_large_for_curve`\|`no_asset_linked_transactions`\|`no_snapshots_to_anchor` naming why. Going over 36 is **not** a 400. |
| `GET /v1/history/cashflow` (v1.6.0) | `resolution` | `weekly` \| `daily` (trimmed; anything else → `weekly`) | `weekly` | `daily` **requires `window_months <= 6`** → else **400 `daily_window_too_large`** (grid cost). `daily` runs in `spawn_blocking`; `weekly` inline. |

No new **env vars** and no new installation settings ship with the history feature (series,
prefill or cashflow) — it is entirely per-user request/data surface. The series and prefill
endpoints have **no cache** (sub-ms compute) and take no `?months`/`?density`; cashflow is also
uncached.

### MCP tool pagination (`limit`/`offset`) — `apps/api/src/mcp/server.rs`

Four `list_*` MCP tools accept `limit`/`offset` and paginate in SQL, echoing
`total_count`/`offset`/`truncated` in the envelope — never a bare array for these (the
"suppression declares itself" pattern, `futurefin-architecture-contract` D22). All
four share the pattern: `limit` omitted → the tool's default; `limit == 0` or `> max` → 400
`limit_out_of_range`.

| Tool | Default `limit` | Max `limit` | Notes |
|---|---|---|---|
| `list_transactions` | `LIST_TRANSACTIONS_DEFAULT_LIMIT` = 100 | `LIST_TRANSACTIONS_MAX_LIMIT` = 500 | Oldest of the four (pre-Fase-5). |
| `list_categorization_rules` | `LIST_RULES_DEFAULT_LIMIT` = 50 | `LIST_RULES_MAX_LIMIT` = 200 | Paginating since **4.0.0** (commit `70417dc`, `git log -S LIST_RULES_DEFAULT_LIMIT`), not 3.8.0. |
| `list_snapshots` (Fase 5, issue #86) | `LIST_SNAPSHOTS_DEFAULT_LIMIT` = 50 | `LIST_SNAPSHOTS_MAX_LIMIT` = 200 | New. `list_snapshots_core` gained `include_items`/`limit`/`offset`, returns `(page, total_count)`. Order `snapshot_date DESC, kind ASC, id ASC` — the `id` tiebreak is new: without a total order, two consecutive pages could repeat or skip rows. |
| `list_transaction_imports` (Fase 5, issue #86) | `LIST_IMPORTS_DEFAULT_LIMIT` = 50 | `LIST_IMPORTS_MAX_LIMIT` = 200 | New. `list_imports_core` renamed to `list_imports_page` with the same widened signature. |

**The HTTP path is unchanged for the two new ones**: `GET /v1/history/snapshots` and
`GET /v1/transactions/imports` call the same widened core with `limit = None`, which still skips
`LIMIT`/`OFFSET` and the `COUNT` query — same pattern `list_transactions_query` already used for the
HTTP/MCP split. Pagination here is an **MCP-tool-only** axis; it does not add HTTP query params.

### `GET /v1/transactions/*` — histórico de gasto query params (`apps/api/src/handlers/transactions/`, v1.6.0)

Most read endpoints accept `?view` (standard §4.1 scope: `GET /v1/transactions`, `/months`,
`/summary`, `/imports`); the **rules** GET is always own-user (no `?view`), and all writes are
`owner_user_id = session user`. Additional filters, all optional unless noted:

| Endpoint | Param | Values | Default | Semantics |
|---|---|---|---|---|
| `GET /v1/transactions` | `month` | `YYYY-MM` (invalid → 400) | omitted → all | Filters `op_date` to that calendar month. Plus `kind` (`expense`\|`income`\|`savings`, invalid → 400), `category_id` (uuid), `import_id` (uuid). |
| `GET /v1/transactions/summary` | `year` + `month` | `year` 1900–3000, `month` 1–12; **provided together or neither** (else 400) | omitted → last **complete** calendar month | Selected month of the comparison. |
| `GET /v1/transactions/summary` | `avg_window` | `3` \| `6` \| `12` \| `ytd` \| `all` (trim + case-insensitive; anything else → **400 `avg_window must be one of 3, 6, 12, ytd, all`**) | `6` | Historical-average window (v1.8.0). Weighted average: denominator = months in the window with ≥1 transaction, not the window width. `ytd` = calendar months of the selected year strictly before the selected month (Jan → empty); `all` = since the first transaction. |
| `GET /v1/transactions/summary` | `avg_months` | u32, **1–24** (out of range → 400) | `6` | **Legacy alias** for `avg_window` (fixed-month window only). `avg_window` wins when both are sent. |
| `DELETE /v1/transactions/imports/{id}` | `confirm` | `bool` | `false` | Must be `true` or **400 `confirm_required`** (undo cascades to the batch's transactions). |

None of these query params (nor any transactions mutation) touch the projection cache — transactions
are not engine inputs (regression `transactions_projection_cache.rs`).

### Body limits (`apps/api/src/routes/mod.rs`, `apps/api/src/mcp/mod.rs`)

- Default: `DEFAULT_BODY_LIMIT_BYTES` = 1 MiB, via `DefaultBodyLimit` on the outer router. This
  was documented as a "1 MiB global" invariant — it is not: `DefaultBodyLimit` acts **through
  axum's extractors**, so any route that reads its body a different way is not covered (see the
  `/mcp` row below, issue #85 finding 6).
- `POST /v1/backup/user-import` and `/v1/backup/user-import/preview`, plus (v1.6.0)
  `POST /v1/transactions/import/preview` and `/v1/transactions/import/confirm`:
  `BACKUP_IMPORT_BODY_LIMIT_BYTES` = 16 MiB (base64 `.ffbackup`/CSV payloads inflate ~33%).
- **`/mcp`**: `mcp::MCP_MAX_REQUEST_BODY_BYTES` = 1 MiB, **set explicitly since 4.4.0**. `/mcp` is
  mounted with `route_service`, not a normal handler — the rmcp Streamable HTTP service reads the
  request body itself, with its own SDK default of **4 MiB**, so `DefaultBodyLimit` never reached
  it: the "1 MiB global" claim was false for the one route in the binary that doesn't go through
  an extractor. Fixed via `StreamableHttpServerConfig::with_max_request_body_bytes`. Regression:
  `body_limits.rs::oversized_mcp_body_returns_413` (a 2 MiB body — above the true global limit,
  below the SDK's old default: exactly the gap a request used to slip through).
- Symptom of hitting the limit: HTTP 413 on an otherwise valid request.

### 4.1 Ejes que no son env, ni query param, ni ajuste de instalación (Fase 3, issue #84)

Dos axes que este catálogo no tenía y que decidían comportamiento igual que los demás. No caben en
§1 (no son env), ni en §5 (no viven en la fila `installation`): uno es **por credencial** y el otro
**por petición**.

| Eje | Dónde vive | Valores / cotas | Quién lo fija | Efecto |
|---|---|---|---|---|
| `api_tokens.scope` | columna de `api_tokens` (migración `20260828140100_api_tokens_scope.sql`), `handlers/api_tokens.rs` | `read_write` (default de la columna, reproduce el comportamiento anterior al scope) \| `read_only`; `CHECK api_tokens_scope_valid` en la BD; un literal desconocido en el body → **400 `token_scope_invalid`** (validado a mano, no por serde, para no devolver el 422 genérico) | la persona, al crear el token (`POST /v1/api-tokens {scope}`); **no editable** después — se revoca y se emite otro | Segunda de las tres puertas de `require_mcp_write` (rol vivo → scope → `mcp_write_enabled`). **Solo RESTA**: un `read_write` sobre un `viewer` sigue sin escribir. `TokenScope::from_db` falla **cerrado** (un valor desconocido en la columna se trata como `read_only` y deja un `warn!`). Los `ffo_…` de OAuth no negocian scope: entran fijos como `ReadWrite` |
| `idempotency_key` | cabecera/campo por petición de escritura; `handlers/transactions/idempotency.rs`, tabla `transaction_idempotency_keys` | 1–200 caracteres sueltos (`MAX_KEY_CHARS`); **180** en la clave de lote (`MAX_BATCH_KEY_CHARS`, porque el sufijo por ítem ocupa 5 con `MAX_BATCH = 1000`); retención **24 h** (`RETENTION`) | el cliente MCP en cada llamada de escritura que la soporte | Un reintento con la misma clave devuelve el resultado original en vez de duplicar la fila. Es lo que hace seguro `create_batch` (un lote reintentado sin idempotencia es peor que N llamadas sueltas) |

Comprobación: `grep -n 'const MAX_KEY_CHARS\|const MAX_BATCH_KEY_CHARS\|const RETENTION' apps/api/src/handlers/transactions/idempotency.rs` y `grep -n 'scope' apps/api/migrations/*api_tokens_scope*.sql`.

## 5. Per-installation runtime settings (`apps/api/src/handlers/installation.rs`)

Stored on the singleton `installation` row; read back in every `InstallationSnapshot`
(`GET /v1/installation`, `GET /v1/installation/session-context`). Amounts/percentages travel as
**strings** on the wire (Decimal-as-string; never floats).

`PATCH /v1/installation` — **owner role only** (403 otherwise); at least one field required (400
otherwise); a successful PATCH **invalidates** the projection cache (like every mutation — it does
NOT warm it; warm-up happens only after login, see futurefin-architecture-contract D7). Frontend surface:
`apps/web/src/views/SettingsView.tsx` (Ajustes) and `RetirementView.tsx` (FIRE settings).

| Setting | Set where | Validation | Default | Meaning |
|---|---|---|---|---|
| `base_currency` | **setup only** (`POST /v1/installation/setup`); not in the PATCH body → immutable afterwards without a migration | trimmed, exactly 3 ASCII letters, uppercased; MVP whitelist `EUR`/`USD`/`GBP` | `EUR` (auto-bootstrap path) | Display currency. |
| `calendar_tz` | setup + PATCH | trimmed, length 3–64, must parse as an IANA zone via `chrono_tz` (e.g. `Europe/Madrid`, `UTC`); DB CHECK mirrors the length/trim rules | `UTC` (serde default at setup; DB column `DEFAULT 'UTC'`) | Civil "today" for the whole installation (projection anchor month, liability expiry filtering, derive-principal). |
| `show_age_mode` | setup + PATCH | `dates` \| `ages` | `dates` | Whether the projection X axis shows calendar dates or the viewer's age. |
| `annual_inflation_assumption_percent` | PATCH only | sent as a **string** (`"2.5"`); empty string → `0`; must parse as decimal, bounds **0–50** (negative rejected) | `0` (column `NOT NULL DEFAULT 0`) | Annual % applied to the **moving FIRE target only** — `target(month_index) = base × (1+pct/100)^(month_index/12)` (`fire_target_at_month_index`; the engine evaluates month k against the target at index k−1 — see futurefin-fire-domain-reference §4); incomes/expenses/contributions stay nominal. `0` = flat target. Semantics owned by futurefin-fire-domain-reference. |
| `fire_settings` | PATCH only | JSONB, shape below | column nullable; `NULL` → defaults applied on read (`resolve_fire_settings`) | FIRE target computation config. |
| `mcp_write_enabled` | PATCH only | bool | `TRUE` (column `NOT NULL DEFAULT TRUE`, migración `20260818120000`) | Kill-switch **vivo** de las tools de escritura del servidor MCP (issue #3): `require_mcp_write` lo lee de la DB en cada llamada de escritura → apagarlo corta la escritura en el siguiente request sin reiniciar (`FUTUREFIN_MCP_ENABLED` sigue siendo el kill-switch de `/mcp` entero, en el entorno; este es un **DB setting**, no una env var — deliberado: tiene toggle en la GUI, Ajustes → Integraciones). Las lecturas MCP no lo consultan. |

### `fire_settings` JSONB shape (as of 2026-07-09; `savings_source` added)

```json
{
  "fire_number_mode": "annual_expense",          // "manual" | "annual_expense" | "current_income"
  "fire_number_manual_amount": null,             // decimal string; REQUIRED and > 0 when mode = "manual"
  "swr_pct": "3.5",                              // decimal string, 0–4 (PERCENT, not ratio)
  "taxes_enabled": true,
  "tax_brackets": [                              // capital-gains schedule used for gross-up
    { "up_to": "6000",   "pct": "19" },
    { "up_to": "50000",  "pct": "21" },
    { "up_to": "200000", "pct": "23" },
    { "up_to": "300000", "pct": "27" },
    { "up_to": null,     "pct": "30" }           // last bracket MUST be open-ended (up_to null)
  ],
  "savings_source": "budget"                     // "budget" (default) | "transactions_avg" | "budget_income_real_expense"
}
```

Validation (`validate_fire_settings` / `validate_tax_brackets`, all 400 on failure):
- `swr_pct` ∈ [0, 4].
- mode `manual` ⇒ `fire_number_manual_amount` present and > 0.
- When `taxes_enabled`: `tax_brackets` non-empty; each `pct` ∈ [0, 99]; only the **last** bracket
  may (and must) have `up_to: null`; non-last `up_to` values must be > 0 and strictly increasing.
- Brackets are **not validated when `taxes_enabled` is false** — stale brackets can sit dormant.

Consumers beyond the FIRE target (v2.3.0 widened them):
- **`swr_pct`** feeds the Jubilación target *and* `GET /v1/summary` → `financial_health.runway_is_indefinite`:
  the runway is indefinite ⟺ the grossed-up annual expense fits inside `swr_pct` × liquid balance
  (`.claude/engine.md` §Runway). `swr_pct = 0` is a valid setting that makes the flag unreachable.
- **`tax_brackets` / `taxes_enabled`** likewise reach the runway through the *same* gross-up
  (`gross_up_net_annual_fire`, `pub(crate)` in `handlers/projection.rs`), so editing brackets moves the
  FIRE target and the runway threshold together. Dormant brackets (`taxes_enabled = false`) affect
  neither.

Deserialization details that matter:
- `fire_number_mode` is **strict**: unknown strings → 422. Sole legacy alias
  `annual_expense_adjusted` (old backup schemas) maps to `annual_expense`.
- **`savings_source`** (`SavingsSource` enum, `installation.rs`, `rename_all = "snake_case"`, `Default = Budget`) — source of the simulation's monthly saving, **three modes**:
  - `budget` (mode A, default — from budget entries, historical behavior).
  - `transactions_avg` (mode B — income and expense from the weighted average of the last 12 complete calendar months of transactions, used **raw** since the 3.4.0 reform: paid cuotas count as ordinary spending; liabilities only subtract their pending principal from net worth, constant across the horizon).
  - `budget_income_real_expense` (mode C — income from the **budget** + **real** expense, same raw average as B; FIRE target `annual_expense` uses the real expense, `current_income` uses budget income).

  The 12m average that feeds the engine (`transactions_avg`) counts only **real months** (≥1 transaction with `recurring_rule_id IS NULL`); pseudo-empty / recurring-only months are excluded entirely. Strict deserialize like `FireNumberMode`: unknown value → **422** (error lists all three valid variants); absent → `budget` (via the struct-level `#[serde(default)]`; old backups load). No extra `validate_fire_settings` bound — any of the three enum values is accepted. **What it affects** (modes B and C, gated by `SavingsSource::uses_transactions()`): `GET /v1/projection/*` (engine income/expense + FIRE target base), `GET /v1/summary` `financial_health` (income/expense/net/savings_rate + fields `savings_source`, `savings_income_basis`/`savings_expense_basis`; in mode C income stays the budget income; since the 3.4.0 reform also `expense_derived`/`expense_total` — derived = 0, total = the raw `expense_avg` — and therefore `runway_months`), `GET /v1/assets` (`contribution_nominal_monthly` **and**, since v2.2.0, the `months_expense`/`income_multiple` caps behind `contribution_target_amount`), `GET /v1/projection/series` (echoes the effective mode in `savings_source` + `savings_income_basis`/`savings_expense_basis`), and — crucially — the **projection-cache invalidation contract**: in B/C transaction mutations invalidate the cache (`invalidate_projection_if_savings_uses_transactions`), in mode A they never do (D12/D12a in futurefin-architecture-contract). Read without a round-trip via `projection_savings_source(pool, iid)`. FIRE-math meaning owned by futurefin-fire-domain-reference.
- The struct has `#[serde(default)]`: omitted fields fill with defaults
  (mode `annual_expense`, `swr_pct` 3.5, `taxes_enabled` true, Spanish IRPF brackets above).
- In the PATCH body `fire_settings` is `Option<Option<FireSettings>>`: **omit** = unchanged,
  JSON **`null`** = clear stored JSON (defaults apply on read), **object** = validate + replace
  wholesale (no deep merge — send the full object).

Note: `installation.projection_target_age` no longer exists — dropped by migration
`20260516120000_drop_projection_target_age.sql` (v1.0.6). The FIRE crossover is the sole
retirement trigger; do not reintroduce an age setting.

## 6. How to add a new configuration axis

First decide the layer: **env var** = per-deployment, operator-set, needs restart;
**installation setting** = per-household runtime data, owner-set via UI, persisted in DB;
**query param** = per-request presentation/scoping only. Behavior changes ride
futurefin-change-control gates regardless of layer.

### New env var — checklist

**Step 0 (new in 3.0.0): decide the consumer, and say so in the docs.** A variable is parsed in
exactly one of three places, and the reader must be told which: the **Rust binary**
(`apps/api/src/main.rs`, needs a restart of the API), the **container entrypoint**
(`apps/api/docker-entrypoint.sh`, only exists in the Docker image, affects DB lifecycle/backups
before the API ever starts), or **compose substitution** (`docker-compose*.yml`, resolved on the
host before the container exists — so it is *not* visible inside the container unless also passed
through `environment:`). Anything touching cluster init, adoption, pg_upgrade, backups or process
supervision belongs to the entrypoint; anything the handlers read at request time belongs to the
binary.

1. **Binary consumer**: `apps/api/src/main.rs` — parse next to the existing helpers
   (`parse_bool_env`, `port()`, `FUTUREFIN_DB_CONNECT_TIMEOUT_SECS`). Follow house style: explicit
   default, bounds via `.filter(...)`, never panic except for truly required values (only
   `DATABASE_URL` and bad `CORS_ORIGINS` panic today).
   **Entrypoint consumer**: add it to the `── Configuración ──` block at the top of
   `apps/api/docker-entrypoint.sh` as `NAME="${FUTUREFIN_X:-default}"` — all of them are defaulted
   there in one place, and CI runs `shellcheck -S warning` over the script.
2. If handlers need it at request time: add a field to `AppState` (`apps/api/src/state.rs`) and
   thread it through `AppState::new(...)` in `main.rs`.
3. Log it: the binary's startup `tracing::info!(... , "server config")` line, or an entrypoint
   `log ...` line, so deployments are auditable.
4. `.env.example` — add it, **commented out** (production must keep working with an empty `.env`),
   with the default noted and in the right block (prod / 2.x compat / dev).
5. If production-relevant *and* it must reach the container: `docker-compose.yml` `environment:`
   block. Compose-only knobs (image, tag, host port) stay in the `${VAR:-default}` interpolations.
   Do **not** reintroduce a `${VAR:?…}` hard requirement — 3.0.0's contract is that production
   needs no variable at all.
6. Docs of record: `.claude/env-and-config.md` table + `README.md` "Environment variables" table,
   plus §1.1/§1.2/§1.3 here, stating **which file parses it**.
7. If integration tests need it: `apps/api/tests/common/mod.rs` follows the
   default-with-override pattern (`TEST_DATABASE_URL`).

### New installation setting — checklist
1. Migration `apps/api/migrations/YYYYMMDDHHMMSS_description.sql`: `ALTER TABLE installation ADD
   COLUMN ... NOT NULL DEFAULT ...` (a default keeps the existing singleton row valid). Never edit
   a shipped migration; data-losing migrations need explicit owner sign-off (change-control rule).
2. `apps/api/src/handlers/installation.rs`: add the field to `InstallationMemberRow`,
   `InstallationSnapshot`, `PatchInstallationBody` (as `Option<...>`, omit = unchanged); write a
   `validate_*` function with explicit bounds; extend the `UPDATE` in `patch_my_installation`, the
   "at least one field" guard, and **all three** `SELECT i.id, i.base_currency, ...` queries (they
   are duplicated in session-context / get / patch — miss one and reads return stale shape).
   Also update `setup_installation`'s hardcoded response snapshot if the field has a setup default.
3. New struct types → register in the schema list in `apps/api/src/openapi.rs`; utoipa path
   annotations pick up body changes automatically.
4. Frontend: `apps/web/src/api/types.ts` (snapshot + patch types) and the editing UI in
   `apps/web/src/views/SettingsView.tsx` (Ajustes) or `RetirementView.tsx` for FIRE-related knobs.
5. If projection math consumes it: thread through `build_installation_projection_input` and
   remember PATCH already invalidates the projection cache.
6. Integration test in `apps/api/tests/` covering the validation bounds (accept boundary, reject
   out-of-range) — see futurefin-validation-and-qa for the TestApp harness.
7. Docs of record: `.claude/data-model.md` (installation row + invariants) and
   `.claude/api-routes.md`; CHANGELOG entry per futurefin-docs-and-writing.

### New query param — checklist
Add the field to the handler's `#[derive(Deserialize)]` query struct with `#[serde(default)]`,
resolve with the trim-then-match pattern (unknown values fall back to the default, they don't
error — match existing `view`/`density` behavior), document it in the `#[utoipa::path]` `params`,
and if the endpoint is the cached projection route, extend `ProjectionCacheKey` in
`apps/api/src/state.rs` or bypass the cache (see §4). Update `.claude/api-routes.md`.

## Provenance and maintenance

Env/compose/entrypoint rows re-verified **2026-08-16 against v3.0.0**, the two OAuth-related
rows (`FUTUREFIN_PUBLIC_URL`, `FUTUREFIN_MCP_ENABLED`) **2026-08-17 against v3.1.0**, and the
`mcp_write_enabled` installation-setting row **2026-08-18** (issue #3; re-verify with
`grep -n "mcp_write_enabled" apps/api/src/handlers/installation.rs apps/api/src/mcp/auth.rs`).
**§1.2 and every `DATABASE_URL` claim re-verified 2026-08-22 against v4.0.0**, which RETIRED the
external-database mode (`exec_api_external`, `automigrate_*`, `FUTUREFIN_DB_MODE=external` and
`FUTUREFIN_EXTERNAL_WAIT_SECS` are gone from `apps/api/docker-entrypoint.sh`).
**§1.1 gained `FUTUREFIN_HA_SSO_URL` + `FUTUREFIN_HA_ADDON` and §1.2.1 the `ha_sso_url` option on
2026-08-27 for v4.3.1** (branch `feat/ha-idp-login`), read from `apps/api/src/main.rs`,
`apps/api/src/ha_idp/`, `apps/api/docker-entrypoint.sh` and `addon/futurefin/config.yaml`. The same
pass corrected the `sso` row: the entrypoint exports `FUTUREFIN_TRUSTED_PROXY_IPS` **always** in
add-on mode, not only when the toggle is on.
**§1.1's `FUTUREFIN_MCP_ENABLED`/`FUTUREFIN_PUBLIC_URL`/`CORS_ORIGINS` rows and §4's body-limit
table re-verified 2026-08-28 for v4.4.0** (branch `feat/mcp-fase-4-transporte`, issue #85): the
kill-switch stopped unmounting routes, the public-URL variable gained subpath support, the CORS
list started feeding two layers instead of one, and `/mcp` gained an explicit body cap — all read
from `apps/api/src/mcp/mod.rs`, `apps/api/src/routes/mod.rs`, `apps/api/src/main.rs` and
`apps/api/src/oauth/url.rs`.
The rest of the tables carry their own dates inline. Every row is re-verifiable — run these from the repo root when
auditing for drift (all confirmed working on 2026-08-28):

- **`FUTUREFIN_HA_SSO_URL` parsing + the four panics, and the add-on-only combination check**:
  `grep -n "FUTUREFIN_HA_SSO_URL" -A 14 apps/api/src/main.rs` and
  `grep -n "FUTUREFIN_HA_ADDON" -B 6 -A 6 apps/api/src/main.rs` (the second must show the
  `ha_sso_url.is_some() && !ha_addon` panic)
- Where it is consumed (state + the single availability predicate, peer-independent):
  `grep -n "ha_sso\|with_ha_idp" apps/api/src/state.rs` and
  `grep -n "fn ha_login_available" -A 4 apps/api/src/ha_idp/mod.rs`
- Add-on option → env, plus the two unconditional exports:
  `grep -n "ha_sso_url\|FUTUREFIN_HA_ADDON\|FUTUREFIN_TRUSTED_PROXY_IPS" apps/api/docker-entrypoint.sh`
  and `grep -n "ha_sso_url" addon/futurefin/config.yaml` (must appear in **both** `options:` and
  `schema:`, the latter as `str?`)

- Env parsing, defaults, bounds, load order: `grep -n "env::var\|unwrap_or\|contains(&d)\|load_env" apps/api/src/main.rs`
- DB connect budget + retry backoff: `grep -n "FUTUREFIN_DB_CONNECT_TIMEOUT_SECS" -A 6 apps/api/src/main.rs` and `grep -n "connect_with_retry" -A 20 apps/api/src/db.rs`
- **Entrypoint variables and their defaults (§1.2)**: `grep -n 'FUTUREFIN_[A-Z_]*:-\|FUTUREFIN_MODE\|FUTUREFIN_PG_LISTEN\|FUTUREFIN_PG_LOG_LEVEL' apps/api/docker-entrypoint.sh` (the whole config block is lines ~17–34)
- Entrypoint guards and abort messages (mountpoint guard, invalid/retired db_mode, leftover-`DATABASE_URL` warn + refusal): `grep -n 'no persistent volume\|invalid FUTUREFIN_DB_MODE\|ya no existe\|refuse_external_database\|se ignora' apps/api/docker-entrypoint.sh`
- Socket `DATABASE_URL` the entrypoint exports: `grep -n 'export DATABASE_URL' apps/api/docker-entrypoint.sh`
- **CORS, since 4.4.0 split into two layers (issue #85)** — the parsing moved out of `main.rs`
  into `routes/mod.rs`, and the MCP-specific layer lives in `mcp/mod.rs`: default origin list +
  panic: `grep -n "CORS_ORIGINS" -A 20 apps/api/src/routes/mod.rs`; API layer (credentialed):
  `grep -n "fn api_cors_layer" -A 12 apps/api/src/routes/mod.rs`; MCP layer (no credentials,
  full preflight headers): `grep -n "fn mcp_cors_layer" -A 25 apps/api/src/mcp/mod.rs`; the
  `route_layer`-not-`layer` trap and the `merge`-order trap are both explained inline right above
  those two functions
- MCP kill-switch (added 2026-08-16, v3.0.0; widened to OAuth 2026-08-17, v3.1.0; **stopped
  unmounting routes 4.4.0, issue #85**): `grep -n "FUTUREFIN_MCP_ENABLED" apps/api/src/main.rs`
  and `grep -n "mcp_enabled" apps/api/src/routes/mod.rs apps/api/src/state.rs
  apps/api/src/handlers/oauth_consent.rs` — the last hit must show
  `oauth_consent_router(mcp_enabled)` gating ONLY `/authorize-details` + `/authorize`, with
  `/connections` mounted unconditionally; the 404-not-405 behavior:
  `grep -n "fn mcp_disabled\|MCP_DISABLED_MESSAGE" -A 4 apps/api/src/mcp/mod.rs` and
  `cargo test -p futurefin-api --test mcp_http -- mcp_disabled_answers_json_even_with_the_spa_mounted`
- `FUTUREFIN_PUBLIC_URL` parsing, bounds and the five panics (subpath validation added 4.4.0):
  `grep -n "FUTUREFIN_PUBLIC_URL" -A 14 apps/api/src/main.rs`; where it is consumed:
  `grep -n "public_url\|state.public_url" apps/api/src/state.rs apps/api/src/oauth/url.rs`; the
  once-per-process warn when a proxy prefix arrives without the var:
  `grep -n "fn warn_missing_public_url_for_prefix" -A 15 apps/api/src/oauth/url.rs`
- Request-derived issuer (the default path) + strict host charset: `grep -n "x-forwarded-proto\|x-forwarded-host\|fn is_valid_host" -A 8 apps/api/src/oauth/url.rs`
- The 7 OAuth protocol routes gated by the kill-switch: `grep -n "route(" apps/api/src/oauth/mod.rs`
- Vite proxy keys (must list `/oauth/token|register|revoke` one by one and **no bare `/oauth`**): `grep -n "proxy\|/oauth\|well-known\|/mcp" apps/web/vite.config.ts`
- Pool constants: `grep -n "connections\|timeout\|lifetime" apps/api/src/db.rs`
- Cache TTL + key + Density docs: `grep -n "PROJECTION_CACHE_TTL\|pub enum Density\|ProjectionCacheKey" -A 6 apps/api/src/state.rs`
- Body limits: `grep -n "BODY_LIMIT" apps/api/src/routes/mod.rs`
- `?months` rejection (NOT a clamp since 4.4.0) + horizon: `grep -n "fn validate_months_override\|months_out_of_range\|LIFESPAN_AGE\|FALLBACK_YEARS\|lifespan_90" apps/api/src/handlers/projection.rs` — `clamp(12, 840)` as a live pattern now finds only the doc comment noting it was retired ("Hasta 4.3.1 … hacía `m.clamp(12, 840)`"), which is itself the tell that the clamp is gone
- **History `window_months` defaults/caps (Fase 5, issue #86, 4.4.0)**: `grep -n "DEFAULT_HISTORY_WINDOW_MONTHS\|MAX_HISTORY_WINDOW_MONTHS\|DEFAULT_CASHFLOW_WINDOW_MONTHS\|MAX_CASHFLOW_WINDOW_MONTHS\|MAX_FINE_CURVE_WINDOW_MONTHS" apps/api/src/handlers/history.rs` and the shared validator `grep -n "fn validate_window_months" -A 12 apps/api/src/handlers/mod.rs` (both endpoints reject out-of-range, neither clamps)
- **MCP list-tool pagination limits (Fase 5, issue #86, 4.4.0 for the two new ones)**: `grep -n "_DEFAULT_LIMIT\|_MAX_LIMIT" apps/api/src/mcp/server.rs`
- `?density` / hybrid indices: `grep -n "resolve_density\|density_month_indices" -A 10 apps/api/src/handlers/projection.rs`
- `?view` resolution: `grep -n "fn resolve" -A 5 apps/api/src/handlers/person_view.rs`
- Installation validation bounds: `grep -n "normalize_currency\|validate_show_age_mode\|validate_annual_inflation\|normalize_calendar_tz\|swr_pct\|from(99u32)" apps/api/src/handlers/installation.rs`
- fire_settings defaults + legacy alias: `grep -n "default_fire_settings\|annual_expense_adjusted" -A 8 apps/api/src/handlers/installation.rs`
- `savings_source` enum + reader + conditional cache gating: `grep -n "enum SavingsSource\|savings_source\|projection_savings_source" apps/api/src/handlers/installation.rs apps/api/src/handlers/transactions/mod.rs`
- Compose file matrix (should list exactly three, no `split-dev`): `ls docker-compose*.yml`
- Compose services, volumes, healthcheck, ports, project names: `grep -n 'name:\|image:\|test:\|start_period\|stop_grace_period\|pull_policy\|5432\|/var/lib' docker-compose*.yml`
- Compose interpolation defaults / absence of hard requirements: `grep -n ":-\|:?" docker-compose*.yml`
- Dockerfile env, label, healthcheck, stages: `grep -n "^ENV\|^LABEL\|^HEALTHCHECK\|^FROM\|^CMD\|^ENTRYPOINT" apps/api/Dockerfile`
- Vite env reading: `grep -n "loadEnv\|FUTUREFIN_API_PORT\|WEB_DEV_PORT\|strictPort" apps/web/vite.config.ts`
- Test DB default: `grep -n "TEST_DATABASE_URL" apps/api/tests/common/mod.rs`
- Version stamp: `grep -n "^version" apps/api/Cargo.toml`
- **Reverse-proxy trio (§1.1, added 2026-08-27, branch `feat/home-assistant-addon`)**:
  `grep -n "FUTUREFIN_BASE_PATH\|FUTUREFIN_TRUSTED_PROXY_IPS\|FUTUREFIN_TRUSTED_PROXY_AUTH" apps/api/src/main.rs`
  (the three parses + the `AUTH requires IPS` panic — `grep -n "TRUSTED_PROXY_AUTH=1 requires" apps/api/src/main.rs`
  must print something); bounds and precedence:
  `grep -n "fn normalize_prefix\|fn validate_base_path_env\|fn request_prefix\|enum PeerPolicy\|fn from_env_value" apps/api/src/prefix.rs`;
  behaviour pinned by `cargo test -p futurefin-api --lib prefix::` and
  `apps/api/tests/{base_path,frame_options,session_cookie_path,sso_login}.rs`
- **Add-on options → env (§1.2.1)**:
  `grep -n "options.json\|ha_opt\|HA_ADDON\|PGDATA=/data/pgdata\|FUTUREFIN_STATE_DIR=/data/state" apps/api/docker-entrypoint.sh`
  (the block is the first ~45 lines, before the Configuración section);
  `grep -n -A12 "^options:" addon/futurefin/config.yaml` (the five keys and their schema);
  `grep -n "is_persisted" -B10 apps/api/docker-entrypoint.sh` (ancestor walk that stops before `/`);
  `grep -n "jq" apps/api/Dockerfile` (the runtime dependency this mapping added)

(The previously stale docs on these topics — data-model.md's `projection_target_age`,
env-and-config.md's fake `DATABASE_URL` "default", and the `mac_*` `horizon_basis` doc comment in
`handlers/projection.rs` — were all fixed on 2026-07-02. The standing-errata record lives in
futurefin-docs-and-writing §7.)

**Drift check for 3.0.0**: any doc, script or skill that still says
`docker-compose.split-dev.yml`, `docker compose up -d futurefin-database`, "`POSTGRES_PASSWORD`
is required", or composes `DATABASE_URL` from `POSTGRES_*` is stale — none of those exist any
more. `grep -rn 'split-dev\|futurefin-database' --include='*.md' --include='*.sh' .` finds them
(legitimate survivors: CHANGELOG history, `README`/`run-and-operate` telling you `--remove-orphans`
retires the old container, and `.github/testdata/docker-compose.v2*.yml`, which recreate a real
2.x stack on purpose to test the upgrade path).

When you change anything cataloged here, update this file in the same change, plus the matching
doc of record (`.claude/env-and-config.md`, `.claude/data-model.md`, `README.md`).
