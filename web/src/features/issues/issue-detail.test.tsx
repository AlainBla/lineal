import { within, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";
import { attachShortcuts, shortcuts } from "@/lib/shortcuts";

import { IssueDetailPage } from "./issue-detail";
import { resetKnownUsers } from "./users";
import { makeIssue, mockGraphql, newClient, renderWithApp, TEAM_META } from "./test-fixtures";
import { clearToasts } from "./toast-store";
import { Toaster } from "./toaster";

vi.mock("./rich-editor", () => ({
  RichEditor: (p: {
    ariaLabel: string;
    placeholder?: string;
    value: string;
    editable?: boolean;
    onChange?: (v: string) => void;
    onBlur?: () => void;
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

function handlers() {
  return {
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
});

const page = () => <IssueDetailPage ws="acme" splat="ENG-120/checkout-ueberarbeiten" />;

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

  it("zeigt Sub-Issues mit Fortschritt, Kommentare mit Reaktionen und Relationen-Abschnitt", async () => {
    mockGraphql(handlers());
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
    const subs = await screen.findByRole("region", { name: "Sub-issues" });
    expect(await within(subs).findByText("Zahlungsformular umbauen")).toBeInTheDocument();
    expect(within(subs).getByLabelText("Sub-issue progress")).toHaveTextContent("1/1");

    const comment = await screen.findByRole("article", { name: "Comment by Alice" });
    expect(within(comment).getByTestId("comment-body")).toHaveTextContent("Sieht gut aus @Alice");
    expect(within(comment).getByRole("button", { name: "👍 1" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByRole("region", { name: "Related issues" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Activity" })).toBeInTheDocument();
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

  it("legt eine Relation „blocking“ über den Dialog an", async () => {
    const gql = mockGraphql(handlers());
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
    await screen.findByLabelText("Title");
    await userEvent.keyboard("m");
    await userEvent.keyboard("x");
    const input = await screen.findByLabelText("Issue identifier");
    await userEvent.type(input, "ENG-132");
    await userEvent.click(screen.getByRole("button", { name: "Add relation" }));
    await waitFor(() => expect(gql.of("IssueRelationCreate")).toHaveLength(1));
    expect(gql.of("IssueRelationCreate")[0]!.variables).toEqual({
      input: { issueId: "i1", relatedIssueId: "i9", type: "blocks" },
    });
    expect(await screen.findByText("Anderes Issue")).toBeInTheDocument();
  });

  it("zeigt einen Fehlerzustand mit Wiederholen, wenn das Issue nicht lädt", async () => {
    mockGraphql({ IssueDetail: () => new Error("not allowed") });
    await renderWithApp(page(), newClient(), "/acme/issue/ENG-120/x");
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not load the issue");
    expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
  });
});
