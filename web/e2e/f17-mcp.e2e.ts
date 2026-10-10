import { expect, test } from "@playwright/test";

import { login, unique } from "./helpers";
import { API_KEY, API_KEY_ENG_ONLY, callTool } from "./mcp";

// docs/08 F17: KI-Agent über MCP (HTTP-Client gegen /mcp mit API-Key).
test("F17: an agent lists and saves issues through MCP and the UI shows the result", async ({
  page,
  request,
}) => {
  test.skip(!API_KEY, "run via scripts/e2e.sh (creates the API keys)");

  // Ohne Token: 401 mit WWW-Authenticate.
  const anon = await request.post("/mcp", { data: {} });
  expect(anon.status()).toBe(401);

  const listed = await callTool(request, API_KEY, "list_issues", { team: "ENG", limit: 5 });
  expect(listed.isError).toBe(false);
  expect(Array.isArray(listed.data.issues)).toBe(true);

  const title = unique("Agent: Retry-Logik prüfen");
  const saved = await callTool(request, API_KEY, "save_issue", {
    title,
    team: "ENG",
    description: "Angelegt über den MCP-Server.",
  });
  expect(saved.isError, JSON.stringify(saved.data)).toBe(false);
  // In MCP-Antworten ist `id` die Kennung (ENG-123).
  const identifier = String(saved.data.id ?? "");
  expect(identifier).toMatch(/^ENG-\d+$/);

  // Der Agent findet das Issue wieder ...
  const found = await callTool(request, API_KEY, "get_issue", { id: identifier });
  expect(found.isError).toBe(false);
  expect(found.data.title).toBe(title);

  // ... und die Web-App zeigt es ebenfalls.
  await login(page, "bob");
  await page.goto("/acme/team/ENG/all");
  await expect(page.getByText(title)).toBeVisible();
});

test("F17: an API key restricted to ENG cannot see or change other teams", async ({ request }) => {
  test.skip(!API_KEY_ENG_ONLY, "run via scripts/e2e.sh (creates the API keys)");

  const own = await callTool(request, API_KEY_ENG_ONLY, "list_issues", { team: "ENG", limit: 3 });
  expect(own.isError).toBe(false);

  // WEB existiert (Sub-Team von ENG) und ist für Bob selbst sichtbar, für den Key aber nicht.
  const other = await callTool(request, API_KEY_ENG_ONLY, "list_issues", { team: "WEB" });
  expect(other.isError).toBe(true);
  expect(other.data.error).toBe("not_found");
  const write = await callTool(request, API_KEY_ENG_ONLY, "save_issue", {
    title: unique("Soll nicht entstehen"),
    team: "WEB",
  });
  expect(write.isError).toBe(true);
  expect(write.data.error).toBe("not_found");

  // Der unbeschränkte Key sieht WEB.
  const full = await callTool(request, API_KEY, "list_issues", { team: "WEB", limit: 3 });
  expect(full.isError).toBe(false);
});
