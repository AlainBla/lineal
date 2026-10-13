import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { mockGraphql, newClient, renderWithApp } from "@/features/issues/test-fixtures";

import { TrashPage } from "./trash-page";

afterEach(() => vi.unstubAllGlobals());

const ISSUE = {
  id: "i1",
  identifier: "ENG-7",
  title: "Alter Fehler",
  updatedAt: "2026-10-12T10:00:00Z",
};

describe("TrashPage (F.04)", () => {
  it("lists deleted issues of a team and restores one", async () => {
    let restored = false;
    const gql = mockGraphql({
      TrashIssues: () => ({ trashedIssues: restored ? [] : [ISSUE] }),
      TrashRestoreIssue: () => {
        restored = true;
        return { issueUnarchive: { success: true } };
      },
    });
    await renderWithApp(<TrashPage ws="acme" teamKey="ENG" />, newClient());
    const user = userEvent.setup();
    expect(await screen.findByText("Alter Fehler")).toBeInTheDocument();
    expect(gql.of("TrashIssues")[0]!.variables).toEqual({ team: "ENG" });
    await user.click(screen.getByRole("button", { name: "Restore Alter Fehler" }));
    await waitFor(() => expect(gql.of("TrashRestoreIssue")).toHaveLength(1));
    expect(gql.of("TrashRestoreIssue")[0]!.variables).toEqual({ id: "i1" });
    expect(await screen.findByText("No deleted issues")).toBeInTheDocument();
  });

  it("workspace page lists projects and initiatives and shows server errors", async () => {
    const forbidden = Object.assign(new Error("Not allowed"), { code: "forbidden" });
    const gql = mockGraphql({
      TrashProjects: () => ({
        trashedProjects: [
          { id: "p1", identifier: "P-ENG-1", name: "Mobile", updatedAt: "2026-10-12T10:00:00Z" },
        ],
      }),
      TrashRestoreProject: () => forbidden,
    });
    await renderWithApp(<TrashPage ws="acme" tab="projects" />, newClient());
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Restore Mobile" }));
    await waitFor(() => expect(gql.of("TrashRestoreProject")).toHaveLength(1));
    expect(await screen.findByRole("alert")).toHaveTextContent(/permission/i);
  });

  it("initiatives tab restores via initiativeDelete(restore)", async () => {
    const gql = mockGraphql({
      TrashInitiatives: () => ({
        trashedInitiatives: [
          { id: "n1", identifier: "I-1", name: "Wachstum", updatedAt: "2026-10-12T10:00:00Z" },
        ],
      }),
      TrashRestoreInitiative: () => ({ initiativeDelete: { success: true } }),
    });
    await renderWithApp(<TrashPage ws="acme" tab="initiatives" />, newClient());
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Restore Wachstum" }));
    await waitFor(() => expect(gql.of("TrashRestoreInitiative")).toHaveLength(1));
  });
});
