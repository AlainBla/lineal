import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";
import { getUiState, setUiState } from "@/app/ui-state";
import { shortcuts } from "@/lib/shortcuts";

import { CreateIssueModal } from "./create-issue-modal";
import {
  clearAllCreateDrafts,
  draftScope,
  purgeForeignDrafts,
  saveCreateDraft,
  clearCreatePreset,
  consumeCreateReopen,
  getCreateDraft,
  markCreateReopen,
  openCreateIssue,
  setCreateContext,
  listSavedDrafts,
  openCreateFromTemplate,
} from "./create-store";
import { makeIssue, mockGraphql, renderWithApp, SESSION, TEAM_META } from "./test-fixtures";
import { clearToasts } from "./toast-store";
import { Toaster } from "./toaster";

// TipTap/ProseMirror braucht Layout-APIs, die jsdom nicht hat: der Editor wird durch ein Textfeld ersetzt.
vi.mock("./rich-editor", () => ({
  RichEditor: (p: {
    ariaLabel: string;
    placeholder?: string;
    value: string;
    onChange?: (v: string) => void;
  }) => (
    <textarea
      aria-label={p.ariaLabel}
      placeholder={p.placeholder}
      defaultValue={p.value}
      onChange={(e) => p.onChange?.(e.target.value)}
    />
  ),
}));

function handlers(onCreate?: (input: Record<string, unknown>) => void) {
  let n = 122;
  return {
    IssueTeams: () => ({
      teams: [
        {
          id: "team-eng",
          key: "ENG",
          name: "Engineering",
          parentId: null,
          issueEstimationType: "fibonacci",
        },
        {
          id: "team-des",
          key: "DES",
          name: "Design",
          parentId: null,
          issueEstimationType: "notUsed",
        },
      ],
    }),
    TeamMeta: (v: Record<string, unknown>) =>
      v.team === "team-des"
        ? {
            team: {
              ...TEAM_META.team,
              id: "team-des",
              key: "DES",
              name: "Design",
              cyclesEnabled: false,
              issueEstimationType: "notUsed",
            },
            workflowStates: [],
            issueLabels: [],
            cycles: [],
            projects: [],
          }
        : {
            team: TEAM_META.team,
            workflowStates: TEAM_META.states,
            issueLabels: TEAM_META.labels,
            cycles: [],
            projects: TEAM_META.projects,
          },
    IssueCreate: (v: Record<string, unknown>) => {
      const input = v.input as Record<string, unknown>;
      onCreate?.(input);
      n += 1;
      return {
        issueCreate: {
          success: true,
          issue: makeIssue({ id: `new-${n}`, identifier: `ENG-${n}`, title: String(input.title) }),
        },
      };
    },
  };
}

const SCOPE = draftScope(SESSION.user.id, SESSION.workspace.id);

beforeEach(() => {
  setUiState({ createIssueOpen: false });
});
afterEach(() => {
  vi.unstubAllGlobals();
  shortcuts.reset();
  clearCreatePreset();
  clearAllCreateDrafts();
  clearToasts();
});

async function openModal() {
  await renderWithApp(
    <>
      <CreateIssueModal />
      <Toaster />
    </>,
  );
  act(() => setUiState({ createIssueOpen: true }));
  return screen.findByLabelText("Issue title");
}

