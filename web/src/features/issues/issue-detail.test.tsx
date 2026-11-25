import { act, fireEvent, within, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";
import { attachShortcuts, shortcuts } from "@/lib/shortcuts";
import { getUiState, setUiState } from "@/app/ui-state";

import type { CollabProvider } from "@/features/collab/provider";

import { IssueDetailPage } from "./issue-detail";
import type { IssueExtras } from "./queries";
import { resetKnownUsers } from "./users";
import { keys } from "./queries";
import { makeIssue, mockGraphql, newClient, renderWithApp, TEAM_META } from "./test-fixtures";
import { clearToasts } from "./toast-store";
import { Toaster } from "./toaster";

// Standard: Collab-Endpunkt nicht erreichbar → Stufe 1 (die Echtzeit-Tests liegen in features/collab).
// `collab.session` ist je Test umstellbar, um `connecting`/`connected` abzudecken (LIN-95); der
// afterEach-Hook setzt wieder auf den Fallback zurück.
const collab = vi.hoisted(() => {
  const fallback = {
    provider: null,
    status: "fallback",
    canWrite: false,
    synced: false,
    everConnected: false,
    generation: 0,
  };
  return { fallback, session: { ...fallback } as Record<string, unknown> };
});

vi.mock("@/features/collab/use-collab", () => ({
  useCollab: () => collab.session,
}));

// Minimaler Provider-Stub, damit `Presence` (awareness/doc) in der Live-Ansicht nicht abstürzt.
function providerStub(): CollabProvider {
  return {
    awareness: { getStates: () => new Map(), on: () => {}, off: () => {} },
    doc: { clientID: 1 },
  } as unknown as CollabProvider;
}

vi.mock("./rich-editor", () => ({
  RichEditor: (p: {
    ariaLabel: string;
    placeholder?: string;
    value: string;
    editable?: boolean;
    onChange?: (v: string) => void;
    onBlur?: () => void;
    autoFocus?: boolean;
  }) =>
    p.editable === false ? (
      <p data-testid="comment-body">{p.value}</p>
    ) : (
      <textarea
        aria-label={p.ariaLabel}
        placeholder={p.placeholder}
        defaultValue={p.value}
        onChange={(e) => p.onChange?.(e.target.value)}
        onBlur={p.onBlur}
        autoFocus={p.autoFocus}
      />
    ),
}));

const ISSUE = makeIssue({
  id: "i1",
  identifier: "ENG-120",
  title: "Checkout überarbeiten",
  description: "Beschreibung **fett**",
  priority: 2,
  stateId: "s-progress",
  assigneeId: "user-bob",
  estimate: 3,
  labelIds: ["l-bug"],
  projectId: "p-1",
  dueDate: "2026-11-01",
});
const CHILD = makeIssue({
  id: "i2",
  identifier: "ENG-132",
  title: "Zahlungsformular umbauen",
  parentId: "i1",
  stateId: "s-done",
});

const OTHER_TEAM_CHILD = makeIssue({
  id: "i3",
  identifier: "DES-4",
  title: "Icon-Set liefern",
  teamId: "team-des",
  parentId: "i1",
  stateId: "s-todo",
});

function handlers(extras: Partial<IssueExtras> = {}) {
  return {
    IssueExtras: () => ({
      issue: {
        id: "i1",
        children: [CHILD, OTHER_TEAM_CHILD],
        relations: { blocks: [], blockedBy: [], relatedTo: [], duplicateOf: null, duplicates: [] },
        history: [],
        ...extras,
      },
    }),
    IssueDetail: (v: Record<string, unknown>) => {
      if (v.id === "ENG-120" || v.id === "i1") return { issue: ISSUE };
      return { issue: makeIssue({ id: "i9", identifier: String(v.id), title: "Anderes Issue" }) };
    },
    TeamMeta: () => ({
      team: TEAM_META.team,
      workflowStates: TEAM_META.states,
      issueLabels: TEAM_META.labels,
      cycles: [],
      projects: TEAM_META.projects,
    }),
    TeamIssues: () => ({ issues: { nodes: [ISSUE, CHILD], hasNextPage: false, endCursor: null } }),
    IssueComments: () => ({
      comments: {
        nodes: [
          {
            id: "c1aaaaaa-1",
            body: "Sieht gut aus @Alice",
            userId: "user-alice",
            user: { id: "user-alice", name: "alice", displayName: "Alice" },
            issueId: "i1",
            parentId: null,
            quotedText: null,
            editedAt: null,
            resolvedAt: null,
            reactionData: [{ emoji: "👍", count: 1, userIds: ["user-bob"] }],
            createdAt: "2026-10-02T08:00:00Z",
            updatedAt: "2026-10-02T08:00:00Z",
          },
        ],
      },
    }),
    IssueUpdate: (v: Record<string, unknown>) => ({
      issueUpdate: { success: true, issue: { ...ISSUE, ...(v.input as object) } },
    }),
    IssueRelationCreate: () => ({ issueRelationCreate: { success: true } }),
  };
}

let detach: () => void;
beforeEach(() => {
  detach = attachShortcuts();
});
afterEach(() => {
  detach();
  vi.unstubAllGlobals();
  shortcuts.reset();
  clearToasts();
  resetKnownUsers();
  // Collab-Session für den nächsten Test auf den Standard (Fallback) zurücksetzen (LIN-95).
  collab.session = { ...collab.fallback };
  // `detailsOpen` ist modul-global: für nachfolgende Tests auf den Standard (offen, LIN-94)
  // zurücksetzen, damit das angedockte Desktop-Panel wie beim ersten Laden sichtbar ist.
  setUiState({ detailsOpen: true });
});

const page = () => <IssueDetailPage ws="acme" splat="ENG-120/checkout-ueberarbeiten" />;

// Media-Query stubben wie app.test.tsx: `max-width`-Query = schmal/breit steuerbar.
function stubViewport(narrow: boolean) {
  const original = window.matchMedia;
  window.matchMedia = ((q: string) => ({
    matches: narrow && q.includes("max-width"),
    media: q,
    addEventListener: () => {},
    removeEventListener: () => {},
  })) as unknown as typeof window.matchMedia;
  return () => {
    window.matchMedia = original;
  };
}

describe("Issue-Detail", () => {
  it("rendert Titel, Beschreibung und die Property-Sidebar in der Reihenfolge aus docs/08", async () => {
    mockGraphql(handlers());
    await renderWithApp(
      <>
        {page()}
        <Toaster />
      </>,
      newClient(),
      "/acme/issue/ENG-120/checkout-ueberarbeiten",
    );
    expect(await screen.findByLabelText("Title")).toHaveValue("Checkout überarbeiten");
    expect(screen.getByLabelText("Description")).toHaveValue("Beschreibung **fett**");

    const sidebar = await screen.findByLabelText("Properties");
    await waitFor(() => expect(within(sidebar).getByText("Engineering")).toBeInTheDocument());
    const labels = [...sidebar.querySelectorAll("dt")].map((d) => d.textContent);
    expect(labels).toEqual([
      "Status",
      "Team",
      "Priority",
      "Assignee",
      "Cycle",
      "Estimate",
      "Due date",
      "Labels",
      "Project",
      "Milestone",
      "Parent",
    ]);
    const row = (name: string) => sidebar.querySelector(`[data-property="${name}"]`)!;
    expect(within(row("Status") as HTMLElement).getByText("In Progress")).toBeInTheDocument();
    expect(within(row("Priority") as HTMLElement).getByText("High")).toBeInTheDocument();
    expect(within(row("Assignee") as HTMLElement).getByText("Bob")).toBeInTheDocument();
    expect(within(row("Estimate") as HTMLElement).getByText("3")).toBeInTheDocument();
    expect(within(row("Due date") as HTMLElement).getByText("2026-11-01")).toBeInTheDocument();
    expect(within(row("Labels") as HTMLElement).getByText("Bug")).toBeInTheDocument();
    expect(
      within(row("Project") as HTMLElement).getByText("Checkout Relaunch"),
    ).toBeInTheDocument();
  });

  it("Pos 1/Ende scrollen den Ticket-Inhalt ganz nach oben/unten (LIN-91)", async () => {
    mockGraphql(handlers());
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
    const article = await screen.findByLabelText("Issue ENG-120");
    const scrollTo = vi.fn();
    // jsdom implementiert scrollTo nicht – auf der Instanz stubben.
    (article as unknown as { scrollTo: typeof scrollTo }).scrollTo = scrollTo;
    Object.defineProperty(article, "scrollHeight", { value: 4321, configurable: true });

    fireEvent.keyDown(document.body, { key: "Home" });
    expect(scrollTo).toHaveBeenCalledWith({ top: 0 });

    fireEvent.keyDown(document.body, { key: "End" });
    expect(scrollTo).toHaveBeenLastCalledWith({ top: 4321 });
  });

  it("Home/End bleiben in Eingabefeldern Cursor-Navigation, kein Scroll (LIN-91)", async () => {
    mockGraphql(handlers());
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
    const article = await screen.findByLabelText("Issue ENG-120");
    const scrollTo = vi.fn();
    (article as unknown as { scrollTo: typeof scrollTo }).scrollTo = scrollTo;
    const title = await screen.findByLabelText("Title");
    fireEvent.keyDown(title, { key: "Home" });
    fireEvent.keyDown(title, { key: "End" });
    expect(scrollTo).not.toHaveBeenCalled();
  });

  it("zeigt Sub-Issues mit Fortschritt, Kommentare mit Reaktionen und Relationen-Abschnitt", async () => {
    mockGraphql(handlers());
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
    const subs = await screen.findByRole("region", { name: "Sub-issues" });
    expect(await within(subs).findByText("Zahlungsformular umbauen")).toBeInTheDocument();
    // Kind aus einem anderen Team (nur über Issue.children sichtbar, nicht in der ENG-Liste).
    expect(within(subs).getByText("Icon-Set liefern")).toBeInTheDocument();
    expect(within(subs).getByText("DES-4")).toBeInTheDocument();
    expect(within(subs).getByLabelText("Sub-issue progress")).toHaveTextContent("1/2");

    const comment = await screen.findByRole("article", { name: "Comment by Alice" });
    expect(within(comment).getByTestId("comment-body")).toHaveTextContent("Sieht gut aus @Alice");
    expect(within(comment).getByRole("button", { name: "👍 1" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByRole("region", { name: "Related issues" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Activity" })).toBeInTheDocument();
  });

  it("mischt Verlauf und Kommentare chronologisch und schwärzt unsichtbare Ziele", async () => {
    mockGraphql(
      handlers({
        history: [
          {
            id: "h-late",
            actorId: "user-bob",
            actorName: null,
            source: "user",
            changes: { stateId: { from: "s-todo", to: "s-done" } },
            addedLabelIds: [],
            removedLabelIds: [],
            relationChanges: null,
            flags: [],
            createdAt: "2026-10-03T08:00:00Z",
          },
          {
            id: "h-early",
            actorId: null,
            actorName: null,
            source: "import",
            changes: { projectId: { from: null, to: null } },
            addedLabelIds: ["l-bug"],
            removedLabelIds: [],
            relationChanges: null,
            flags: ["created"],
            createdAt: "2026-10-01T08:00:00Z",
          },
        ],
      }),
    );
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
    const timeline = await screen.findByRole("list", { name: "Activity timeline" });
    const early = await within(timeline).findByLabelText(
      "Import created the issue, changed the project (a private item), added label Bug",
    );
    const comment = await within(timeline).findByRole("article", { name: "Comment by Alice" });
    const late = within(timeline).getByLabelText("Bob changed status from Todo to Done");
    // Reihenfolge: Import (1.10.) < Kommentar (2.10.) < Statuswechsel (3.10.)
    expect(early.compareDocumentPosition(comment) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(comment.compareDocumentPosition(late) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("F.07: Shift+M öffnet den Meilenstein-Picker des Projekts, Auswahl sendet projectMilestoneId", async () => {
    const gql = mockGraphql(handlers());
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
    await screen.findByLabelText("Title");
    await screen.findByText("Set milestone");
    await userEvent.keyboard("{Shift>}M{/Shift}");
    const input = await screen.findByPlaceholderText("Set milestone…");
    await userEvent.type(input, "Beta{Enter}");
    await waitFor(() => expect(gql.of("IssueUpdate")).toHaveLength(1));
    expect(gql.of("IssueUpdate")[0]!.variables).toEqual({
      id: "i1",
      input: { projectMilestoneId: "m-beta" },
    });
  });

  it("Taste S öffnet den Status-Picker, Auswahl sendet issueUpdate", async () => {
    const gql = mockGraphql(handlers());
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
    await screen.findByLabelText("Title");
    await userEvent.keyboard("s");
    const input = await screen.findByPlaceholderText("Change status…");
    await userEvent.type(input, "Done{Enter}");
    await waitFor(() => expect(gql.of("IssueUpdate")).toHaveLength(1));
    expect(gql.of("IssueUpdate")[0]!.variables).toEqual({
      id: "i1",
      input: { stateId: "s-done" },
    });
  });

  it("Tasten P, A, L, Umschalt+E öffnen die weiteren Picker; I weist mir zu", async () => {
    const gql = mockGraphql(handlers());
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
    await screen.findByLabelText("Title");
    await waitFor(() =>
      expect(screen.getAllByText("Engineering").length).toBeGreaterThanOrEqual(2),
    );

    for (const [key, placeholder] of [
      ["p", "Set priority…"],
      ["a", "Assign to…"],
      ["l", "Change labels…"],
      ["{Shift>}e{/Shift}", "Set estimate…"],
    ] as const) {
      await userEvent.keyboard(key);
      expect(await screen.findByPlaceholderText(placeholder)).toBeInTheDocument();
      await userEvent.keyboard("{Escape}");
      await waitFor(() => expect(screen.queryByPlaceholderText(placeholder)).toBeNull());
    }

    await userEvent.keyboard("i");
    await waitFor(() => expect(gql.of("IssueUpdate")).toHaveLength(1));
    expect(gql.of("IssueUpdate")[0]!.variables).toEqual({
      id: "i1",
      input: { assigneeId: "user-bob" },
    });
  });

  it("Einzeltasten-Kürzel sind beim Tippen im Titel inaktiv", async () => {
    mockGraphql(handlers());
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
    const title = await screen.findByLabelText("Title");
    await userEvent.click(title);
    await userEvent.keyboard("s");
    expect(screen.queryByPlaceholderText("Change status…")).toBeNull();
    expect(title).toHaveValue("Checkout überarbeitens");
  });

  it("Esc führt per Router-Verlauf zurück zur Liste, von der das Issue geöffnet wurde (LIN-4)", async () => {
    mockGraphql(handlers());
    const { router } = await renderWithApp(page(), newClient(), "/acme/team/ENG/all");
    await router.navigate({ to: "/acme/issue/ENG-120/x" as "/" });
    await screen.findByLabelText("Title");
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(router.state.location.pathname).toBe("/acme/team/ENG/all"));
  });

  it("Esc ohne Verlauf (direkter Aufruf) landet in der Team-Liste", async () => {
    mockGraphql(handlers());
    const { router } = await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
    await screen.findByLabelText("Title");
    await waitFor(() =>
      expect(screen.getAllByText("Engineering").length).toBeGreaterThanOrEqual(2),
    );
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(router.state.location.pathname).toBe("/acme/team/ENG/all"));
  });

  it("Esc im Titelfeld und im Editor verlässt nur das Feld; erst das zweite Esc navigiert", async () => {
    mockGraphql(handlers());
    const { router } = await renderWithApp(page(), newClient(), "/acme/team/ENG/all");
    await router.navigate({ to: "/acme/issue/ENG-120/x" as "/" });
    const title = await screen.findByLabelText("Title");
    for (const field of [title, screen.getByLabelText("Description")]) {
      await userEvent.click(field);
      expect(field).toHaveFocus();
      await userEvent.keyboard("{Escape}");
      expect(field).not.toHaveFocus();
      expect(router.state.location.pathname).toBe("/acme/issue/ENG-120/x");
    }
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(router.state.location.pathname).toBe("/acme/team/ENG/all"));
  });

  it("Esc schließt einen offenen Picker, ohne zu navigieren", async () => {
    mockGraphql(handlers());
    const { router } = await renderWithApp(page(), newClient(), "/acme/team/ENG/all");
    await router.navigate({ to: "/acme/issue/ENG-120/x" as "/" });
    await screen.findByLabelText("Title");
    await userEvent.keyboard("s");
    expect(await screen.findByPlaceholderText("Change status…")).toBeInTheDocument();
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByPlaceholderText("Change status…")).toBeNull());
    expect(router.state.location.pathname).toBe("/acme/issue/ENG-120/x");
  });

  it("zeigt „Mentioned in“ mit Quelle und blendet den Abschnitt ohne Rückverweise aus", async () => {
    mockGraphql(
      handlers({
        mentionedIn: [
          {
            id: "i7",
            identifier: "ENG-140",
            title: "Beschreibt ENG-120",
            source: "description",
            commentId: null,
            createdAt: "2026-10-03T08:00:00Z",
            stateType: "started",
            stateName: "In Progress",
            stateColor: "#f2c94c",
          },
          {
            id: "i8",
            identifier: "WEB-9",
            title: "Kommentiert ENG-120",
            source: "comment",
            commentId: "c9",
            createdAt: "2026-10-04T08:00:00Z",
            stateType: "completed",
            stateName: "Done",
            stateColor: "#4cb782",
          },
        ],
      }),
    );
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
    const sec = await screen.findByRole("region", { name: "Mentioned in" });
    expect(within(sec).getByText("Beschreibt ENG-120")).toBeInTheDocument();
    expect(within(sec).getByText("in the description")).toBeInTheDocument();
    expect(within(sec).getByText("in a comment")).toBeInTheDocument();
    const link = within(sec).getByText("Kommentiert ENG-120").closest("a");
    expect(link?.getAttribute("href")).toContain("WEB-9");
    // Status-Icon je Rückverweis, mit Statusname als Label und Hover-Tooltip (LIN-55).
    expect(within(sec).getByRole("img", { name: "In Progress" })).toBeInTheDocument();
    expect(within(sec).getByRole("img", { name: "Done" })).toBeInTheDocument();
    const tooltips = Array.from(sec.querySelectorAll("[title]")).map((t) =>
      t.getAttribute("title"),
    );
    expect(tooltips).toEqual(expect.arrayContaining(["In Progress", "Done"]));
  });

  it("blendet „Mentioned in“ ohne Rückverweise aus", async () => {
    const gql = mockGraphql(handlers({ mentionedIn: [] }));
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
    await screen.findByLabelText("Title");
    await waitFor(() => expect(gql.of("IssueExtras").length).toBeGreaterThanOrEqual(1));
    await screen.findByRole("region", { name: "Related issues" });
    expect(screen.queryByRole("region", { name: "Mentioned in" })).toBeNull();
  });

  it("legt eine Relation „blocking“ über den Dialog an", async () => {
    let blocks: Array<{
      id: string;
      identifier: string;
      title: string;
      stateType?: string;
      stateName?: string;
      stateColor?: string;
    }> = [];
    const h = handlers();
    const gql = mockGraphql({
      ...h,
      IssueExtras: () => ({
        issue: {
          id: "i1",
          children: [],
          relations: { blocks, blockedBy: [], relatedTo: [], duplicateOf: null, duplicates: [] },
          history: [],
        },
      }),
      IssuePickerSearch: () => ({
        searchIssues: {
          nodes: [
            { issue: { id: "i1", identifier: "ENG-120", title: "Dieses", stateType: "started" } },
            {
              issue: {
                id: "i9",
                identifier: "ENG-132",
                title: "Anderes Issue",
                stateType: "started",
              },
            },
          ],
        },
      }),
      IssueRelationCreate: () => {
        blocks = [
          {
            id: "i9",
            identifier: "ENG-132",
            title: "Anderes Issue",
            stateType: "started",
            stateName: "In Progress",
            stateColor: "#f2c94c",
          },
        ];
        return { issueRelationCreate: { success: true } };
      },
    });
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
    await screen.findByLabelText("Title");
    await userEvent.keyboard("m");
    await userEvent.keyboard("x");
    const input = await screen.findByLabelText("Search issues");
    await userEvent.type(input, "ENG-132");
    // Das aktuelle Ticket (ENG-120) ist ausgeblendet; Enter wählt den ersten Treffer.
    await screen.findByRole("option", { name: /ENG-132/ });
    expect(screen.queryByRole("option", { name: /ENG-120/ })).toBeNull();
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(gql.of("IssueRelationCreate")).toHaveLength(1));
    expect(gql.of("IssueRelationCreate")[0]!.variables).toEqual({
      input: { issueId: "i1", relatedIssueId: "i9", type: "blocks" },
    });
    // Die Relation kommt aus der neu geladenen Detail-Query.
    const rel = await screen.findByRole("region", { name: "Related issues" });
    expect(await within(rel).findByText("Anderes Issue")).toBeInTheDocument();
    expect(within(rel).getByText("Blocking")).toBeInTheDocument();
    expect(gql.of("IssueExtras").length).toBeGreaterThanOrEqual(2);
  });

  it("entfernt eine Relation per X-Button und bietet Undo (LIN-80)", async () => {
    let blocks = [
      {
        id: "i9",
        identifier: "ENG-132",
        title: "Anderes Issue",
        stateType: "started",
        stateName: "In Progress",
        stateColor: "#f2c94c",
      },
    ];
    const h = handlers();
    const gql = mockGraphql({
      ...h,
      IssueExtras: () => ({
        issue: {
          id: "i1",
          children: [],
          relations: { blocks, blockedBy: [], relatedTo: [], duplicateOf: null, duplicates: [] },
          history: [],
        },
      }),
      IssueRelationDelete: () => {
        blocks = [];
        return { issueRelationDelete: { success: true } };
      },
    });
    await renderWithApp(
      <>
        {page()}
        <Toaster />
      </>,
      newClient(),
      "/acme/issue/ENG-120/x",
    );
    const rel = await screen.findByRole("region", { name: "Related issues" });
    expect(await within(rel).findByText("Anderes Issue")).toBeInTheDocument();

    await userEvent.click(within(rel).getByRole("button", { name: "Remove Blocking ENG-132" }));
    await waitFor(() => expect(gql.of("IssueRelationDelete")).toHaveLength(1));
    // Richtung wie `issueRelationCreate`: issueId ist das Issue, das blockiert.
    expect(gql.of("IssueRelationDelete")[0]!.variables).toEqual({
      input: { issueId: "i1", relatedIssueId: "i9", type: "blocks" },
    });
    // Nach dem Neuladen ist die Relation verschwunden.
    await waitFor(() => expect(within(rel).queryByText("Anderes Issue")).toBeNull());
    // Undo-Toast stellt die Relation per `issueRelationCreate` wieder her.
    const undo = await screen.findByRole("button", { name: "Undo" });
    await userEvent.click(undo);
    await waitFor(() => expect(gql.of("IssueRelationCreate")).toHaveLength(1));
    expect(gql.of("IssueRelationCreate")[0]!.variables).toEqual({
      input: { issueId: "i1", relatedIssueId: "i9", type: "blocks" },
    });
  });

  it("Glocke zeigt den Abo-Zustand des Servers und schaltet ihn um", async () => {
    let subscribed = true;
    const h = handlers();
    const gql = mockGraphql({
      ...h,
      IssueExtras: () => ({
        issue: {
          id: "i1",
          subscribed,
          children: [],
          relations: {
            blocks: [],
            blockedBy: [],
            relatedTo: [],
            duplicateOf: null,
            duplicates: [],
          },
          history: [],
        },
      }),
      IssueSubscription: () => {
        subscribed = !subscribed;
        return { issueUnsubscribe: { success: true, changed: true } };
      },
    });
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
    const bell = await screen.findByRole("button", { name: "Unsubscribe from this issue" });
    expect(bell).toHaveAttribute("aria-pressed", "true");
    await userEvent.click(bell);
    await waitFor(() => expect(gql.of("IssueSubscription")).toHaveLength(1));
    expect(gql.of("IssueSubscription")[0]!.variables).toEqual({ id: "i1" });
    expect(await screen.findByRole("button", { name: "Subscribe to this issue" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  });

  it("zeigt einen Fehlerzustand mit Wiederholen, wenn das Issue nicht lädt", async () => {
    mockGraphql({ IssueDetail: () => new Error("not allowed") });
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not load the issue");
    expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
  });

  it("Doppelklick auf Comment legt nur einen Kommentar an (B4.13)", async () => {
    const gql = mockGraphql({
      ...handlers(),
      CommentCreate: () => ({
        commentCreate: {
          success: true,
          comment: {
            id: "c-new",
            body: "Hallo",
            userId: "user-bob",
            user: { id: "user-bob", name: "bob", displayName: "Bob" },
            issueId: "i1",
            parentId: null,
            quotedText: null,
            editedAt: null,
            resolvedAt: null,
            reactionData: [],
            createdAt: "2026-10-04T08:00:00Z",
            updatedAt: "2026-10-04T08:00:00Z",
          },
        },
      }),
    });
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
    await userEvent.type(await screen.findByLabelText("Leave a comment"), "Hallo");
    await userEvent.dblClick(screen.getByRole("button", { name: "Comment" }));
    await waitFor(() => expect(gql.of("CommentCreate")).toHaveLength(1));
    await new Promise((r) => setTimeout(r, 50));
    expect(gql.of("CommentCreate")).toHaveLength(1);
  });

  it("Enter dreimal im Sub-Issue-Feld legt nur ein Sub-Issue an (B4.13)", async () => {
    const gql = mockGraphql({
      ...handlers(),
      IssueCreate: () => ({
        issueCreate: {
          success: true,
          issue: makeIssue({ id: "i-new", identifier: "ENG-200", title: "Neu", parentId: "i1" }),
        },
      }),
    });
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
    await userEvent.click(await screen.findByRole("button", { name: /New sub-issue/ }));
    const input = await screen.findByLabelText("Sub-issue title");
    await userEvent.type(input, "Neu{Enter}{Enter}{Enter}");
    await waitFor(() => expect(gql.of("IssueCreate")).toHaveLength(1));
  });

  it("Leave a reply fokussiert das Antwortfeld (B4.12), Anker nutzen die volle ID (B4.22)", async () => {
    mockGraphql(handlers());
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
    const comment = await screen.findByRole("article", { name: "Comment by Alice" });
    expect(comment.id).toBe("comment-c1aaaaaa-1");
    await userEvent.click(within(comment).getByRole("button", { name: "Leave a reply…" }));
    expect(await screen.findByLabelText("Leave a reply")).toHaveFocus();
  });

  const coded = (message: string, code: string) => Object.assign(new Error(message), { code });

  it("Geteiltes Issue ohne Teamzugriff: schreibgeschützt, Kennung statt „Team“ (B4.16)", async () => {
    mockGraphql({
      ...handlers(),
      TeamMeta: () => coded("Could not find referenced Team", "not_found"),
    });
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
    const title = await screen.findByLabelText("Title");
    await waitFor(() => expect(title).toHaveAttribute("readonly"));
    const sidebar = await screen.findByLabelText("Properties");
    await waitFor(() => expect(within(sidebar).getByText("ENG")).toBeInTheDocument());
    expect(within(sidebar).queryByRole("button", { name: "Priority: change" })).toBeNull();
    expect(within(sidebar).queryByText("Team")).not.toBeNull();
    expect(screen.queryByRole("button", { name: "Set due date" })).toBeNull();
  });

  it("Teamdaten nicht ladbar (z. B. 429): Hinweis mit Retry statt „Team“ (B4.27)", async () => {
    mockGraphql({ ...handlers(), TeamMeta: () => new Error("Too many requests") });
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
    const sidebar = await screen.findByLabelText("Properties");
    expect(await within(sidebar).findByText("Could not load team")).toBeInTheDocument();
    expect(within(sidebar).getByRole("button", { name: "Retry" })).toBeInTheDocument();
    expect(within(sidebar).queryByRole("button", { name: "Status: change" })).toBeNull();
  });

  it("Titel ist mehrzeilig (Textarea), fremde Titeländerung überschreibt keine Eingabe (B4.37, B4.34)", async () => {
    mockGraphql(handlers());
    const client = newClient();
    await renderWithApp(page(), client, "/acme/issue/ENG-120/x");
    const title = await screen.findByLabelText("Title");
    expect(title.tagName).toBe("TEXTAREA");
    await userEvent.type(title, " plus");
    expect(title).toHaveValue("Checkout überarbeiten plus");
    client.setQueryData(keys.issue("ENG-120"), { ...ISSUE, title: "Von Alice geändert" });
    expect(await screen.findByRole("status")).toHaveTextContent("Von Alice geändert");
    expect(title).toHaveValue("Checkout überarbeiten plus");
  });

  it("Gelöschter Parent wird als gelöscht angezeigt (B4.36)", async () => {
    const child = makeIssue({ id: "i1", identifier: "ENG-120", title: "Kind", parentId: "gone" });
    mockGraphql({
      ...handlers(),
      IssueDetail: (v) => (v.id === "gone" ? coded("not found", "not_found") : { issue: child }),
    });
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
    expect(await screen.findByText("Parent issue was deleted")).toBeInTheDocument();
  });

  it("LIN-13: Kopfzeile zeigt Kennung und Titel (mit Tooltip) und folgt Titeländerungen", async () => {
    mockGraphql(handlers());
    const client = newClient();
    await renderWithApp(page(), client, "/acme/issue/ENG-120/x");
    await screen.findByLabelText("Title");
    const nav = screen.getByRole("navigation", { name: "Issue breadcrumb" });
    expect(within(nav).getByText("ENG-120")).toBeInTheDocument();
    const t = within(nav).getByText("Checkout überarbeiten");
    expect(t).toHaveAttribute("title", "Checkout überarbeiten");
    act(() => {
      client.setQueryData(keys.issue("ENG-120"), { ...ISSUE, title: "Neuer Titel" });
    });
    expect(await within(nav).findByText("Neuer Titel")).toHaveAttribute("title", "Neuer Titel");
  });

  it("LIN-13: Titel in der Kopfzeile erscheint erst, wenn das große Titelfeld aus dem Blick scrollt", async () => {
    const observers: { cb: IntersectionObserverCallback }[] = [];
    vi.stubGlobal(
      "IntersectionObserver",
      class {
        constructor(cb: IntersectionObserverCallback) {
          observers.push({ cb });
        }
        observe() {}
        disconnect() {}
        unobserve() {}
      },
    );
    try {
      mockGraphql(handlers());
      await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
      await screen.findByLabelText("Title");
      const header = screen.getByTestId("header-title");
      const fire = (isIntersecting: boolean) =>
        act(() => {
          for (const o of observers)
            o.cb([{ isIntersecting } as IntersectionObserverEntry], {} as IntersectionObserver);
        });
      expect(header).toHaveAttribute("aria-hidden", "true");
      fire(true);
      expect(header).toHaveAttribute("aria-hidden", "true");
      fire(false);
      expect(header).toHaveAttribute("aria-hidden", "false");
      fire(true);
      expect(header).toHaveAttribute("aria-hidden", "true");
    } finally {
      vi.unstubAllGlobals();
    }
  });
});

describe("Beschreibung beim Verbinden (LIN-95)", () => {
  it("connecting: zeigt die Beschreibung schreibgeschützt (kein Skeleton) mit Verbinde-Hinweis", async () => {
    collab.session = { ...collab.fallback, status: "connecting" };
    mockGraphql(handlers());
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
    await screen.findByLabelText("Title");
    // Der Text aus dem schnellen GraphQL-Query ist sofort sichtbar – als reine Anzeige (read-only),
    // nicht als bearbeitbares Description-Feld.
    expect(screen.getByText("Beschreibung **fett**")).toBeInTheDocument();
    expect(screen.queryByLabelText("Description")).toBeNull();
    // Statushinweis signalisiert den laufenden Verbindungsaufbau (Editieren noch nicht bereit).
    expect(screen.getByText("Connecting…")).toBeInTheDocument();
  });

  it("disconnected ohne je verbunden gewesen zu sein: ebenfalls Vorschau mit Verbinde-Hinweis", async () => {
    collab.session = { ...collab.fallback, status: "disconnected", everConnected: false };
    mockGraphql(handlers());
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
    await screen.findByLabelText("Title");
    expect(screen.getByText("Beschreibung **fett**")).toBeInTheDocument();
    expect(screen.queryByLabelText("Description")).toBeNull();
    expect(screen.getByText("Connecting…")).toBeInTheDocument();
  });

  it("connected: der Live-Editor greift – leerer Collab-Wert statt GraphQL-Text", async () => {
    collab.session = {
      provider: providerStub(),
      status: "connected",
      canWrite: true,
      synced: true,
      everConnected: true,
      generation: 1,
    };
    mockGraphql(handlers());
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
    const desc = await screen.findByLabelText("Description");
    // Collab-Pfad: der Inhalt kommt aus dem Yjs-Dokument, `value` bleibt leer (kein Doppel-Insert).
    expect(desc).toHaveValue("");
    // Kein statischer GraphQL-Text, stattdessen der Live-Status.
    expect(screen.queryByText("Beschreibung **fett**")).toBeNull();
    expect(screen.getByText("Live")).toBeInTheDocument();
  });

  it("fallback: Stufe 1 bleibt unverändert – editierbarer statischer Text", async () => {
    collab.session = { ...collab.fallback };
    mockGraphql(handlers());
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
    const desc = await screen.findByLabelText("Description");
    expect(desc.tagName).toBe("TEXTAREA");
    expect(desc).toHaveValue("Beschreibung **fett**");
  });
});

describe("Blocked-Indikator (LIN-79)", () => {
  const relatedOpen = {
    id: "r5",
    identifier: "ENG-5",
    title: "Zahlungsanbieter anbinden",
    stateType: "started",
    stateName: "In Progress",
    stateColor: "#F2C94C",
  };

  it("zeigt im Header ein barrierefreies Blocked-Badge, das die Blocker nennt", async () => {
    const blocked = makeIssue({
      id: "i1",
      identifier: "ENG-120",
      title: "Checkout überarbeiten",
      stateId: "s-progress",
      blockedByIdentifiers: ["ENG-5", "ENG-9"],
    });
    mockGraphql({ ...handlers(), IssueDetail: () => ({ issue: blocked }) });
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/checkout-ueberarbeiten");
    const badge = await screen.findByTestId("blocked-badge");
    expect(badge).toHaveTextContent("Blocked");
    expect(within(badge).getByLabelText("Blocked by ENG-5, ENG-9")).toBeInTheDocument();
  });

  it("blendet das Badge aus, wenn keine offenen Blocker vorhanden sind", async () => {
    mockGraphql(handlers());
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/checkout-ueberarbeiten");
    await screen.findByLabelText("Title");
    expect(screen.queryByTestId("blocked-badge")).toBeNull();
  });

  it("zeigt das Badge, nachdem über den Dialog ein Blocker angelegt wurde", async () => {
    let detailIssue = makeIssue({
      id: "i1",
      identifier: "ENG-120",
      title: "Checkout überarbeiten",
      stateId: "s-progress",
    });
    const gql = mockGraphql({
      ...handlers(),
      IssueDetail: () => ({ issue: detailIssue }),
      IssuePickerSearch: () => ({
        searchIssues: {
          nodes: [
            { issue: { id: "i1", identifier: "ENG-120", title: "Dieses", stateType: "started" } },
            { issue: { id: "i9", identifier: "ENG-132", title: "Blocker", stateType: "started" } },
          ],
        },
      }),
      IssueRelationCreate: () => {
        // Der Server kennt ENG-120 jetzt als blockiert durch ENG-132.
        detailIssue = { ...detailIssue, blockedByIdentifiers: ["ENG-132"] };
        return { issueRelationCreate: { success: true } };
      },
    });
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
    await screen.findByLabelText("Title");
    expect(screen.queryByTestId("blocked-badge")).toBeNull();
    // „Blocked by" (m b) über den Picker anlegen.
    await userEvent.keyboard("m");
    await userEvent.keyboard("b");
    const input = await screen.findByLabelText("Search issues");
    await userEvent.type(input, "ENG-132");
    await screen.findByRole("option", { name: /ENG-132/ });
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(gql.of("IssueRelationCreate")).toHaveLength(1));
    // Das Badge erscheint, weil der `["issue", ref]`-Cache invalidiert und neu geladen wird.
    expect(await screen.findByTestId("blocked-badge")).toBeInTheDocument();
  });

  it("hebt in der Relationsliste die 'Blocked by'-Zeile mit Icon hervor", async () => {
    mockGraphql(
      handlers({
        relations: {
          blocks: [],
          blockedBy: [relatedOpen],
          relatedTo: [],
          duplicateOf: null,
          duplicates: [],
        },
      }),
    );
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/checkout-ueberarbeiten");
    const related = await screen.findByLabelText("Related issues");
    const row = (await within(related).findByText("ENG-5")).closest("a")!;
    expect(within(row).getByText("Blocked by")).toBeInTheDocument();
    expect(within(row).getAllByLabelText("Blocked by").length).toBeGreaterThan(0);
  });
});

describe("Eigenschaften-Panel mobil (LIN-93)", () => {
  it("Schmal + geschlossen: kein Panel-Sheet und kein Scrim", async () => {
    const restore = stubViewport(true);
    try {
      mockGraphql(handlers());
      await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/checkout-ueberarbeiten");
      await screen.findByLabelText("Title");
      // `detailsOpen` wird beim Mount auf schmal zurückgesetzt – Sheet bleibt zu.
      expect(screen.queryByLabelText("Issue details")).toBeNull();
      expect(screen.queryByTestId("details-scrim")).toBeNull();
      expect(screen.queryByLabelText("Properties")).toBeNull();
    } finally {
      restore();
    }
  });

  it("Schmal + offen: Scrim, Schließen-Knopf und Eigenschaften sichtbar; Scrim schließt", async () => {
    const restore = stubViewport(true);
    try {
      mockGraphql(handlers());
      await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/checkout-ueberarbeiten");
      await screen.findByLabelText("Title");
      // Öffnen wie der Topbar-Knopf „Toggle details" / Shortcut `]`.
      act(() => setUiState({ detailsOpen: true }));

      const scrim = await screen.findByTestId("details-scrim");
      expect(scrim).toBeInTheDocument();
      expect(screen.getByLabelText("Close details")).toBeInTheDocument();
      expect(await screen.findByLabelText("Properties")).toBeInTheDocument();
      const sheet = screen.getByLabelText("Issue details");
      expect(sheet.className).toContain("fixed");

      fireEvent.click(scrim);
      await waitFor(() => expect(getUiState().detailsOpen).toBe(false));
      expect(screen.queryByLabelText("Issue details")).toBeNull();
      expect(screen.queryByTestId("details-scrim")).toBeNull();
    } finally {
      restore();
    }
  });

  it("Schmal + offen: der Schließen-Knopf schließt das Sheet", async () => {
    const restore = stubViewport(true);
    try {
      mockGraphql(handlers());
      await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/checkout-ueberarbeiten");
      await screen.findByLabelText("Title");
      act(() => setUiState({ detailsOpen: true }));

      const close = await screen.findByLabelText("Close details");
      fireEvent.click(close);
      await waitFor(() => expect(getUiState().detailsOpen).toBe(false));
      expect(screen.queryByLabelText("Issue details")).toBeNull();
    } finally {
      restore();
    }
  });

  it("Breit + Default: Panel angedockt (md:block), kein Scrim/Schließen-Knopf", async () => {
    const restore = stubViewport(false);
    try {
      mockGraphql(handlers());
      await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/checkout-ueberarbeiten");
      await screen.findByLabelText("Title");

      const aside = await screen.findByLabelText("Issue details");
      expect(aside.className).toContain("md:block");
      expect(await screen.findByLabelText("Properties")).toBeInTheDocument();
      // Angedockt (kein Overlay): kein Scrim, kein Schließen-Knopf.
      expect(screen.queryByTestId("details-scrim")).toBeNull();
      expect(screen.queryByLabelText("Close details")).toBeNull();
    } finally {
      restore();
    }
  });
});

describe("Eigenschaften-Zeile mobil (LIN-92)", () => {
  it("Schmal: Status, Priorität, Zuständiger und Labels stehen in der Zeile – ohne Panel", async () => {
    const restore = stubViewport(true);
    try {
      mockGraphql(handlers());
      await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/checkout-ueberarbeiten");
      await screen.findByLabelText("Title");
      const row = await screen.findByLabelText("Issue properties");
      // Status und Priorität kommen direkt aus dem Issue.
      expect(within(row).getByText("In Progress")).toBeInTheDocument();
      expect(within(row).getByText("High")).toBeInTheDocument();
      // Zuständiger und Labels erscheinen erst nach Laden von Team-Meta und Nutzern.
      expect(await within(row).findByText("Bob")).toBeInTheDocument();
      expect(await within(row).findByText("Bug")).toBeInTheDocument();
      // Das angedockte Panel ist schmal + geschlossen nicht gerendert – keine Doppelanzeige.
      expect(screen.queryByLabelText("Properties")).toBeNull();
    } finally {
      restore();
    }
  });

  it("Schmal: Tipp auf den Status-Chip öffnet den Status-Picker", async () => {
    const restore = stubViewport(true);
    try {
      mockGraphql(handlers());
      await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/checkout-ueberarbeiten");
      const row = await screen.findByLabelText("Issue properties");
      await userEvent.click(within(row).getByRole("button", { name: "Change status" }));
      expect(await screen.findByPlaceholderText("Change status…")).toBeInTheDocument();
    } finally {
      restore();
    }
  });

  it("Breit: die Zeile ist nicht vorhanden, das Panel bleibt angedockt", async () => {
    const restore = stubViewport(false);
    try {
      mockGraphql(handlers());
      await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/checkout-ueberarbeiten");
      await screen.findByLabelText("Title");
      expect(screen.queryByLabelText("Issue properties")).toBeNull();
      expect(await screen.findByLabelText("Properties")).toBeInTheDocument();
    } finally {
      restore();
    }
  });
});

describe("Desktop Panel-Toggle (LIN-94)", () => {
  it("Breit + Default: das angedockte Panel ist sichtbar", async () => {
    const restore = stubViewport(false);
    try {
      mockGraphql(handlers());
      await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/checkout-ueberarbeiten");
      await screen.findByLabelText("Title");
      // Ohne Zutun (Standard offen) steht das Panel da.
      expect(await screen.findByLabelText("Issue details")).toBeInTheDocument();
      expect(screen.getByLabelText("Properties")).toBeInTheDocument();
    } finally {
      restore();
    }
  });

  it("Breit: detailsOpen=false blendet das Panel aus, wieder true zeigt es", async () => {
    const restore = stubViewport(false);
    try {
      mockGraphql(handlers());
      await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/checkout-ueberarbeiten");
      await screen.findByLabelText("Title");
      expect(await screen.findByLabelText("Properties")).toBeInTheDocument();

      // Toggle aus (wie Topbar-Knopf / Shortcut `]`): Panel verschwindet, kein Overlay/Scrim.
      act(() => setUiState({ detailsOpen: false }));
      await waitFor(() => expect(screen.queryByLabelText("Issue details")).toBeNull());
      expect(screen.queryByLabelText("Properties")).toBeNull();
      expect(screen.queryByTestId("details-scrim")).toBeNull();

      // Toggle wieder an: angedocktes Panel kommt zurück.
      act(() => setUiState({ detailsOpen: true }));
      expect(await screen.findByLabelText("Properties")).toBeInTheDocument();
      expect(screen.getByLabelText("Issue details").className).toContain("md:block");
    } finally {
      restore();
    }
  });

  it("Breit: der Ticket-Inhalt bleibt gerendert, wenn das Panel aus ist", async () => {
    const restore = stubViewport(false);
    try {
      mockGraphql(handlers());
      await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/checkout-ueberarbeiten");
      await screen.findByLabelText("Title");

      act(() => setUiState({ detailsOpen: false }));
      await waitFor(() => expect(screen.queryByLabelText("Issue details")).toBeNull());
      // Titel/Beschreibung bleiben sichtbar; nur das angedockte Panel ist weg.
      expect(screen.getByLabelText("Title")).toHaveValue("Checkout überarbeiten");
      expect(screen.getByLabelText("Issue ENG-120")).toBeInTheDocument();
    } finally {
      restore();
    }
  });

  it("Schmal schließt nur solange die schmale Ansicht lebt; danach gilt wieder der Standard (offen)", async () => {
    const restore = stubViewport(true);
    try {
      mockGraphql(handlers());
      const { unmount } = await renderWithApp(
        page(),
        newClient(),
        "/acme/issue/ENG-120/checkout-ueberarbeiten",
      );
      await screen.findByLabelText("Title");
      // Schmal schließt das Sheet beim Mount (LIN-93).
      expect(getUiState().detailsOpen).toBe(false);
      // Verlässt man die schmale Ansicht (Unmount/Breiter-Resize), wird der Standard wiederhergestellt,
      // damit Desktop kein durch Schmal gesetztes „zu“ erbt.
      unmount();
      expect(getUiState().detailsOpen).toBe(true);
    } finally {
      restore();
    }
  });
});
