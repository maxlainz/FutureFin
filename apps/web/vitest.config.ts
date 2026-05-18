/// <reference types="vitest" />
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    // `node` basta para Fase 4.1 (funciones puras de formato y fechas).
    // Cuando se añadan tests de componentes en Fase 4.4, cambiar a `happy-dom` o `jsdom`.
    environment: "node",
    include: ["src/**/*.test.{ts,tsx}"],
    globals: false,
  },
});
