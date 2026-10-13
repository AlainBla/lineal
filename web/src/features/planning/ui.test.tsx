import { screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { mockGraphql, newClient, renderWithApp } from "@/features/issues/test-fixtures";
import type { Project } from "@/gql/graphql";

import { ProjectsPage } from "./projects-page";
import { cycleRange, dayInZone, shortDate } from "./ui";

afterEach(() => vi.unstubAllGlobals());

describe("shortDate (B5.12)", () => {
  it("zeigt Kalenderdaten unabhängig von der Browser-Zeitzone", () => {
    const old = process.env.TZ;
    process.env.TZ = "America/Los_Angeles";
    try {
      expect(shortDate("2026-12-31")).toBe("31 Dec 2026");
      expect(shortDate("2026-12-31T00:00:00Z")).toBe("31 Dec 2026");
    } finally {
      if (old === undefined) delete process.env.TZ;
      else process.env.TZ = old;
    }
  });

  it("zeigt die Auflösung statt des Tages", () => {
    expect(shortDate("2026-12-31", "quarter")).toBe("Q4 2026");
    expect(shortDate("2027-01-31", "quarter", 1)).toBe("Q4 2027");
  });

  it("dayInZone nutzt die angegebene Zeitzone (B5.20)", () => {
    expect(dayInZone("2026-10-04T22:00:00Z", "Europe/Berlin")).toBe("5 Oct");
    expect(dayInZone("2026-10-04T22:00:00Z", "America/Los_Angeles")).toBe("4 Oct");
  });
});

describe("cycleRange (B5.20)", () => {
  it("zeigt erster und letzter Tag in der Team-Zeitzone", () => {
    expect(cycleRange("2026-10-04T22:00:00Z", "2026-10-18T22:00:00Z", "Europe/Berlin")).toBe(
      "5 Oct – 18 Oct",
    );
  });
});

describe("Projektliste (B5.11)", () => {
  it("lädt alle Seiten nach", async () => {
    const mk = (n: number) =>
      ({
        id: `p-${n}`,
        identifier: `P-ENG-${n}`,
        name: `Projekt ${n}`,
        status: { id: "s", name: "Planned", type: "planned", color: "#000000" },
        progress: 0,
        teams: [],
        initiatives: [],
        labels: [],
        members: [],
        milestones: [],
        health: null,
        targetDate: null,
        targetDateResolution: null,
        archivedAt: null,
        leadTeam: null,
        lead: null,
      }) as unknown as Project;
    const gql = mockGraphql({
      PlanningProjects: (v: Record<string, unknown>) =>
        v.after
          ? {
              projectsPage: {
                nodes: [mk(2)],
                pageInfo: { hasNextPage: false, endCursor: null },
              },
            }
          : {
              projectsPage: {
                nodes: [mk(1)],
                pageInfo: { hasNextPage: true, endCursor: "c1" },
              },
            },
    });
    await renderWithApp(<ProjectsPage ws="acme" tab="all" />, newClient(), "/acme/projects/all");
    expect(await screen.findByText("Projekt 1")).toBeInTheDocument();
    expect(await screen.findByText("Projekt 2")).toBeInTheDocument();
    expect(gql.of("PlanningProjects")).toHaveLength(2);
  });
});
