import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { SESSION_QUERY_KEY } from "@/app/auth";
import { mockGraphql, newClient, renderWithApp, SESSION } from "@/features/issues/test-fixtures";

import { parentCandidates } from "./team-nesting-queries";
import { TeamsSettingsPage } from "./teams-page";
import type { TeamRow } from "./teams-queries";

afterEach(() => vi.unstubAllGlobals());

const row = (over: Partial<TeamRow> & Pick<TeamRow, "id" | "key" | "name">): TeamRow => ({
  parentId: null,
  icon: null,
  color: null,
  visibility: "public",
  allMembersCanJoin: true,
  retiredAt: null,
  createdAt: "2026-01-01T00:00:00Z",
  memberCount: 2,
  ...over,
});

const ENG = row({ id: "team-eng", key: "ENG", name: "Engineering" });
const WEB = row({ id: "team-web", key: "WEB", name: "Web", parentId: "team-eng" });
const DES = row({ id: "team-des", key: "DES", name: "Design" });
const OLD = row({ id: "team-old", key: "OLD", name: "Legacy", retiredAt: "2026-03-01T00:00:00Z" });
const TEAMS = [ENG, WEB, DES, OLD];

function client(role: string) {
  const c = newClient();
  c.setQueryData(SESSION_QUERY_KEY, { ...SESSION, user: { ...SESSION.user, role } });
  return c;
}

describe("parentCandidates", () => {
  it("excludes the team itself, its descendants and retired teams", () => {
    expect(parentCandidates(ENG, TEAMS).map((t) => t.key)).toEqual(["DES"]);
    expect(parentCandidates(WEB, TEAMS).map((t) => t.key)).toEqual(["DES", "ENG"]);
  });
});

const DELETED = {
  id: "team-gone",
  key: "OPS",
  name: "Operations",
  displayName: "Operations",
  parentId: null,
  trashedAt: "2026-10-01T00:00:00Z",
  restorableUntil: "2026-10-31T00:00:00Z",
  subTeamCount: 2,
};

describe("Recently deleted teams", () => {
  it("lists deleted teams and restores one", async () => {
    let deleted = [DELETED];
    const gql = mockGraphql({
      SettingsTeams: () => ({ teams: TEAMS }),
      SettingsDeletedTeams: () => ({ deletedTeams: deleted }),
      SettingsTeamUnarchive: () => {
        deleted = [];
        return { teamUnarchive: { success: true } };
      },
    });
    await renderWithApp(<TeamsSettingsPage ws="acme" />, client("owner"), "/acme/settings/teams");
    const list = await screen.findByRole("list", { name: "Recently deleted teams" });
    expect(list).toHaveTextContent("restorable until 2026-10-31");
    expect(list).toHaveTextContent("with 2 sub-teams");
    await userEvent.click(within(list).getByRole("button", { name: "Restore Operations" }));
    await waitFor(() => expect(gql.of("SettingsTeamUnarchive")).toHaveLength(1));
    expect(gql.of("SettingsTeamUnarchive")[0]!.variables).toEqual({ id: "team-gone" });
    await waitFor(() =>
      expect(
        screen.queryByRole("list", { name: "Recently deleted teams" }),
      ).not.toBeInTheDocument(),
    );
  });

  it("is not requested for regular members", async () => {
    const gql = mockGraphql({ SettingsTeams: () => ({ teams: TEAMS }) });
    await renderWithApp(<TeamsSettingsPage ws="acme" />, client("user"), "/acme/settings/teams");
    await screen.findByRole("list", { name: "Teams" });
    expect(gql.of("SettingsDeletedTeams")).toHaveLength(0);
    expect(screen.queryByRole("button", { name: /^Move / })).not.toBeInTheDocument();
  });
});

const preview = (conflicts: unknown[]) => ({ previewTeamNesting: { conflicts } });

