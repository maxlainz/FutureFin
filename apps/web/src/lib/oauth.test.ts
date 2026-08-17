import { describe, expect, it } from "vitest";
import {
  authorizeErrorMessage,
  parseAuthorizeParams,
  redirectHostLabel,
} from "./oauth";

const FULL =
  "?client_id=ffc_abc&redirect_uri=https%3A%2F%2Fclaude.ai%2Fapi%2Fmcp%2Fauth_callback" +
  "&response_type=code&code_challenge=xyz&code_challenge_method=S256&state=st1" +
  "&scope=read&resource=https%3A%2F%2Fff.example%2Fmcp";

describe("parseAuthorizeParams", () => {
  it("parsea una query completa con opcionales", () => {
    const p = parseAuthorizeParams(FULL);
    expect(p).not.toBeNull();
    expect(p!.client_id).toBe("ffc_abc");
    expect(p!.redirect_uri).toBe("https://claude.ai/api/mcp/auth_callback");
    expect(p!.state).toBe("st1");
    expect(p!.scope).toBe("read");
    expect(p!.resource).toBe("https://ff.example/mcp");
  });

  it("devuelve null si falta cualquier obligatorio", () => {
    for (const missing of [
      "client_id",
      "redirect_uri",
      "response_type",
      "code_challenge",
      "code_challenge_method",
    ]) {
      const q = new URLSearchParams(FULL);
      q.delete(missing);
      expect(parseAuthorizeParams(`?${q.toString()}`)).toBeNull();
    }
  });

  it("omite opcionales ausentes sin inventarlos", () => {
    const p = parseAuthorizeParams(
      "?client_id=a&redirect_uri=b&response_type=code&code_challenge=c&code_challenge_method=S256",
    );
    expect(p).not.toBeNull();
    expect(p!.state).toBeUndefined();
    expect(p!.scope).toBeUndefined();
  });

  it("method=plain SÍ parsea — el rechazo es del servidor, no del cliente", () => {
    const p = parseAuthorizeParams(
      "?client_id=a&redirect_uri=b&response_type=code&code_challenge=c&code_challenge_method=plain",
    );
    expect(p).not.toBeNull();
    expect(p!.code_challenge_method).toBe("plain");
  });
});

describe("redirectHostLabel", () => {
  it("extrae el host, con puerto si lo hay", () => {
    expect(redirectHostLabel("https://claude.ai/api/mcp/auth_callback")).toBe("claude.ai");
    expect(redirectHostLabel("http://localhost:53682/callback")).toBe("localhost:53682");
  });

  it("devuelve el string original si no parsea como URL", () => {
    expect(redirectHostLabel("no-es-una-url")).toBe("no-es-una-url");
  });
});

describe("authorizeErrorMessage", () => {
  it("tiene mensaje específico para los códigos fatales conocidos", () => {
    expect(authorizeErrorMessage("unknown_client")).toContain("no está registrada");
    expect(authorizeErrorMessage("redirect_uri_mismatch")).toContain("no coincide");
  });

  it("default legible para códigos desconocidos", () => {
    expect(authorizeErrorMessage("algo_nuevo")).toContain("algo_nuevo");
  });
});
