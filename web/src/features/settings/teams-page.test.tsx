import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { getUiState, setUiState } from "@/app/ui-state";
import { mockGraphql, newClient, renderWithApp, SESSION } from "@/features/issues/test-fixtures";

import { orderTeams, TeamsSettingsPage } from "./teams-page";
import type { TeamRow } from "./teams-queries";

afterEach(() => {
  vi.unstubAllGlobals();
  setUiState({ createTeamOpen: false });
});

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

const TEAMS = [
  row({ id: "team-web", key: "WEB", name: "Web", parentId: "team-eng" }),
  row({ id: "team-eng", key: "ENG", name: "Engineering", memberCount: 1 }),
  row({ id: "team-des", key: "DES", name: "Design", visibility: "private", memberCount: null }),
  row({ id: "team-old", key: "OLD", name: "Legacy", retiredAt: "2026-03-01T00:00:00Z" }),
];

describe("orderTeams", () => {
  it("puts sub-teams directly below their parent", () => {
    expect(orderTeams(TEAMS).map((r) => [r.team.key, r.depth])).toEqual([
      ["DES", 0],
      ["ENG", 0],
      ["WEB", 1],
      ["OLD", 0],
    ]);
  });
});

describe("TeamsSettingsPage", () => {
  it("lists visible teams with key, parent, visibility, members and status", async () => {
    mockGraphql({ SettingsTeams: () => ({ teams: TEAMS }) });
    await renderWithApp(<TeamsSettingsPage ws="acme" />, newClient(), "/acme/settings/teams");
    const list = await screen.findByRole("list", { name: "Teams" });
    const items = within(list).getAllByRole("listitem");
    expect(items).toHaveLength(4);
    const eng = items.find((i) => within(i).queryByRole("link", { name: "Engineering" }))!;
    expect(within(eng).getByRole("link", { name: "Engineering" })).toHaveAttribute(
      "href",
      "/acme/settings/teams/ENG/general",
    );
    expect(within(eng).getByText("1 member")).toBeInTheDocument();
    expect(within(eng).getByText("Active")).toBeInTheDocument();
    const web = items.find((i) => within(i).queryByRole("link", { name: "Web" }))!;
    expect(within(web).getByText(/in Engineering/)).toBeInTheDocument();
    const des = items.find((i) => within(i).queryByRole("link", { name: "Design" }))!;
    expect(within(des).getByText("private")).toBeInTheDocument();
    expect(within(des).getByLabelText("Members of Design")).toHaveTextContent("–");
    const old = items.find((i) => within(i).queryByRole("link", { name: "Legacy" }))!;
    expect(within(old).getByText("Retired")).toBeInTheDocument();
  });

  it("opens the new-team dialog from the button", async () => {
    mockGraphql({ SettingsTeams: () => ({ teams: TEAMS }) });
    await renderWithApp(<TeamsSettingsPage ws="acme" />, newClient(), "/acme/settings/teams");
    await userEvent.click(await screen.findByRole("button", { name: "New team" }));
    expect(getUiState().createTeamOpen).toBe(true);
  });

  it("hides the button for guests and shows load errors", async () => {
    mockGraphql({ SettingsTeams: () => new Error("boom") });
    const client = newClient();
    client.setQueryData(["session"], {
      ...SESSION,
      user: { ...SESSION.user, role: "guest" },
    });
    await renderWithApp(<TeamsSettingsPage ws="acme" />, client, "/acme/settings/teams");
    expect(await screen.findByText("Could not load teams")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "New team" })).not.toBeInTheDocument();
  });
});
