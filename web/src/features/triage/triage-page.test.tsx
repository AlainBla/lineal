import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";
import { attachShortcuts, shortcuts } from "@/lib/shortcuts";
import {
  makeIssue,
  mockGraphql,
  newClient,
  renderWithApp,
  TEAM_META,
} from "@/features/issues/test-fixtures";
import { clearToasts } from "@/features/issues/toast-store";

import { sortTriage, TriagePage } from "./triage-page";
import { snoozePresets } from "./triage-dialogs";

vi.mock("@/features/issues/issue-detail", () => ({
  IssueDetailPage: ({ splat }: { splat: string }) => <div data-testid="detail">detail {splat}</div>,
}));

const OLD = makeIssue({
  id: "t1",
  identifier: "ENG-7",
  title: "Alter Eingang",
  stateId: "s-triage",
  stateType: "triage",
  creatorId: "user-alice",
  startedTriageAt: "2026-10-01T08:00:00Z",
});
const NEW = makeIssue({
  id: "t2",
  identifier: "ENG-9",
  title: "Neuer Eingang",
  stateType: "triage",
  creatorId: "user-alice",
  startedTriageAt: "2026-10-03T08:00:00Z",
});

function handlers(extra: Record<string, (v: Record<string, unknown>) => unknown> = {}) {
  return {
    TeamMeta: () => ({
      team: { ...TEAM_META.team, triageEnabled: true },
      workflowStates: TEAM_META.states,
      issueLabels: [],
      cycles: [],
      projects: [],
    }),
    TriageIssues: () => ({ triageIssues: [NEW, OLD] }),
    TriageResponsibility: () => ({
      triageResponsibility: {
        triageEnabled: true,
        action: "notify",
        currentUser: { id: "user-bob", name: "Bob" },
      },
    }),
    ...extra,
  };
}

describe("Triage-Ansicht (F.02)", () => {
  beforeEach(() => {
    clearToasts();
    shortcuts.reset();
    attachShortcuts(document);
  });

  it("sortiert älteste zuerst", () => {
    expect(sortTriage([NEW, OLD]).map((i) => i.identifier)).toEqual(["ENG-7", "ENG-9"]);
  });

  it("Snooze-Voreinstellungen liegen in der Zukunft um 09:00", () => {
    const now = new Date(2026, 9, 13, 15, 0); // Dienstag
    const [tomorrow, week] = snoozePresets(now);
    expect(tomorrow!.at.getDate()).toBe(14);
    expect(tomorrow!.at.getHours()).toBe(9);
    expect(week!.at.getDay()).toBe(1);
    expect(week!.at.getTime()).toBeGreaterThan(now.getTime());
  });

  it("zeigt die Liste, wählt das älteste Issue und akzeptiert mit Taste 1", async () => {
    const gql = mockGraphql(
      handlers({
        IssueTriageAccept: () => ({
          issueTriageAccept: { success: true, issue: { ...OLD, stateType: "backlog" } },
        }),
      }),
    );
    await renderWithApp(<TriagePage ws="acme" teamKey="ENG" />, newClient());
    const list = await screen.findByRole("list", { name: "Triage issues" });
    const rows = within(list).getAllByRole("button");
    expect(rows[0]).toHaveTextContent("ENG-7");
    expect(await screen.findByTestId("detail")).toHaveTextContent("detail ENG-7");
    expect(screen.getByLabelText("Issues in triage")).toHaveTextContent("2");
    expect(screen.getByText("On triage: Bob")).toBeInTheDocument();

    await userEvent.keyboard("1");
    const dialog = await screen.findByRole("dialog");
    await userEvent.type(within(dialog).getByLabelText("Comment"), "Passt");
    await userEvent.selectOptions(within(dialog).getByLabelText("Status"), "s-todo");
    await userEvent.click(within(dialog).getByRole("button", { name: "Accept" }));
    await waitFor(() => expect(gql.of("IssueTriageAccept")).toHaveLength(1));
    expect(gql.of("IssueTriageAccept")[0]!.variables).toEqual({
      id: "t1",
      input: { stateId: "s-todo", comment: "Passt" },
    });
  });

  it("lehnt mit Kommentar ab", async () => {
    const gql = mockGraphql(
      handlers({
        IssueTriageDecline: () => ({ issueTriageDecline: { success: true, issue: OLD } }),
      }),
    );
    await renderWithApp(<TriagePage ws="acme" teamKey="ENG" />, newClient());
    await screen.findByRole("list", { name: "Triage issues" });
    await userEvent.click(screen.getByRole("button", { name: /Decline/ }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.type(within(dialog).getByLabelText("Comment"), "Kein Bug");
    await userEvent.click(within(dialog).getByRole("button", { name: "Decline" }));
    await waitFor(() => expect(gql.of("IssueTriageDecline")).toHaveLength(1));
    expect(gql.of("IssueTriageDecline")[0]!.variables).toEqual({
      id: "t1",
      input: { comment: "Kein Bug" },
    });
  });

  it("markiert als Duplikat und snoozt", async () => {
    const gql = mockGraphql(
      handlers({
        IssueDetail: () => ({ issue: makeIssue({ id: "d1", identifier: "ENG-1" }) }),
        IssueTriageDuplicate: () => ({ issueTriageDuplicate: { success: true, issue: OLD } }),
        IssueTriageSnooze: () => ({ issueTriageSnooze: { success: true, issue: OLD } }),
      }),
    );
    await renderWithApp(<TriagePage ws="acme" teamKey="ENG" />, newClient());
    await screen.findByRole("list", { name: "Triage issues" });
    await userEvent.click(screen.getByRole("button", { name: /Mark as duplicate/ }));
    let dialog = await screen.findByRole("dialog");
    await userEvent.type(within(dialog).getByLabelText("Issue identifier"), "ENG-1");
    await userEvent.click(within(dialog).getByRole("button", { name: "Mark as duplicate" }));
    await waitFor(() => expect(gql.of("IssueTriageDuplicate")).toHaveLength(1));
    expect(gql.of("IssueTriageDuplicate")[0]!.variables).toMatchObject({
      id: "t1",
      duplicateOfId: "d1",
    });

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await userEvent.click(screen.getByRole("button", { name: /Snooze/ }));
    dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Snooze" }));
    await waitFor(() => expect(gql.of("IssueTriageSnooze")).toHaveLength(1));
    const at = gql.of("IssueTriageSnooze")[0]!.variables.snoozedUntilAt as string;
    expect(Date.parse(at)).toBeGreaterThan(Date.now());
  });

  it("zeigt bei leerer Triage einen Leerzustand", async () => {
    mockGraphql(handlers({ TriageIssues: () => ({ triageIssues: [] }) }));
    await renderWithApp(<TriagePage ws="acme" teamKey="ENG" />, newClient());
    expect(await screen.findByText("Triage is empty")).toBeInTheDocument();
  });
});