describe("Create-Issue-Modal", () => {
  it("öffnet über createIssueOpen mit Team aus dem Pfad und Standardstatus", async () => {
    mockGraphql(handlers());
    await openModal();
    expect(await screen.findByRole("button", { name: "Team: ENG" })).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: "Status: Backlog" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Estimate" })).toBeInTheDocument();
  });

  it("Validierung: Create-Button ohne Titel deaktiviert, Strg+Enter zeigt den Fehler", async () => {
    const gql = mockGraphql(handlers());
    const title = await openModal();
    const button = screen.getByRole("button", { name: /Create issue/ });
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Team: ENG" })).toBeInTheDocument(),
    );
    expect(button).toBeDisabled();

    await userEvent.type(title, "   ");
    await userEvent.keyboard("{Control>}{Enter}{/Control}");
    expect(await screen.findByRole("alert")).toHaveTextContent("Title is required.");
    expect(gql.of("IssueCreate")).toHaveLength(0);

    await userEvent.type(title, "Login schlägt fehl");
    expect(screen.queryByText("Title is required.")).not.toBeInTheDocument();
    await waitFor(() => expect(button).toBeEnabled());
  });

  it("legt ein Issue mit den gewählten Werten an und schließt das Modal", async () => {
    const created: Array<Record<string, unknown>> = [];
    mockGraphql(handlers((i) => created.push(i)));
    const title = await openModal();
    await screen.findByRole("button", { name: "Status: Backlog" });
    await userEvent.type(title, "Login schlägt fehl");
    await userEvent.type(screen.getByLabelText("Description"), "Steps");

    await userEvent.click(screen.getByRole("button", { name: /^Priority/ }));
    await userEvent.type(await screen.findByPlaceholderText("Set priority…"), "2"); // High
    await userEvent.click(screen.getByRole("button", { name: /Create issue/ }));

    await waitFor(() => expect(created).toHaveLength(1));
    expect(created[0]).toMatchObject({
      teamId: "team-eng",
      title: "Login schlägt fehl",
      description: "Steps",
      priority: 2,
    });
    // Der Default-Status wird nicht gesendet: sonst würde er die Triage-Regel aushebeln.
    expect(created[0]).not.toHaveProperty("stateId");
    expect(await screen.findByText("ENG-123 created")).toBeInTheDocument();
    await waitFor(() => expect(getUiState().createIssueOpen).toBe(false));
  });

  it("F.07: Meilenstein-Chip erscheint mit Projekt, Auswahl geht als projectMilestoneId mit", async () => {
    const created: Array<Record<string, unknown>> = [];
    mockGraphql(handlers((i) => created.push(i)));
    const title = await openModal();
    await screen.findByRole("button", { name: "Status: Backlog" });
    expect(screen.queryByRole("button", { name: "Milestone" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Project" }));
    await userEvent.type(await screen.findByPlaceholderText("Add to project…"), "Checkout{Enter}");
    await userEvent.click(await screen.findByRole("button", { name: "Milestone" }));
    await userEvent.type(await screen.findByPlaceholderText("Set milestone…"), "Beta{Enter}");
    await userEvent.type(title, "Mit Meilenstein");
    await userEvent.keyboard("{Control>}{Enter}{/Control}");
    await waitFor(() => expect(created).toHaveLength(1));
    expect(created[0]).toMatchObject({ projectId: "p-1", projectMilestoneId: "m-beta" });
  });

  it("Create more: Modal bleibt offen, Titel/Beschreibung werden geleert, Properties bleiben", async () => {
    const created: Array<Record<string, unknown>> = [];
    mockGraphql(handlers((i) => created.push(i)));
    const title = await openModal();
    await screen.findByRole("button", { name: "Status: Backlog" });
    await userEvent.click(screen.getByRole("switch", { name: "Create more" }));
    await userEvent.click(screen.getByRole("button", { name: /^Priority/ }));
    await userEvent.type(await screen.findByPlaceholderText("Set priority…"), "1"); // Urgent

    await userEvent.type(title, "Erster Bug");
    await userEvent.keyboard("{Control>}{Enter}{/Control}");
    await waitFor(() => expect(created).toHaveLength(1));
    await screen.findByText("ENG-123 created");

    expect(getUiState().createIssueOpen).toBe(true);
    expect(screen.getByLabelText("Issue title")).toHaveValue("");
    expect(screen.getByRole("button", { name: "Priority: Urgent" })).toBeInTheDocument();

    await userEvent.type(screen.getByLabelText("Issue title"), "Zweiter Bug");
    await userEvent.keyboard("{Control>}{Enter}{/Control}");
    await waitFor(() => expect(created).toHaveLength(2));
    expect(created[1]).toMatchObject({ title: "Zweiter Bug", priority: 1 });
    expect(await screen.findByText("ENG-124 created")).toBeInTheDocument();
  });

  it("Serverfehler bleibt im Modal sichtbar, Eingaben bleiben", async () => {
    mockGraphql({ ...handlers(), IssueCreate: () => new Error("Team is retired") });
    const title = await openModal();
    await screen.findByRole("button", { name: "Status: Backlog" });
    await userEvent.type(title, "Bug");
    await userEvent.click(screen.getByRole("button", { name: /Create issue/ }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Team is retired");
    expect(screen.getByLabelText("Issue title")).toHaveValue("Bug");
    expect(getUiState().createIssueOpen).toBe(true);
  });

  it("Teamwechsel leert teamgebundene Werte; Preset aus der Spalte belegt den Status vor", async () => {
    const created: Array<Record<string, unknown>> = [];
    mockGraphql(handlers((i) => created.push(i)));
    await renderWithApp(<CreateIssueModal />);
    act(() => openCreateIssue({ teamKey: "ENG", stateId: "s-progress" }));
    const title = await screen.findByLabelText("Issue title");
    expect(await screen.findByRole("button", { name: "Status: In Progress" })).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Team: ENG" }));
    await userEvent.type(await screen.findByPlaceholderText("Set team…"), "Design{Enter}");
    expect(await screen.findByRole("button", { name: "Team: DES" })).toBeInTheDocument();
    // Design hat weder Estimates noch Cycles
    await waitFor(() => expect(screen.queryByRole("button", { name: "Estimate" })).toBeNull());
    await userEvent.type(title, "Icon-Set");
    await userEvent.click(screen.getByRole("button", { name: /Create issue/ }));
    await waitFor(() => expect(created).toHaveLength(1));
    expect(created[0]).toMatchObject({ teamId: "team-des", title: "Icon-Set" });
    expect(created[0]).not.toHaveProperty("stateId");
  });

  it("Esc mit ungespeichertem Inhalt fragt nach; Discard verwirft, Keep editing behält (B4.26)", async () => {
    mockGraphql(handlers());
    const title = await openModal();
    await screen.findByRole("button", { name: "Status: Backlog" });
    await userEvent.type(title, "Halbfertig");
    await userEvent.keyboard("{Escape}");
    expect(await screen.findByRole("alertdialog")).toBeInTheDocument();
    expect(getUiState().createIssueOpen).toBe(true);
    await userEvent.click(screen.getByRole("button", { name: "Keep editing" }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(screen.getByLabelText("Issue title")).toHaveValue("Halbfertig");

    await userEvent.keyboard("{Escape}");
    await userEvent.click(await screen.findByRole("button", { name: "Discard" }));
    await waitFor(() => expect(getUiState().createIssueOpen).toBe(false));
    expect(getCreateDraft(SCOPE)).toBeNull();
  });

  it("Esc ohne Inhalt schließt sofort", async () => {
    mockGraphql(handlers());
    await openModal();
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(getUiState().createIssueOpen).toBe(false));
  });

  it("Entwurf übersteht Session-Ablauf: nach dem Login wieder da (B4.11)", async () => {
    mockGraphql(handlers());
    const title = await openModal();
    await userEvent.type(title, "Wichtiger Text");
    await waitFor(() => expect(getCreateDraft(SCOPE)?.title).toBe("Wichtiger Text"));
    markCreateReopen(SCOPE);
    act(() => setUiState({ createIssueOpen: false })); // Shell unmountet beim Redirect
    expect(consumeCreateReopen(SCOPE)).toBe(true);
    expect(consumeCreateReopen(SCOPE)).toBe(false);
    act(() => setUiState({ createIssueOpen: true }));
    expect(await screen.findByLabelText("Issue title")).toHaveValue("Wichtiger Text");
  });

  it("Entwürfe sind an Nutzer und Workspace gebunden, Logout löscht alle (Security)", () => {
    const alice = draftScope("user-alice", "org-1");
    const bob = draftScope("user-bob", "org-1");
    const otherWs = draftScope("user-alice", "org-2");
    saveCreateDraft(alice, { title: "Geheim", description: "privat" });
    // anderer Nutzer, anderer Workspace: kein fremder Entwurf
    expect(getCreateDraft(bob)).toBeNull();
    expect(getCreateDraft(otherWs)).toBeNull();
    markCreateReopen(alice);
    expect(consumeCreateReopen(bob)).toBe(false);
    // Login von Bob: Entwürfe anderer Scopes werden verworfen
    purgeForeignDrafts(bob);
    expect(getCreateDraft(alice)).toBeNull();
    // derselbe Nutzer nach Session-Ablauf: Entwurf bleibt, Wiederöffnen genau einmal
    saveCreateDraft(alice, { title: "Weiter", description: "" });
    markCreateReopen(alice);
    purgeForeignDrafts(alice);
    expect(consumeCreateReopen(alice)).toBe(true);
    expect(getCreateDraft(alice)?.title).toBe("Weiter");
    // Logout: alles weg, auch für denselben Nutzer
    clearAllCreateDrafts();
    expect(getCreateDraft(alice)).toBeNull();
    expect(sessionStorage.length).toBe(0);
    expect(localStorage.getItem("lineal-create-draft:" + alice)).toBeNull();
  });

  it("Taste C übernimmt den Seitenkontext (Team/Projekt) (B4.15)", async () => {
    mockGraphql(handlers());
    setCreateContext({ teamKey: "DES", projectId: "p-1" });
    try {
      await openModal();
      expect(await screen.findByRole("button", { name: "Team: DES" })).toBeInTheDocument();
    } finally {
      setCreateContext(null);
    }
  });

  it("Strg+Enter wirkt auch direkt nach der Auswahl im Picker (B4.25)", async () => {
    const created: Array<Record<string, unknown>> = [];
    mockGraphql(handlers((i) => created.push(i)));
    const title = await openModal();
    await screen.findByRole("button", { name: "Status: Backlog" });
    await userEvent.type(title, "Fokus bleibt");
    await userEvent.click(screen.getByRole("button", { name: /^Priority/ }));
    await userEvent.type(await screen.findByPlaceholderText("Set priority…"), "1");
    await userEvent.keyboard("{Control>}{Enter}{/Control}");
    await waitFor(() => expect(created).toHaveLength(1));
    expect(created[0]).toMatchObject({ title: "Fokus bleibt", priority: 1 });
  });

  it("Template-Auswahl: Chip, Vorbelegung aus templateData, templateId im Request (F.16)", async () => {
    const created: Array<Record<string, unknown>> = [];
    mockGraphql({
      ...handlers((i) => created.push(i)),
      IssueTemplates: () => ({
        templates: [
          {
            id: "tpl-bug",
            name: "Bug report",
            description: null,
            teamId: "team-eng",
            teamKey: "ENG",
            isForm: false,
            templateData: {
              priority: 2,
              description: "{{placeholder: Schritte}}Details",
              labelIds: ["l-bug"],
            },
          },
        ],
      }),
      TeamDefaults: () => ({ teamDefaultTemplates: { forMembersId: null, forNonMembersId: null } }),
    });
    const title = await openModal();
    await userEvent.click(await screen.findByRole("button", { name: "Template: none" }));
    await userEvent.type(await screen.findByPlaceholderText("Choose template…"), "Bug{Enter}");
    expect(await screen.findByRole("button", { name: "Template: Bug report" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Priority: High" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Labels" })).toHaveTextContent("1 label");
    await userEvent.type(title, "Login kaputt");
    await userEvent.click(screen.getByRole("button", { name: /Create issue/ }));
    await waitFor(() => expect(created).toHaveLength(1));
    expect(created[0]).toMatchObject({
      title: "Login kaputt",
      templateId: "tpl-bug",
      priority: 2,
      labelIds: ["l-bug"],
    });
    // Beschreibung unverändert: der Server wendet die Template-Beschreibung an.
    expect(created[0]).not.toHaveProperty("description");
  });

  it("Default-Template des Teams ist vorausgewählt und abwählbar", async () => {
    mockGraphql({
      ...handlers(),
      IssueTemplates: () => ({
        templates: [
          {
            id: "tpl-d",
            name: "Standard",
            description: null,
            teamId: "team-eng",
            teamKey: "ENG",
            isForm: false,
            templateData: {},
          },
        ],
      }),
      TeamDefaults: () => ({
        teamDefaultTemplates: { forMembersId: "tpl-d", forNonMembersId: null },
      }),
    });
    await openModal();
    expect(await screen.findByRole("button", { name: "Template: Standard" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Template: Standard" }));
    await userEvent.click(await screen.findByRole("option", { name: /No template/ }));
    expect(await screen.findByRole("button", { name: "Template: none" })).toBeInTheDocument();
  });

  it("Formular-Template rendert Felder, prüft Pflichtfelder und sendet formAnswers", async () => {
    const created: Array<Record<string, unknown>> = [];
    mockGraphql({
      ...handlers((i) => created.push(i)),
      IssueTemplates: () => ({
        templates: [
          {
            id: "tpl-form",
            name: "Anfrage",
            description: null,
            teamId: "team-eng",
            teamKey: "ENG",
            isForm: true,
            templateData: {
              formFields: [
                { id: "t", kind: "title", label: "Kurztitel", required: true },
                {
                  id: "info",
                  kind: "instructions",
                  label: "Hinweis",
                  text: "Bitte genau beschreiben.",
                },
                { id: "d", kind: "longText", label: "Details", required: true },
                { id: "k", kind: "dropdown", label: "Art", options: ["Bug", "Wunsch"] },
                { id: "c", kind: "checkboxes", label: "Bereiche", options: ["Web", "API"] },
                { id: "due", kind: "dueDate", label: "Bis wann" },
              ],
            },
          },
        ],
      }),
      TeamDefaults: () => ({ teamDefaultTemplates: { forMembersId: null, forNonMembersId: null } }),
    });
    await openModal();
    await userEvent.click(await screen.findByRole("button", { name: "Template: none" }));
    await userEvent.click(await screen.findByRole("option", { name: /Anfrage/ }));
    const form = await screen.findByRole("group", { name: "Form" });
    expect(within(form).getByText("Bitte genau beschreiben.")).toBeInTheDocument();
    // Titel kommt aus dem Formular: kein separates Titelfeld, keine Beschreibung.
    expect(screen.queryByLabelText("Issue title")).toBeNull();
    expect(screen.queryByLabelText("Description")).toBeNull();

    await userEvent.type(within(form).getByLabelText("Kurztitel"), "VPN Zugang");
    await userEvent.click(screen.getByRole("button", { name: /Create issue/ }));
    expect(await screen.findByRole("alert")).toHaveTextContent("required fields");
    expect(created).toHaveLength(0);

    await userEvent.type(within(form).getByLabelText("Details"), "Brauche Zugang");
    await userEvent.selectOptions(within(form).getByLabelText("Art"), "Wunsch");
    await userEvent.click(within(form).getByLabelText("API"));
    await userEvent.click(screen.getByRole("button", { name: /Create issue/ }));
    await waitFor(() => expect(created).toHaveLength(1));
    expect(created[0]).toMatchObject({ templateId: "tpl-form" });
    expect(created[0]!.formAnswers).toEqual([
      { fieldId: "t", value: "VPN Zugang" },
      { fieldId: "d", value: "Brauche Zugang" },
      { fieldId: "k", value: "Wunsch" },
      { fieldId: "c", value: ["API"] },
    ]);
    expect(created[0]).not.toHaveProperty("description");
  });

  it("Fälligkeit und Parent landen im Request (F.16)", async () => {
    const created: Array<Record<string, unknown>> = [];
    mockGraphql({
      ...handlers((i) => created.push(i)),
      ParentSearch: () => ({
        searchIssues: {
          nodes: [
            { issue: { id: "p1", identifier: "ENG-2", title: "Epic", stateType: "started" } },
          ],
        },
      }),
      IssueDetail: () => ({ issue: makeIssue({ id: "p1", identifier: "ENG-2", title: "Epic" }) }),
    });
    const title = await openModal();
    await userEvent.type(title, "Mit Parent");
    await userEvent.click(screen.getByRole("button", { name: "Due date: none" }));
    const dueDialog = await screen.findByRole("dialog", { name: /due/i });
    fireEvent.change(within(dueDialog).getByLabelText(/due date/i), {
      target: { value: "2026-12-24" },
    });
    await userEvent.click(within(dueDialog).getByRole("button", { name: /save/i }));
    await userEvent.click(await screen.findByRole("button", { name: "Parent: none" }));
    await userEvent.type(await screen.findByLabelText("Search issues"), "Epic");
    await userEvent.click(await screen.findByRole("button", { name: /ENG-2/ }));
    await userEvent.click(screen.getByRole("button", { name: /Create issue/ }));
    await waitFor(() => expect(created).toHaveLength(1));
    expect(created[0]).toMatchObject({ dueDate: "2026-12-24", parentId: "p1" });
  });

  it("Esc mit Inhalt: Save draft legt einen Draft ab, der Arbeitsentwurf ist leer", async () => {
    mockGraphql(handlers());
    const title = await openModal();
    await screen.findByRole("button", { name: "Status: Backlog" });
    await userEvent.type(title, "Später weiter");
    await userEvent.keyboard("{Escape}");
    await userEvent.click(await screen.findByRole("button", { name: "Save draft" }));
    await waitFor(() => expect(getUiState().createIssueOpen).toBe(false));
    expect(getCreateDraft(SCOPE)).toBeNull();
    expect(listSavedDrafts(SCOPE).map((d) => d.title)).toEqual(["Später weiter"]);
    expect(await screen.findByText("Draft saved")).toBeInTheDocument();
  });

  it("Alt+C öffnet das Modal direkt mit der Template-Auswahl", async () => {
    mockGraphql({
      ...handlers(),
      IssueTemplates: () => ({ templates: [] }),
      TeamDefaults: () => ({ teamDefaultTemplates: { forMembersId: null, forNonMembersId: null } }),
    });
    await renderWithApp(<CreateIssueModal />);
    act(() => openCreateFromTemplate());
    expect(await screen.findByPlaceholderText("Choose template…")).toBeInTheDocument();
  });
});
