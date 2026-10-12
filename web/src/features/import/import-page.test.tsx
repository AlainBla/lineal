import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";
import { mockGraphql, renderWithApp } from "@/features/issues/test-fixtures";

import { progressPercent } from "./api";
import { ImportPage } from "./import-page";

const TARGETS = { teams: [{ id: "kp-id", key: "KP", name: "Kolleg Plus" }] };

const PLANNED = {
  id: "run-1",
  source: "linear",
  status: "planned",
  phase: null,
  step: null,
  dryRun: false,
  scan: {
    workspace: { name: "Acme" },
    teams: [
      {
        id: "t1",
        key: "ENG",
        name: "Engineering",
        private: false,
        parentId: null,
        issueCount: 110,
        conflict: true,
        projects: [
          { id: "p1", name: "Checkout", status: "In Progress", issueCount: 40 },
          { id: "p2", name: "API v2", status: "Planned", issueCount: 25 },
        ],
      },
      {
        id: "t2",
        key: "DES",
        name: "Design",
        private: true,
        parentId: null,
        issueCount: 20,
        conflict: false,
      },
    ],
    users: [
      {
        id: "u1",
        name: "Alice",
        displayName: "alice",
        email: "alice@acme.test",
        admin: true,
        guest: false,
        matchedUserId: "x",
      },
      {
        id: "u2",
        name: "Dave",
        displayName: "dave",
        email: "dave@acme.test",
        admin: false,
        guest: false,
        matchedUserId: null,
      },
    ],
    counts: { Issue: 130, Team: 2 },
    conflicts: [{ key: "ENG", reason: "key_exists" }],
  },
  stats: {},
  error: null,
  startedAt: null,
  finishedAt: null,
  createdAt: "2026-10-10T08:00:00.000Z",
};

afterEach(() => vi.unstubAllGlobals());

describe("progressPercent", () => {
  it("maps phases to a percentage", () => {
    expect(progressPercent({ status: "completed", phase: 8 })).toBe(100);
    expect(progressPercent({ status: "running", phase: null })).toBe(0);
    expect(progressPercent({ status: "running", phase: 5 })).toBe(50);
  });
});

