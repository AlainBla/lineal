import { expect, type APIRequestContext } from "@playwright/test";

/** Adresse des Servers (der MCP-Client spricht direkt mit dem Backend, nicht mit dem Vite-Proxy). */
export const API_BASE = `http://127.0.0.1:${process.env.LINEAL_E2E_API_PORT ?? "18741"}`;

/** Von `scripts/e2e.sh` angelegte API-Keys (erfundene Tokens, Besitzer Bob). */
export const API_KEY = process.env.LINEAL_E2E_API_KEY ?? "";
export const API_KEY_ENG_ONLY = process.env.LINEAL_E2E_API_KEY_ENG ?? "";

export interface ToolResult {
  isError: boolean;
  /** Geparster JSON-Text der Antwort (Daten bzw. Fehlermodell). */
  data: Record<string, unknown>;
}

/** Ruft ein MCP-Tool per JSON-RPC über Streamable HTTP auf (`POST /mcp`, Bearer-API-Key). */
export async function callTool(
  request: APIRequestContext,
  key: string,
  name: string,
  args: Record<string, unknown> = {},
): Promise<ToolResult> {
  const res = await request.post(`${API_BASE}/mcp`, {
    headers: {
      authorization: `Bearer ${key}`,
      accept: "application/json, text/event-stream",
      "content-type": "application/json",
    },
    data: { jsonrpc: "2.0", id: 1, method: "tools/call", params: { name, arguments: args } },
  });
  expect(res.status(), await res.text()).toBe(200);
  const body = (await res.json()) as {
    error?: unknown;
    result?: { isError?: boolean; content?: Array<{ text?: string }> };
  };
  expect(body.error, JSON.stringify(body)).toBeUndefined();
  const text = body.result?.content?.[0]?.text ?? "{}";
  return {
    isError: body.result?.isError === true,
    data: JSON.parse(text) as Record<string, unknown>,
  };
}
