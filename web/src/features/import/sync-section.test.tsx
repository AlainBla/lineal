import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";
import { mockGraphql, renderWithApp } from "@/features/issues/test-fixtures";

import type { ImportRunView, ImportSyncView } from "./api";
import { minuteToTime, SyncSection, timeToMinute } from "./sync-section";

afterEach(() => vi.unstubAllGlobals());

const COMPLETED = {
  id: "run-1",
  source: "linear",
  status: "completed",
  phase: 8,
  step: null,
  dryRun: false,
  scan: null,
  options: { teams: ["ENG", "DES"] },
  stats: {},
  error: null,
  startedAt: "2026-10-10T08:00:00.000Z",
  finishedAt: "2026-10-10T08:10:00.000Z",
  createdAt: "2026-10-10T08:00:00.000Z",
} as ImportRunView;

const SYNC: ImportSyncView = {
  id: "sync-1",
  interval: "daily",
  dailyAtMinute: 185,
  paused: false,
  teams: ["ENG"],
  projects: [],
  hasKey: true,
  lastRunId: "run-9",
  lastRunAt: "2026-10-15T03:05:00.000Z",
  lastSuccessAt: "2026-10-15T03:05:00.000Z",
  lastStatus: "completed",
  lastError: null,
  nextRunAt: "2026-10-16T03:05:00.000Z",
};

describe("time helpers", () => {
  it("converts minutes since midnight UTC", () => {
    expect(minuteToTime(185)).toBe("03:05");
    expect(timeToMinute("03:05")).toBe(185);
    expect(timeToMinute("23:59")).toBe(1439);
    expect(timeToMinute("")).toBe(0);
  });
});

describe("SyncSection", () => {
  it("asks for a completed import first", async () => {
    mockGraphql({ ImportSync: () => ({ importSync: null }) });
    await renderWithApp(<SyncSection completedRun={null} onOpenRun={() => {}} />);
    expect(await screen.findByText(/Complete an import first/)).toBeInTheDocument();
  });

  it("sets up the sync with the key, interval and teams, and never shows the key again", async () => {
    let stored: ImportSyncView | null = null;
    const api = mockGraphql({
      ImportSync: () => ({ importSync: stored }),
      ImportSyncSave: (v) => {
        const input = (v as { input: { interval: ImportSyncView["interval"]; teams: string[] } })
          .input;
        stored = { ...SYNC, interval: input.interval, teams: input.teams, lastStatus: null };
        return { importSyncSave: stored };
      },
    });
    await renderWithApp(<SyncSection completedRun={COMPLETED} onOpenRun={() => {}} />);
    const user = userEvent.setup();
    const submit = await screen.findByRole("button", { name: "Set up sync" });
    // Ohne Schlüssel kein Speichern.
    expect(submit).toBeDisabled();
    await user.type(screen.getByLabelText("Linear API key for the sync"), "lin_api_SYNCKEY123456");
    await user.selectOptions(screen.getByLabelText("Sync interval"), "hourly");
    await user.click(screen.getByLabelText("Sync team DES"));
    await user.click(submit);
    await waitFor(() => expect(api.of("ImportSyncSave")).toHaveLength(1));
    expect(api.of("ImportSyncSave")[0]?.variables).toEqual({
      input: { interval: "hourly", teams: ["ENG"], token: "lin_api_SYNCKEY123456" },
    });
    expect(await screen.findByTestId("sync-state")).toHaveTextContent("Active");
    expect(document.body.innerHTML).not.toContain("SYNCKEY123456");
  });

  it("shows status, next run and the last result with a link to the report", async () => {
    const onOpen = vi.fn();
    mockGraphql({ ImportSync: () => ({ importSync: SYNC }) });
    await renderWithApp(<SyncSection completedRun={COMPLETED} onOpenRun={onOpen} />);
    expect(await screen.findByTestId("sync-interval")).toHaveTextContent("Daily at 03:05 UTC");
    expect(screen.getByTestId("sync-state")).toHaveTextContent("Active");
    expect(screen.getByTestId("sync-next")).not.toHaveTextContent("–");
    expect(screen.getByTestId("sync-last")).toHaveTextContent("Completed");
    await userEvent.setup().click(screen.getByRole("button", { name: "Show report" }));
    expect(onOpen).toHaveBeenCalledWith("run-9");
  });

  it("shows a failed run with its error", async () => {
    mockGraphql({
      ImportSync: () => ({
        importSync: {
          ...SYNC,
          lastStatus: "failed",
          lastError: "the source rejected the credentials",
        },
      }),
    });
    await renderWithApp(<SyncSection completedRun={COMPLETED} onOpenRun={() => {}} />);
    expect(await screen.findByTestId("sync-last")).toHaveTextContent("Failed");
    expect(screen.getByRole("alert")).toHaveTextContent("rejected the credentials");
  });

  it("runs now with the chosen options, pauses and deletes", async () => {
    let current: ImportSyncView | null = SYNC;
    const api = mockGraphql({
      ImportSync: () => ({ importSync: current }),
      ImportRuns: () => ({ importRuns: [] }),
      ImportSyncRun: () => ({
        importSyncRun: { ...COMPLETED, status: "running", options: { syncId: "sync-1" } },
      }),
      ImportSyncPause: (v) => {
        current = { ...SYNC, paused: (v as { paused: boolean }).paused };
        return { importSyncPause: current };
      },
      ImportSyncDelete: () => {
        current = null;
        return { importSyncDelete: true };
      },
    });
    vi.spyOn(window, "confirm").mockReturnValue(true);
    await renderWithApp(<SyncSection completedRun={COMPLETED} onOpenRun={() => {}} />);
    const user = userEvent.setup();
    await screen.findByTestId("sync-interval");
    await user.click(screen.getByLabelText(/Reload texts that contain placeholders/));
    await user.click(screen.getByLabelText(/Merge earlier duplicate labels/));
    await user.click(screen.getByRole("button", { name: "Sync now" }));
    await waitFor(() => expect(api.of("ImportSyncRun")).toHaveLength(1));
    expect(api.of("ImportSyncRun")[0]?.variables).toEqual({
      reloadContent: true,
      mergeLinearLabels: true,
    });
    await user.click(screen.getByRole("button", { name: "Pause" }));
    await waitFor(() => expect(screen.getByTestId("sync-state")).toHaveTextContent("Paused"));
    expect(api.of("ImportSyncPause")[0]?.variables).toEqual({ paused: true });
    expect(screen.getByTestId("sync-next")).toHaveTextContent("–");
    await user.click(screen.getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(api.of("ImportSyncDelete")).toHaveLength(1));
    expect(await screen.findByRole("button", { name: "Set up sync" })).toBeInTheDocument();
  });

  it("is hidden for people without permission", async () => {
    mockGraphql({
      ImportSync: () => Object.assign(new Error("no"), { code: "forbidden" }),
    });
    await renderWithApp(<SyncSection completedRun={COMPLETED} onOpenRun={() => {}} />);
    await waitFor(() => expect(screen.queryByText("Loading…")).not.toBeInTheDocument());
    expect(screen.queryByTestId("sync-section")).not.toBeInTheDocument();
  });
});
