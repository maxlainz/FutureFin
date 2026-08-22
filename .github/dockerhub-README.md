# FutureFin

**Tus finanzas de hoy y las de dentro de treinta años, en el mismo sitio.** Apunta lo que tienes
y lo que debes, cuadra tu presupuesto, importa los movimientos de tu banco y mira cuándo dejarías
de depender de tu sueldo. Todo en tu servidor, sin cuentas de terceros y sin que tus datos salgan
de casa.

Aplicación autoalojada de finanzas del hogar y planificación **FIRE** (independencia financiera).
Interfaz en español.

- 📖 **Código, documentación e incidencias:** <https://github.com/maxlainz/FutureFin>
- 🧾 **Licencia:** AGPL-3.0-only

---

## Un solo contenedor

Desde la 3.0.0 **PostgreSQL va dentro de la imagen**. No hay que levantar una base de datos
aparte ni configurar ninguna variable: la imagen se basta sola.

Guarda esto como `docker-compose.yml`:

```yaml
name: futurefin

services:
  futurefin:
    image: maxlainz/futurefin:latest
    container_name: futurefin
    restart: unless-stopped
    # PostgreSQL vive dentro del contenedor: dale margen para cerrar bien.
    stop_grace_period: 60s
    ports:
      - "8080:8080"
    volumes:
      - pgdata:/var/lib/postgresql/data   # la base de datos
      - ffdata:/var/lib/futurefin         # backups automáticos
    healthcheck:
      test: ["CMD-SHELL", "curl -fsS http://127.0.0.1:8080/v1/ready >/dev/null"]
      interval: 15s
      timeout: 5s
      retries: 5
      start_period: 120s

volumes:
  pgdata:
  ffdata:
```

Y arranca:

```bash
docker compose up -d
```

Abre `http://localhost:8080` y **crea tu cuenta: la primera persona que se registra es la
propietaria del hogar**. Si alguien más se registra, verá una pantalla de espera hasta que le des
acceso desde `Ajustes → Usuarios`.

> ⚠️ Sin un volumen montado en `/var/lib/postgresql/data` el contenedor **se niega a arrancar**.
> Es deliberado: sin él, tus datos morirían con el contenedor.

## Etiquetas

| Etiqueta | Qué es |
|---|---|
| `latest` | Última versión estable |
| `X.Y.Z` | Una versión concreta (recomendado en producción) |
| `X.Y`, `X` | Última de esa serie menor / mayor |

Arquitecturas: `linux/amd64` y `linux/arm64` (sirve para un Raspberry Pi o un NAS ARM).

## Volúmenes

| Ruta | Contenido |
|---|---|
| `/var/lib/postgresql/data` | La base de datos. **Imprescindible.** |
| `/var/lib/futurefin` | Backups automáticos previos a cada migración |

## Actualizar

```bash
docker compose pull && docker compose up -d
```

Las migraciones se aplican solas al arrancar, después de escribir un backup automático. Si vienes
de la serie 2.x (dos contenedores), añade `--remove-orphans` para retirar el antiguo
`futurefin-database`; tus datos se reutilizan tal cual.

## Más documentación

[Instalación](https://github.com/maxlainz/FutureFin/blob/main/docs/instalacion.md) ·
[Actualizar](https://github.com/maxlainz/FutureFin/blob/main/docs/actualizar.md) ·
[Configuración](https://github.com/maxlainz/FutureFin/blob/main/docs/configuracion.md) ·
[Copias de seguridad](https://github.com/maxlainz/FutureFin/blob/main/docs/backups.md) ·
[Conectar Claude (MCP)](https://github.com/maxlainz/FutureFin/blob/main/docs/mcp.md)
