# Publicar el repositorio en GitHub

Esta guía resume operaciones típicas con GitHub para este repo (clonar, crear ramas, y subir cambios).

## 1) Clonar

```bash
git clone https://github.com/maxlainz/FutureFin.git
cd FutureFin
```

## 2) Crear rama de trabajo y push

Desde la carpeta del proyecto:

```bash
git checkout -b my-branch
git push -u origin my-branch
```

## 3) Rama por defecto en GitHub (opcional)

En **Settings → General → Default branch** puedes dejar `main` como predeterminada y seguir trabajando en `dev` con merge/PR hacia `main`.

## Notas

- `main`: estable (releases).
- `dev`: desarrollo.