describe("ImportPage", () => {
  it("sends the token once and never keeps it in the page", async () => {
    const api = mockGraphql({
      ImportRuns: () => ({ importRuns: [] }),
      ImportScan: () => ({ importScan: { ...PLANNED, status: "scanning", scan: null } }),
      ImportRun: () => ({ importRun: { ...PLANNED, status: "scanning", scan: null } }),
    });
    await renderWithApp(<ImportPage />);
    const user = userEvent.setup();
    const input = await screen.findByLabelText("Linear API key");
    expect(input).toHaveAttribute("type", "password");
    await user.type(input, "lin_api_TESTKEY123456");
    await user.click(screen.getByRole("button", { name: "Check connection" }));
    await waitFor(() => expect(api.of("ImportScan")).toHaveLength(1));
    expect(api.of("ImportScan")[0]?.variables).toEqual({
      input: { source: "linear", token: "lin_api_TESTKEY123456" },
    });
    expect(await screen.findByText(/Reading teams/)).toBeInTheDocument();
    expect(document.body.innerHTML).not.toContain("TESTKEY123456");
  });

  it("requires a new key for conflicting teams and starts a dry run with the key mapping", async () => {
    const api = mockGraphql({
      ImportRuns: () => ({ importRuns: [PLANNED] }),
      ImportRun: () => ({ importRun: PLANNED }),
      ImportStart: () => ({ importStart: { ...PLANNED, status: "running", phase: 1 } }),
      ImportTargetTeams: () => TARGETS,
    });
    await renderWithApp(<ImportPage />);
    const user = userEvent.setup();
    expect(await screen.findByText("Engineering")).toBeInTheDocument();
    expect(screen.getByText("private, stays private")).toBeInTheDocument();
    expect(screen.getByText(/2 people found, 1 matched/)).toBeInTheDocument();
    const dry = screen.getByRole("button", { name: "Dry run" });
    expect(dry).toBeDisabled();
    expect(screen.getByRole("alert")).toHaveTextContent("Team key conflict");
    await user.type(screen.getByLabelText("Target key for ENG"), "engx");
    expect(dry).toBeEnabled();
    await user.click(dry);
    await waitFor(() => expect(api.of("ImportStart")).toHaveLength(1));
    expect(api.of("ImportStart")[0]?.variables).toEqual({
      input: {
        runId: "run-1",
        teams: ["ENG", "DES"],
        keyMapping: { ENG: "ENGX" },
        dryRun: true,
        delta: false,
        projects: [],
        targetTeams: {},
      },
    });
  });

  it("imports into an existing team and only the checked projects", async () => {
    const api = mockGraphql({
      ImportRuns: () => ({ importRuns: [PLANNED] }),
      ImportRun: () => ({ importRun: PLANNED }),
      ImportStart: () => ({ importStart: { ...PLANNED, status: "running", phase: 1 } }),
      ImportTargetTeams: () => TARGETS,
    });
    await renderWithApp(<ImportPage />);
    const user = userEvent.setup();
    await screen.findByText("Engineering");
    // Ziel: bestehendes Team statt neuem Team; der Key-Konflikt entfällt damit.
    const target = screen.getByLabelText("Target for ENG");
    expect(target).toHaveDisplayValue("As new team (key ENG)");
    await waitFor(() => expect(screen.getAllByRole("option")).toHaveLength(3 + 1));
    await user.selectOptions(target, "kp-id");
    expect(screen.queryByLabelText("Target key for ENG")).not.toBeInTheDocument();
    expect(screen.queryByText(/Team key conflict/)).not.toBeInTheDocument();
    // Projekte als Checkliste, Standard: alle.
    const checkout = screen.getByLabelText("Import project Checkout of ENG");
    expect(checkout).toBeChecked();
    await user.click(screen.getByLabelText("Import project API v2 of ENG"));
    await user.click(screen.getByRole("button", { name: "Start import" }));
    await waitFor(() => expect(api.of("ImportStart")).toHaveLength(1));
    expect(api.of("ImportStart")[0]?.variables).toEqual({
      input: {
        runId: "run-1",
        teams: ["ENG", "DES"],
        keyMapping: {},
        dryRun: false,
        delta: false,
        projects: ["p1"],
        targetTeams: { ENG: "kp-id" },
      },
    });
  });

  it("shows the planned mappings and warnings of a dry run", async () => {
    const dry = {
      ...PLANNED,
      dryRun: true,
      stats: {
        entities: {
          Issue: { created: 40, updated: 0, unchanged: 0, skipped: 0, conflicts: 0, errors: 0 },
          Comment: { created: 7, updated: 0, unchanged: 0, skipped: 0, conflicts: 0, errors: 0 },
        },
        plan: {
          statesMapped: 2,
          statesNew: 11,
          labelsNew: 14,
          usersMatched: 1,
          usersPlaceholder: 2,
          relationsOutside: 8,
        },
      },
    };
    mockGraphql({
      ImportRuns: () => ({ importRuns: [dry] }),
      ImportRun: () => ({ importRun: dry }),
      ImportTargetTeams: () => TARGETS,
      ImportReport: () => ({
        importReport: {
          infoCount: 3,
          warningCount: 1,
          errorCount: 0,
          entries: [
            {
              severity: "warning",
              sourceType: "IssueRelation",
              sourceId: "r",
              message: "the related issue KOL-9 is not part of this import",
            },
          ],
        },
      }),
    });
    await renderWithApp(<ImportPage />);
    const plan = await screen.findByTestId("plan-report");
    expect(plan).toHaveTextContent("Statuses mapped to existing statuses2");
    expect(plan).toHaveTextContent("Statuses to be created11");
    expect(plan).toHaveTextContent("People created as placeholders2");
    expect(plan).toHaveTextContent("Relations to issues outside the import (kept as links)8");
    expect(plan).toHaveTextContent("Issues planned40");
    expect(await screen.findByText(/KOL-9 is not part of this import/)).toBeInTheDocument();
  });

  it("shows the report of a finished run", async () => {
    const done = {
      ...PLANNED,
      status: "completed",
      stats: {
        entities: {
          Issue: { created: 130, updated: 0, unchanged: 0, skipped: 0, conflicts: 0, errors: 0 },
        },
      },
    };
    mockGraphql({
      ImportRuns: () => ({ importRuns: [done] }),
      ImportRun: () => ({ importRun: done }),
      ImportReport: () => ({
        importReport: {
          infoCount: 1,
          warningCount: 1,
          errorCount: 0,
          entries: [
            {
              severity: "warning",
              sourceType: "Attachment",
              sourceId: "a",
              message: "file could not be downloaded",
            },
          ],
        },
      }),
    });
    await renderWithApp(<ImportPage />);
    expect(await screen.findByText("file could not be downloaded")).toBeInTheDocument();
    expect(screen.getByTestId("run-status")).toHaveTextContent("Completed");
    expect(screen.getByText("130")).toBeInTheDocument();
  });

  it("explains missing permissions", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(
            JSON.stringify({ errors: [{ message: "no", extensions: { code: "forbidden" } }] }),
            { status: 200 },
          ),
      ),
    );
    await renderWithApp(<ImportPage />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Only workspace admins");
  });
});
