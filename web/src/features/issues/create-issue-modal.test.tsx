import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";
import { getUiState, setUiState } from "@/app/ui-state";
import { shortcuts } from "@/lib/shortcuts";

import { CreateIssueModal } from "./create-issue-modal";
import { clearCreatePreset, openCreateIssue } from "./create-store";
import { makeIssue, mockGraphql, renderWithApp, TEAM_META } from "./test-fixtures";
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

beforeEach(() => {
  setUiState({ createIssueOpen: false });
});
afterEach(() => {
  vi.unstubAllGlobals();
  shortcuts.reset();
  clearCreatePreset();
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
    await userEvent.type(await screen.findByPlaceholderText("Set priority…"), "3"); // High
    await userEvent.click(screen.getByRole("button", { name: /Create issue/ }));

    await waitFor(() => expect(created).toHaveLength(1));
    expect(created[0]).toMatchObject({
      teamId: "team-eng",
      title: "Login schlägt fehl",
      description: "Steps",
      stateId: "s-backlog",
      priority: 2,
    });
    expect(await screen.findByText("ENG-123 created")).toBeInTheDocument();
    await waitFor(() => expect(getUiState().createIssueOpen).toBe(false));
  });

  it("Create more: Modal bleibt offen, Titel/Beschreibung werden geleert, Properties bleiben", async () => {
    const created: Array<Record<string, unknown>> = [];
    mockGraphql(handlers((i) => created.push(i)));
    const title = await openModal();
    await screen.findByRole("button", { name: "Status: Backlog" });
    await userEvent.click(screen.getByRole("switch", { name: "Create more" }));
    await userEvent.click(screen.getByRole("button", { name: /^Priority/ }));
    await userEvent.type(await screen.findByPlaceholderText("Set priority…"), "2"); // Urgent

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
});
