import js from "@eslint/js";
import globals from "globals";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import tseslint from "typescript-eslint";

export default tseslint.config(
  { ignores: ["dist"] },
  {
    extends: [js.configs.recommended, ...tseslint.configs.recommended],
    files: ["**/*.{ts,tsx}"],
    languageOptions: {
      ecmaVersion: 2022,
      globals: globals.browser,
    },
    plugins: {
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      "react-refresh/only-export-components": [
        "warn",
        { allowConstantExport: true },
      ],
      // La app puede servirse bajo un prefijo (Ingress de Home Assistant, X-Forwarded-Prefix):
      // una ruta absoluta escrita a pelo en un `fetch` se sale del subpath y acaba en el 404
      // del proxy. Los selectores cubren las dos formas de literal (comillas y plantilla).
      "no-restricted-syntax": [
        "error",
        {
          selector:
            "CallExpression[callee.name='fetch'] > Literal:first-child[value=/^\\//]",
          message:
            "No pases una ruta absoluta a fetch: envuélvela en apiUrl() (apps/web/src/lib/basePath.ts) o usa apiFetch, o se romperá bajo un prefijo de proxy.",
        },
        {
          selector:
            "CallExpression[callee.name='fetch'] > TemplateLiteral:first-child > TemplateElement:first-child[value.raw=/^\\//]",
          message:
            "No pases una ruta absoluta a fetch: envuélvela en apiUrl() (apps/web/src/lib/basePath.ts) o usa apiFetch, o se romperá bajo un prefijo de proxy.",
        },
      ],
    },
  }
);
