import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { mockGraphql, newClient, renderWithApp } from "@/features/issues/test-fixtures";

import { NewTeamDialog } from "./new-team-dialog";
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
  memberCount: 1,
  ...over,
});

const TEAMS = [
  row({ id: "team-eng", key: "ENG", name: "Engineering" }),
  row({ id: "team-des", key: "DES", name: "Design", visibility: "private" }),
];

const handlers = () => ({
  SettingsTeams: () => ({ teams: TEAMS }),
  SettingsTeamCreate: () => ({ teamCreate: { success: true, team: { id: "team-kp", key: "KP" } } }),
});

async function open() {
  const onOpenChange = vi.fn();
  await renderWithApp(
    <NewTeamDialog open onOpenChange={onOpenChange} />,
    newClient(),
    "/acme/settings/teams",
  );
  return { user: userEvent.setup(), onOpenChange };
}

describe("NewTeamDialog", () => {
  it("suggests a key from the name until the key is edited", async () => {
    mockGraphql(handlers());
    const { user } = await open();
    const key = await screen.findByLabelText("Team identifier");
    await user.type(screen.getByLabelText("Team name"), "Customer Success Ops");
    expect(key).toHaveValue("CSO");
    await user.clear(key);
    await user.type(key, "kp");
    expect(key).toHaveValue("KP");
    await user.type(screen.getByLabelText("Team name"), " Extra");
    expect(key).toHaveValue("KP");
  });

  it("validates the key live and blocks submitting an invalid or reserved key", async () => {
    mockGraphql(handlers());
    const { user } = await open();
    await user.type(await screen.findByLabelText("Team name"), "Plus");
    const key = screen.getByLabelText("Team identifier");
    await user.clear(key);
    await user.type(key, "1X");
    expect(await screen.findByText(/starting with a letter/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Create team" })).toBeDisabled();
    await user.clear(key);
    await user.type(key, "P");
    expect(await screen.findByText(/reserved/)).toBeInTheDocument();
  });

  it("warns when the key is used by a visible team", async () => {
    mockGraphql(handlers());
    const { user } = await open();
    await screen.findByLabelText("Team identifier");
    await waitFor(() => expect(screen.getByLabelText("Parent team")).toBeInTheDocument());
    await user.type(screen.getByLabelText("Team name"), "Eng Two");
    const key = screen.getByLabelText("Team identifier");
    await user.clear(key);
    await user.type(key, "ENG");
    expect(await screen.findByText(/already used by another team/)).toBeInTheDocument();
  });

  it("creates the team with the entered values and closes", async () => {
    const gql = mockGraphql(handlers());
    const { user, onOpenChange } = await open();
    await user.type(await screen.findByLabelText("Team name"), "KollegePlus");
    const key = screen.getByLabelText("Team identifier");
    await user.clear(key);
    await user.type(key, "KP");
    await screen.findAllByRole("option", { name: "Design (DES)", hidden: true });
    await user.selectOptions(screen.getByLabelText("Copy settings from team"), "team-eng");
    await user.click(screen.getByRole("button", { name: "Create team" }));
    await waitFor(() => expect(gql.of("SettingsTeamCreate")).toHaveLength(1));
    expect(gql.of("SettingsTeamCreate")[0]!.variables).toEqual({
      input: {
        name: "KollegePlus",
        key: "KP",
        visibility: "public",
        copySettingsFromTeamId: "team-eng",
      },
    });
    await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false));
  });

  it("B5.13: a double click creates only one team", async () => {
    const gql = mockGraphql(handlers());
    const { user } = await open();
    await user.type(await screen.findByLabelText("Team name"), "Doppel");
    await user.dblClick(screen.getByRole("button", { name: "Create team" }));
    await waitFor(() => expect(gql.of("SettingsTeamCreate")).toHaveLength(1));
    await new Promise((r) => setTimeout(r, 50));
    expect(gql.of("SettingsTeamCreate")).toHaveLength(1);
  });

  it("shows the server's conflict error and stays open", async () => {
    const gql = mockGraphql({
      ...handlers(),
      SettingsTeamCreate: () => new Error("Team key KP is already in use"),
    });
    const { user, onOpenChange } = await open();
    await user.type(await screen.findByLabelText("Team name"), "KollegePlus");
    const key = screen.getByLabelText("Team identifier");
    await user.clear(key);
    await user.type(key, "KP");
    await user.click(screen.getByRole("button", { name: "Create team" }));
    expect(await screen.findByText("Team key KP is already in use")).toBeInTheDocument();
    expect(gql.of("SettingsTeamCreate")).toHaveLength(1);
    expect(onOpenChange).not.toHaveBeenCalled();
  });

  it("restricts visibility below a private parent", async () => {
    mockGraphql(handlers());
    const { user } = await open();
    await screen.findAllByRole("option", { name: "Design (DES)", hidden: true });
    await user.selectOptions(screen.getByLabelText("Parent team"), "team-des");
    const vis = screen.getByLabelText("Visibility") as HTMLSelectElement;
    expect([...vis.options].map((o) => o.value)).toEqual(["restricted", "private"]);
    expect(vis.value).toBe("restricted");
  });
});
