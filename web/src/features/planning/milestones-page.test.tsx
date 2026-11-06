import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import "@/app/test-polyfills";
import { mockGraphql, newClient, renderWithApp } from "@/features/issues/test-fixtures";
import type { Project, ProjectMilestone } from "@/gql/graphql";

import { groupMilestones, MilestonesPage } from "./milestones-page";

const ms = (id: string, name: string, over: Partial<ProjectMilestone> = {}) =>
  ({
    id,
    name,
    status: "unstarted",
    targetDate: "2027-06-01",
    issueCount: 3,
    completedIssueCount: 1,
    startedIssueCount: 1,
    targetDateHistory: [],
    ...over,
  }) as ProjectMilestone;

const NOW = Date.parse("2026-11-05T12:00:00Z");
const PROJECTS = [
  {
    id: "p-1",
    identifier: "P-ENG-1",
    name: "Checkout",
    milestones: [
      ms("a", "Spät"),
      ms("b", "Überfällig", { status: "overdue", targetDate: "2026-11-01" }),
      ms("c", "Bald", { targetDate: "2026-11-20" }),
      ms("d", "Ohne Datum", { targetDate: null }),
      ms("e", "Fertig", { status: "done", targetDate: "2026-10-01" }),
    ],
  },
  {
    id: "p-2",
    identifier: "P-ENG-2",
    name: "Payments",
    milestones: [ms("f", "Früher bald", { targetDate: "2026-11-10" })],
  },
] as Project[];

describe("Alle Meilensteine (LIN-25)", () => {
  it("gruppiert nach Fälligkeit und blendet Abgeschlossene aus", () => {
    const groups = groupMilestones(PROJECTS, false, NOW);
    expect(groups.map((g) => g.key)).toEqual(["overdue", "soon", "later", "none"]);
    expect(groups[1]!.rows.map((r) => r.milestone.name)).toEqual(["Früher bald", "Bald"]);
    expect(groupMilestones(PROJECTS, true, NOW).at(-1)!.key).toBe("completed");
  });

  it("verlinkt zur Detailseite und zeigt Abgeschlossene auf Wunsch", async () => {
    mockGraphql({
      PlanningProjects: () => ({
        projectsPage: { nodes: PROJECTS, pageInfo: { hasNextPage: false, endCursor: null } },
      }),
    });
    const user = userEvent.setup();
    await renderWithApp(<MilestonesPage ws="acme" />, newClient(), "/acme/milestones");
    expect(await screen.findByRole("link", { name: "Bald" })).toHaveAttribute(
      "href",
      "/acme/project/P-ENG-1/milestone/c",
    );
    expect(screen.queryByRole("link", { name: "Fertig" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Show completed" }));
    expect(screen.getByRole("link", { name: "Fertig" })).toBeInTheDocument();
    expect(within(screen.getByRole("table")).getAllByText("Checkout").length).toBeGreaterThan(1);
  });

  describe("Kalender-Abo", () => {
    const setup = (enabled: boolean) => {
      const gql = mockGraphql({
        PlanningProjects: () => ({
          projectsPage: { nodes: [], pageInfo: { hasNextPage: false, endCursor: null } },
        }),
        CalendarFeedEnabled: () => ({ calendarFeedEnabled: enabled }),
        CalendarFeedRotate: () => ({ calendarFeedRotate: "https://x.test/calendar/t.ics" }),
        CalendarFeedRevoke: () => ({ calendarFeedRevoke: true }),
      });
      return gql;
    };
    const open = async () => {
      const user = userEvent.setup();
      await renderWithApp(<MilestonesPage ws="acme" />, newClient(), "/acme/milestones");
      await user.click(await screen.findByRole("button", { name: "Calendar" }));
      return user;
    };

    it("legt den Link an und zeigt ihn einmal", async () => {
      const gql = setup(false);
      const user = await open();
      await user.click(await screen.findByRole("button", { name: "Create link" }));
      expect(await screen.findByLabelText("Calendar link")).toHaveValue(
        "https://x.test/calendar/t.ics",
      );
      expect(gql.of("CalendarFeedRotate")).toHaveLength(1);
    });

    it("zeigt einen aktiven Link nicht erneut und kann ihn abschalten", async () => {
      const gql = setup(true);
      const user = await open();
      expect(await screen.findByRole("button", { name: "Create new link" })).toBeInTheDocument();
      expect(screen.queryByLabelText("Calendar link")).toBeNull();
      await user.click(screen.getByRole("button", { name: "Turn off" }));
      await waitFor(() => expect(gql.of("CalendarFeedRevoke")).toHaveLength(1));
    });
  });

  it("sagt, wenn es keine offenen Meilensteine gibt", async () => {
    mockGraphql({
      PlanningProjects: () => ({
        projectsPage: { nodes: [], pageInfo: { hasNextPage: false, endCursor: null } },
      }),
    });
    await renderWithApp(<MilestonesPage ws="acme" />, newClient(), "/acme/milestones");
    expect(await screen.findByText("No open milestones.")).toBeInTheDocument();
  });
});
