import path from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig, loadEnv } from "vite";
import react from "@vitejs/plugin-react";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
/** Repo root (`FutureFin/`), so `.env` at root applies to dev ports + proxy. */
const repoRoot = path.resolve(__dirname, "..", "..");

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, repoRoot, "");
  const apiPort = env.FUTUREFIN_API_PORT ?? "8081";
  const apiTarget = `http://127.0.0.1:${apiPort}`;
  const webPort = Number(env.WEB_DEV_PORT ?? "8080");

  return {
    plugins: [react()],
    server: {
      port: webPort,
      strictPort: false,
      proxy: {
        "/health": { target: apiTarget, changeOrigin: true },
        "/openapi.json": { target: apiTarget, changeOrigin: true },
        "/v1": { target: apiTarget, changeOrigin: true },
      },
    },
  };
});
