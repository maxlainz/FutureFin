# Publicar el repositorio en GitHub

En este entorno no hay acceso de red para crear el remoto automáticamente. Sigue estos pasos en tu máquina (ya tienes **commits locales** en `main` y rama `**dev`** lista).

## 1. Crear el repositorio en GitHub

1. Entra en [GitHub → New repository](https://github.com/new).
2. **Repository name:** `FutureFin` (exactamente como quieres que figure en la URL).
3. Deja el repo **vacío** (sin README, sin .gitignore, sin licencia) para evitar conflictos en el primer push.

## 2. Añadir remoto y subir ramas

Desde la carpeta del proyecto (`Documents/GitHub/FutureFin` o tu copia sincronizada):

```bash
cd /Users/maxlainz/Documents/GitHub/FutureFin

git remote add origin git@github.com:TU_USUARIO/FutureFin.git
# o con HTTPS:
# git remote add origin https://github.com/TU_USUARIO/FutureFin.git

git push -u origin main
git push -u origin dev
```

## 3. Rama por defecto en GitHub (opcional)

En **Settings → General → Default branch** puedes dejar `main` como predeterminada y seguir trabajando en `dev` con merge/PR hacia `main`.

## Estado local ya preparado

- `**main`**: primer commit con `README.md`, `.gitignore` y carpeta `docs/` de especificación.
- `**dev`**: misma base que `main`; aquí conviene hacer todo el desarrollo nuevo.

