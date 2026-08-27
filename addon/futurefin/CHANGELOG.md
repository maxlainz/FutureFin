# Changelog del add-on

Las novedades de la aplicación están en el
[CHANGELOG del proyecto](https://github.com/maxlainz/FutureFin/blob/main/CHANGELOG.md).
Aquí solo se registra lo propio del empaquetado como add-on.

## 4.3.1

- Nueva opción `ha_sso_url`: habilita el botón «Entrar con Home Assistant» al
  abrir FutureFin **fuera del panel** (puerto directo o túnel). Vacía = apagado.
- Con ella, autorizar el conector MCP/OAuth ya no exige una segunda cuenta con
  contraseña. FutureFin no guarda ninguna credencial de Home Assistant.

## 4.3.0

- Primera versión de FutureFin como add-on de Home Assistant.
- Panel en la barra lateral vía ingress.
- Inicio de sesión con la identidad de Home Assistant (opción `sso`).
- Puerto directo opcional (deshabilitado por defecto) para MCP y OAuth, que no
  pueden funcionar a través del ingress.
