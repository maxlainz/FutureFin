# Comunicación — Sunset del cliente macOS FutureFin

## Mensajes clave (público / release notes)

1. **La aplicación FutureFin para macOS está deprecada.** No recibirá nuevas funciones; el producto oficial pasa a la **versión self-hosted** (Docker / web).
2. **No hay migración automática** desde datos, backups `.ffbackup` ni CSV generados por el cliente macOS hacia la nueva línea. Es una base **independiente**.
3. **No hay soporte de compatibilidad** entre el binario macOS obsoleto y el servidor nuevo.
4. Los usuarios que deseen usar la nueva línea deben **configurar una instancia nueva** y **introducir datos manualmente** o mediante los **nuevos** formatos de export/import publicados en la documentación del servidor (cuando estén disponibles).

## Mensajes internos (equipo)

- El código Swift sigue disponible como **especificación ejecutable** y conjunto de **tests oráculo** hasta que el stack nuevo alcance paridad verificada.
- Priorizar comunicación temprana en cualquier canal donde se distribuyera el `.app`.

## Checklist de comunicación

- Aviso destacado en README del repo Swift (deprecation banner + enlace al repo nuevo).
- Nota en última release del `.app` si se publica binario final.
- Documentación de usuario única en el repo nuevo (sin referencias cruzadas de import desde Mac).

## Fechas

- **Por definir:** fecha oficial de fin de soporte y archivo del repo macOS (rellenar cuando se decida).