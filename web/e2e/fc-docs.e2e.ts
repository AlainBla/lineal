import { expect, test, type Page } from "@playwright/test";

import { login, unique } from "./helpers";

async function gql<T>(page: Page, query: string, variables: Record<string, unknown> = {}) {
  return page.evaluate(
    async ([q, v]) => {
      const res = await fetch("/graphql", {
        method: "POST",
        headers: { "content-type": "application/json", "x-lineal-csrf": "1" },
        body: JSON.stringify({ query: q, variables: v }),
      });
      return (await res.json()) as T;
    },
    [query, variables] as const,
  );
}

// F.05/F.06 (docs/06 §1.4, §1.8): Dokument aus einem Template am Projekt anlegen, Text markieren und
// kommentieren (Inline-Kommentar); ein zweiter Nutzer sieht Markierung und Thread und antwortet.
test("F.05/F.06: document from template on a project, inline comment visible to a second user", async ({
  browser,
}) => {
  const aliceCtx = await browser.newContext();
  const bobCtx = await browser.newContext();
  const alice = await aliceCtx.newPage();
  const bob = await bobCtx.newPage();
  await login(alice, "alice");
  await login(bob, "bob");

  // Ein Dokument-Template des Workspaces und ein Projekt aus den Demo-Daten.
  const templateName = unique("Runbook");
  const created = await gql<{ data?: { templateCreate: { success: boolean } }; errors?: unknown }>(
    alice,
    `mutation($input: TemplateCreateInput!) { templateCreate(input: $input) { success } }`,
    {
      input: {
        type: "document",
        name: templateName,
        templateData: {
          title: templateName,
          content: "Der Dienst startet nach dem Deployment neu.\n\nZweiter Absatz.",
        },
      },
    },
  );
  expect(created.data?.templateCreate.success).toBe(true);
  const projects = await gql<{ data: { projects: Array<{ identifier: string }> } }>(
    alice,
    `{ projects(team: "ENG", first: 1) { identifier } }`,
  );
  const project = projects.data.projects[0]!.identifier;

  // Alice: Projekt, Tab Resources, neues Dokument aus dem Template.
  await alice.goto(`/acme/project/${project}/resources`);
  await alice.getByRole("button", { name: /New document/ }).click();
  const form = alice.getByRole("form", { name: "New document" });
  await expect(form.getByRole("option", { name: templateName })).toBeAttached();
  await form.getByLabel("Template").selectOption({ label: templateName });
  await form.getByRole("button", { name: "Create document" }).click();
  await expect(alice.getByLabel("Document title")).toHaveValue(templateName);
  await expect(alice.getByRole("status").filter({ hasText: "Live" })).toBeVisible();
  const slug = /\/document\/([^/?#]+)/.exec(alice.url())![1]!;

  // Die Liste am Projekt zeigt das neue Dokument.
  await alice.goto(`/acme/project/${project}/resources`);
  await expect(alice.getByRole("link", { name: templateName })).toBeVisible();
  await alice.getByRole("link", { name: templateName }).click();
  await expect(alice.getByRole("status").filter({ hasText: "Live" })).toBeVisible();

  // Text markieren (zwei Wörter am Anfang) und „Comment“ wählen.
  const aliceEditor = alice.getByRole("textbox", { name: "Document content" });
  await expect(aliceEditor).toContainText("Der Dienst startet");
  await aliceEditor.click();
  await alice.keyboard.press("Control+Home");
  await alice.keyboard.press("Control+Shift+ArrowRight");
  await alice.keyboard.press("Control+Shift+ArrowRight");
  await alice.getByRole("button", { name: "Comment on selection" }).click();
  const panel = alice.getByRole("complementary", { name: "Comment threads" });
  const body = unique("Bitte präzisieren");
  await panel.getByRole("textbox", { name: "Inline comment" }).click();
  await alice.keyboard.type(body);
  await panel.getByRole("button", { name: "Comment", exact: true }).click();
  await expect(panel.getByRole("list", { name: "Open threads" }).getByText(body)).toBeVisible();
  await expect(aliceEditor.locator("[data-comment-id]")).toContainText("Der Dienst");

  // Bob: sieht Markierung und Thread, antwortet.
  await bob.goto(`/acme/document/${slug}`);
  await expect(bob.getByRole("status").filter({ hasText: "Live" })).toBeVisible();
  const bobEditor = bob.getByRole("textbox", { name: "Document content" });
  await expect(bobEditor.locator("[data-comment-id]")).toContainText("Der Dienst", {
    timeout: 15_000,
  });
  await bob.getByRole("button", { name: /Comments \(1\)/ }).click();
  const bobPanel = bob.getByRole("complementary", { name: "Comment threads" });
  await expect(bobPanel.getByRole("list", { name: "Open threads" }).getByText(body)).toBeVisible();
  await bobPanel.getByRole("button", { name: "Leave a reply…" }).click();
  const reply = unique("Erledigt");
  await bobPanel.getByRole("textbox", { name: "Leave a reply" }).click();
  await bob.keyboard.type(reply);
  await bobPanel.getByRole("button", { name: "Reply" }).click();
  await expect(bobPanel.getByRole("list", { name: "Open threads" }).getByText(reply)).toBeVisible();

  // Alice sieht die Antwort und löst den Thread auf: die Markierung verschwindet.
  await expect(panel.getByRole("list", { name: "Open threads" }).getByText(reply)).toBeVisible({
    timeout: 15_000,
  });
  await panel.getByRole("button", { name: "Resolve" }).click();
  await expect(aliceEditor.locator("[data-comment-id]")).toHaveCount(0);
  await expect(bobEditor.locator("[data-comment-id]")).toHaveCount(0, { timeout: 15_000 });

  // Papierkorb: Dokument löschen und aus „Recently deleted“ wiederherstellen.
  await alice.goto(`/acme/project/${project}/resources`);
  alice.once("dialog", (d) => void d.accept());
  await alice.getByRole("button", { name: `Delete ${templateName}` }).click();
  await expect(alice.getByRole("link", { name: templateName })).toHaveCount(0);
  await alice.getByRole("button", { name: "Recently deleted" }).click();
  await alice.getByRole("button", { name: `Restore ${templateName}` }).click();
  await alice.getByRole("button", { name: "Recently deleted" }).click();
  await expect(alice.getByRole("link", { name: templateName })).toBeVisible();

  await aliceCtx.close();
  await bobCtx.close();
});

// F.06: Kommentar an einem Projekt; der Benachrichtigungslink `#comment-<id>` führt (auch über die
// Weiterleitung auf den Overview-Tab) zum Kommentar.
test("F.06: project comment and #comment link scrolls to the comment", async ({ browser }) => {
  const aliceCtx = await browser.newContext();
  const bobCtx = await browser.newContext();
  const alice = await aliceCtx.newPage();
  const bob = await bobCtx.newPage();
  await login(alice, "alice");
  await login(bob, "bob");
  const projects = await gql<{ data: { projects: Array<{ id: string; identifier: string }> } }>(
    alice,
    `{ projects(team: "ENG", first: 1) { id identifier } }`,
  );
  const project = projects.data.projects[0]!;

  await alice.goto(`/acme/project/${project.identifier}/overview`);
  const text = unique("Projektkommentar");
  const section = alice.getByRole("region", { name: "Project comments" });
  await section.getByRole("textbox", { name: "Leave a comment" }).click();
  await alice.keyboard.type(text);
  await section.getByRole("button", { name: "Comment", exact: true }).click();
  await expect(section.getByRole("list", { name: "Project comments list" })).toContainText(text);

  const listed = await gql<{ data: { comments: { nodes: Array<{ id: string; body: string }> } } }>(
    alice,
    `query($p: String) { comments(projectId: $p, first: 100) { nodes { id body } } }`,
    { p: project.id },
  );
  const id = listed.data.comments.nodes.find((c) => c.body.includes(text))!.id;

  await bob.goto(`/acme/project/${project.id}#comment-${id}`);
  const target = bob.locator(`#comment-${id}`);
  await expect(target).toContainText(text);
  await expect(target).toHaveClass(/ring-2/);

  await aliceCtx.close();
  await bobCtx.close();
});
