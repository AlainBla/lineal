import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { mockGraphql, newClient, renderWithApp } from "@/features/issues/test-fixtures";
import type { Initiative, Project } from "@/gql/graphql";

import { buildInitiativeTree } from "./initiatives-page";
import { ProjectPage } from "./project-page";
import { filterProjects } from "./projects-page";

const ref = (id: string, name: string, key: string | null = null) => ({ id, name, key });

function project(over: Partial<Project> = {}): Project {
  return {
    id: "p-1",
    identifier: "P-ENG-1",
    previousIdentifiers: [],
    slugId: "abc",
    name: "Checkout Relaunch",
    summary: "Rebuild checkout",
    description: null,
    icon: null,
    color: "#4cb782",
    priority: 2,
    status: { id: "ps-1", name: "In Progress", type: "started", color: "#f2c94c" },
    health: "atRisk",
    progress: 0.4,
    scope: 10,
    completedScope: 4,
    issueCount: 10,
    completedIssueCount: 4,
    startDate: null,
    startDateResolution: null,
    targetDate: "2026-12-01",
    targetDateResolution: null,
    startedAt: null,
    completedAt: null,
    canceledAt: null,
    archivedAt: null,
    leadTeam: ref("team-eng", "Engineering", "ENG"),
    teams: [ref("team-eng", "Engineering", "ENG")],
    lead: ref("u-alice", "Alice"),
    members: [],
    labels: [],
    initiatives: [],
    milestones: [
      {
        id: "m-1",
        project: { id: "p-1", identifier: "P-ENG-1", name: "Checkout Relaunch" },
        name: "Beta",
        description: null,
        targetDate: "2026-11-01",
        sortOrder: 1,
        status: "planned",
        progress: 0.5,
        issueCount: 4,
        completedIssueCount: 2,
        createdAt: "2026-10-01T00:00:00Z",
        updatedAt: "2026-10-01T00:00:00Z",
      },
    ],
    openIssueCount: 6,
    createdAt: "2026-10-01T00:00:00Z",
    updatedAt: "2026-10-01T00:00:00Z",
    ...over,
  };
}

afterEach(() => vi.unstubAllGlobals());

describe("ProjectPage", () => {
  const handlers = (posted: string[] = []) => ({
    PlanningProject: () => ({ project: project() }),
    PlanningProjectUpdates: () => ({ projectUpdates: [] }),
    PlanningProjectUpdateCreate: (v: Record<string, unknown>) => {
      posted.push(`${String(v.health)}:${String(v.body)}`);
      return { projectUpdateCreate: { success: true } };
    },
    PlanningMilestoneCreate: () => ({ projectMilestoneCreate: { success: true } }),
  });

  it("renders header, tabs, properties and milestones", async () => {
    mockGraphql(handlers());
    await renderWithApp(
      <ProjectPage ws="acme" projectRef="P-ENG-1" tab="overview" />,
      newClient(),
      "/acme/project/P-ENG-1/overview",
    );
    expect(await screen.findByRole("heading", { name: "Checkout Relaunch" })).toBeInTheDocument();
    for (const t of ["Overview", "Issues", "Updates", "Milestones"]) {
      expect(screen.getByRole("link", { name: t })).toBeInTheDocument();
    }
    expect(screen.getByText("At risk")).toBeInTheDocument();
    expect(screen.getByText("Rebuild checkout")).toBeInTheDocument();
    expect(screen.getByText("Beta")).toBeInTheDocument();
  });

  it("posts a status update with health", async () => {
    const posted: string[] = [];
    mockGraphql(handlers(posted));
    await renderWithApp(
      <ProjectPage ws="acme" projectRef="P-ENG-1" tab="updates" />,
      newClient(),
      "/acme/project/P-ENG-1/updates",
    );
    const user = userEvent.setup();
    await screen.findByRole("form", { name: "Post update" });
    await user.click(screen.getByRole("radio", { name: "Off track" }));
    await user.type(screen.getByLabelText("Update text"), "Slipping");
    await user.click(screen.getByRole("button", { name: "Post update" }));
    await waitFor(() => expect(posted).toEqual(["offTrack:Slipping"]));
  });
});

describe("planning helpers", () => {
  it("filters projects by tab", () => {
    const a = project();
    const b = project({
      id: "p-2",
      status: { id: "x", name: "Backlog", type: "backlog", color: "#000000" },
    });
    expect(filterProjects([a, b], "active")).toEqual([a]);
    expect(filterProjects([a, b], "backlog")).toEqual([b]);
    expect(filterProjects([a, b], "all")).toHaveLength(2);
  });

  it("builds an indented initiative tree, multi-parent children repeat", () => {
    const init = (id: string, parents: string[]) =>
      ({
        id,
        name: id,
        parentInitiatives: parents.map((p) => ref(p, p)),
      }) as unknown as Initiative;
    const nodes = buildInitiativeTree([init("a", []), init("b", []), init("c", ["a", "b"])]);
    expect(nodes.map((n) => `${n.initiative.id}:${n.depth}`)).toEqual(["a:0", "c:1", "b:0", "c:1"]);
  });
});