describe("Nesting wizard", () => {
  const base = {
    SettingsTeams: () => ({ teams: TEAMS }),
    SettingsDeletedTeams: () => ({ deletedTeams: [] }),
    SettingsParentStates: () => ({
      workflowStates: [
        { id: "p-todo", name: "Todo", type: "unstarted" },
        { id: "p-prog", name: "In Progress", type: "started" },
        { id: "p-doing", name: "Doing", type: "started" },
      ],
    }),
    SettingsNestTeam: () => ({ nestTeam: { success: true } }),
  };

  it("un-nests a sub-team without a preview", async () => {
    const gql = mockGraphql(base);
    await renderWithApp(<TeamsSettingsPage ws="acme" />, client("owner"), "/acme/settings/teams");
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Move Web" }));
    expect(screen.getByLabelText("Parent team")).toHaveValue("team-eng");
    await user.selectOptions(screen.getByLabelText("Parent team"), "");
    await user.click(screen.getByRole("button", { name: "Make top-level" }));
    await waitFor(() => expect(gql.of("SettingsNestTeam")).toHaveLength(1));
    expect(gql.of("SettingsNestTeam")[0]!.variables).toEqual({
      teamId: "team-web",
      parentId: null,
    });
    expect(gql.of("SettingsNestingPreview")).toHaveLength(0);
  });

  it("shows conflicts, lets the admin choose options and a status mapping", async () => {
    const gql = mockGraphql({
      ...base,
      SettingsNestingPreview: () =>
        preview([
          {
            kind: "membership",
            message: "m",
            details: { userIds: ["u1", "u2"] },
          },
          {
            kind: "visibility",
            message: "Teams would be more open than their parent.",
            details: { teams: [{ teamId: "team-des", from: "public", to: "restricted" }] },
          },
          {
            kind: "workflowState",
            message: "mapped",
            details: {
              states: [
                {
                  id: "s-prog",
                  name: "In Progress",
                  type: "started",
                  targetId: "p-prog",
                  targetName: "In Progress",
                },
              ],
            },
          },
          {
            kind: "label",
            message: "Labels stay.",
            details: { labels: [{ id: "l1", name: "Bug", existingId: "l9" }] },
          },
          { kind: "cycle", message: "Cycles adopted.", details: { upcomingCycles: 2 } },
          { kind: "estimate", message: "e", details: { from: "linear", to: "fibonacci" } },
        ]),
    });
    await renderWithApp(<TeamsSettingsPage ws="acme" />, client("owner"), "/acme/settings/teams");
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Move Design" }));
    await user.selectOptions(screen.getByLabelText("Parent team"), "team-eng");
    await user.click(screen.getByRole("button", { name: "Review changes" }));
    expect(await screen.findByRole("region", { name: "Members" })).toHaveTextContent(
      "Add 2 missing members",
    );
    expect(screen.getByRole("region", { name: "Cycles" })).toHaveTextContent("replaced: 2");
    expect(screen.getByRole("list", { name: "Duplicate labels" })).toHaveTextContent("Bug");
    // Sichtbarkeit muss bestätigt werden, sonst ist „Move team“ gesperrt.
    const move = screen.getByRole("button", { name: "Move team" });
    expect(move).toBeDisabled();
    await user.click(screen.getByLabelText(/Downgrade 1 team to restricted/));
    expect(move).toBeEnabled();
    const target = await screen.findByLabelText("Target for In Progress");
    expect(target).toHaveValue("p-prog");
    await user.selectOptions(target, "p-doing");
    await user.click(screen.getByLabelText(/Use the estimation scale of Engineering/));
    await user.click(move);
    await waitFor(() => expect(gql.of("SettingsNestTeam")).toHaveLength(1));
    expect(gql.of("SettingsNestTeam")[0]!.variables).toEqual({
      teamId: "team-des",
      parentId: "team-eng",
      options: {
        addMissingMembersToParent: true,
        downgradeVisibility: true,
        inheritWorkflowStatuses: true,
        inheritIssueEstimation: false,
        stateMapping: [{ fromStateId: "s-prog", toStateId: "p-doing" }],
      },
    });
  });

  it("shows server errors from the preview", async () => {
    mockGraphql({ ...base, SettingsNestingPreview: () => new Error("max depth exceeded") });
    await renderWithApp(<TeamsSettingsPage ws="acme" />, client("owner"), "/acme/settings/teams");
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Move Design" }));
    await user.selectOptions(screen.getByLabelText("Parent team"), "team-eng");
    await user.click(screen.getByRole("button", { name: "Review changes" }));
    expect(await screen.findByText(/max depth exceeded/)).toBeInTheDocument();
  });
});
