import { useEffect, useState } from "react";
import "./App.css";

type HealthResponse = {
  status: string;
  service: string;
  version: string;
};

export default function App() {
  const [health, setHealth] = useState<HealthResponse | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    fetch("/v1/health")
      .then(async (res) => {
        if (!res.ok) {
          throw new Error(`HTTP ${res.status}`);
        }
        return res.json() as Promise<HealthResponse>;
      })
      .then((json) => {
        if (!cancelled) {
          setHealth(json);
        }
      })
      .catch((e: unknown) => {
        if (!cancelled) {
          setError(e instanceof Error ? e.message : String(e));
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <main className="layout">
      <h1>FutureFin</h1>
      <p className="muted">
        Cliente web en desarrollo. La API debe estar en{" "}
        <code>http://127.0.0.1:8080</code> (o usar el proxy de Vite).
      </p>
      <section className="card">
        <h2>Estado de la API</h2>
        {error ? (
          <p className="error">
            No se pudo contactar <code>/v1/health</code>: {error}
          </p>
        ) : health ? (
          <pre className="mono">{JSON.stringify(health, null, 2)}</pre>
        ) : (
          <p className="muted">Comprobando…</p>
        )}
      </section>
    </main>
  );
}
