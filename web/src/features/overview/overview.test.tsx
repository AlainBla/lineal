import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { mockGraphql, newClient, renderWithApp } from "@/features/issues/test-fixtures";
import type { OverviewCounts, WorkspaceOverviewGql } from "@/gql/graphql";

import { OverviewPage } from "./overview-page";
import { orderTeams } from "./workspace-issues-page";

const counts = (open: number): OverviewCounts => ({
  openIssues: open,
  inProgress: 1,
  triage: 0,
  triageOldestAt: null,
  completed: 2,
  completedPoints: 3,
  overdue: 0,
  slaAtRisk: 0,
  slaBreached: 0,
});
const health = { onTrack: 1, atRisk: 0, offTrack: 0, noUpdate: 0, total: 1 };

function overview(includeSubTeams: boolean): WorkspaceOverviewGql {
  return {
    period: "7d",
    includeSubTeams,
    teams: [
      {
        teamId: "t-eng",
        key: "ENG",
        name: "Engineering",
        parentId: null,
        depth: 0,
        counts: counts(includeSubTeams ? 15 : 10),
        throughput: [],
        cycleProgress: 0.4,
        projects: health,
      },
      {
        teamId: "t-web",
        key: "WEB",
        name: "Web",
        parentId: "t-eng",
        depth: 1,
        counts: counts(5),
        throughput: [],
        cycleProgress: null,
        projects: health,
      },
    ],
    totals: counts(15),
    projects: health,
    initiatives: [],
    atRisk: [],
  };
}

afterEach(() => vi.unstubAllGlobals());

describe("OverviewPage rollup", () => {
  it("passes includeSubTeams and shows the rolled-up numbers", async () => {
    const gql = mockGraphql({
      WorkspaceOverview: (v) => ({ workspaceOverview: overview(v.includeSubTeams === true) }),
    });
    await renderWithApp(<OverviewPage ws="acme" />, newClient(), "/acme/overview");
    const eng = await screen.findByRole("article", { name: "Team ENG" });
    expect(within(eng).getByText("15")).toBeInTheDocument();
    expect(gql.of("WorkspaceOverview")[0]!.variables.includeSubTeams).toBe(true);

    await userEvent.setup().click(screen.getByLabelText("Include sub-teams"));
    await waitFor(() =>
      expect(
        within(screen.getByRole("article", { name: "Team ENG" })).getByText("10"),
      ).toBeInTheDocument(),
    );
    expect(gql.of("WorkspaceOverview").at(-1)!.variables.includeSubTeams).toBe(false);
  });
});

describe("orderTeams", () => {
  it("nests sub-teams below their parent", () => {
    const out = orderTeams([
      { id: "w", key: "WEB", parentId: "e" },
      { id: "d", key: "DES", parentId: null },
      { id: "e", key: "ENG", parentId: null },
    ]);
    expect(out.map((o) => [o.team.key, o.depth])).toEqual([
      ["DES", 0],
      ["ENG", 0],
      ["WEB", 1],
    ]);
  });
});
