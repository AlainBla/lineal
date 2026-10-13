import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { SESSION_QUERY_KEY } from "@/app/auth";
import { mockGraphql, newClient, renderWithApp, SESSION } from "@/features/issues/test-fixtures";

import { parseEmails } from "./invite-emails";
import { MembersSettingsPage } from "./members-page";

afterEach(() => vi.unstubAllGlobals());

const INV = {
  id: "inv-1",
  email: "dora@acme.example",
  role: "user",
  teamIds: ["team-eng"],
  inviterId: "user-bob",
  status: "pending",
  expiresAt: "2999-01-01T00:00:00Z",
  lastSentAt: "2026-10-01T00:00:00Z",
  createdAt: "2026-10-01T00:00:00Z",
};

const SECURITY = {
  teamCreationRole: "user",
  invitationsRole: "user",
  adminManagementRole: "admin",
  labelManagementRole: "user",
  templateManagementRole: "user",
  personalApiKeysRole: "user",
  apiSettingsRole: "admin",
  workspaceInitiativesRole: "user",
  pinnedViewManagementRole: "admin",
  importRole: "admin",
  allowedInviteDomains: [],
};

function member(id: string, displayName: string, email: string, role: string, status: string) {
  return {
    id,
    name: displayName,
    displayName,
    email,
    role,
    status,
    lastSeenAt: null,
    createdAt: "2026-10-01T00:00:00Z",
    teams: [],
  };
}

function clientWithRole(role: string, id = "user-bob") {
  const client = newClient();
  client.setQueryData(SESSION_QUERY_KEY, { ...SESSION, user: { ...SESSION.user, id, role } });
  return client;
}

const handlers = () => ({
  SettingsMemberList: () => ({
    users: [
      { id: "user-bob", name: "Bob", displayName: "bob" },
      { id: "user-carol", name: "Carol", displayName: "carol" },
    ],
  }),
  SettingsAdminMembers: () => ({
    workspaceMembers: [
      member("user-alice", "alice", "alice@acme.example", "owner", "active"),
      member("user-bob", "bob", "bob@acme.example", "user", "active"),
      member("user-carol", "carol", "carol@acme.example", "guest", "suspended"),
    ],
  }),
  SettingsSecurity: () => ({
    workspaceSecuritySettings: { ...SECURITY, allowedInviteDomains: ["acme.example"] },
  }),
  SettingsUserRoleUpdate: () => ({
    userRoleUpdate: { success: true, member: { id: "x", role: "admin" } },
  }),
  SettingsUserSuspend: () => ({
    userSuspend: { success: true, member: { id: "x", status: "suspended" } },
  }),
  SettingsUserUnsuspend: () => ({
    userUnsuspend: { success: true, member: { id: "x", status: "active" } },
  }),
  SettingsInvitations: () => ({ invitations: [INV] }),
  SettingsInviteUsers: () => ({ inviteUsers: { success: true, invitations: [{ id: "inv-2" }] } }),
  SettingsInvitationResend: () => ({ invitationResend: { success: true } }),
  SettingsInvitationRevoke: () => ({ invitationRevoke: { success: true } }),
});

describe("parseEmails", () => {
  it("splits, dedupes case-insensitively and reports invalid entries", () => {
    expect(parseEmails("a@acme.example, B@acme.example;\n a@ACME.example nope")).toEqual({
      emails: ["a@acme.example", "B@acme.example"],
      invalid: ["nope"],
    });
  });
});

describe("MembersSettingsPage", () => {
  it("lists members and pending invitations with teams and status", async () => {
    mockGraphql(handlers());
    await renderWithApp(
      <MembersSettingsPage ws="acme" />,
      clientWithRole("admin"),
      "/acme/settings/members",
    );
    expect(await screen.findByText("carol@acme.example")).toBeInTheDocument();
    expect(await screen.findByText("dora@acme.example")).toBeInTheDocument();
    expect(screen.getByText(/Teams: Engineering/)).toBeInTheDocument();
    expect(screen.getByLabelText("Status of dora@acme.example")).toHaveTextContent("pending");
    expect(screen.getByRole("link", { name: "Members" })).toHaveAttribute("aria-current", "page");
  });

  it("resends and revokes an invitation", async () => {
    const gql = mockGraphql(handlers());
    await renderWithApp(
      <MembersSettingsPage ws="acme" />,
      clientWithRole("admin"),
      "/acme/settings/members",
    );
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: /Resend invitation to dora/ }));
    await waitFor(() => expect(gql.of("SettingsInvitationResend")).toHaveLength(1));
    expect(gql.of("SettingsInvitationResend")[0]!.variables).toEqual({ id: "inv-1" });
    await user.click(screen.getByRole("button", { name: /Revoke invitation to dora/ }));
    await waitFor(() => expect(gql.of("SettingsInvitationRevoke")).toHaveLength(1));
  });

  it("invites several addresses with role and teams", async () => {
    const gql = mockGraphql(handlers());
    await renderWithApp(
      <MembersSettingsPage ws="acme" />,
      clientWithRole("admin"),
      "/acme/settings/members",
    );
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: /Invite people/ }));
    await user.type(
      screen.getByLabelText("Email addresses"),
      "erik@acme.example, fay@acme.example",
    );
    await user.selectOptions(screen.getByLabelText("Role"), "admin");
    await user.click(screen.getByRole("checkbox", { name: /Engineering/ }));
    await user.click(screen.getByRole("button", { name: "Send invitations" }));
    await waitFor(() => expect(gql.of("SettingsInviteUsers")).toHaveLength(1));
    expect(gql.of("SettingsInviteUsers")[0]!.variables).toEqual({
      input: {
        emails: ["erik@acme.example", "fay@acme.example"],
        role: "admin",
        teamIds: ["team-eng"],
      },
    });
  });

  it("B5.14: the invite dialog scrolls internally so the actions stay reachable", async () => {
    mockGraphql(handlers());
    await renderWithApp(
      <MembersSettingsPage ws="acme" />,
      clientWithRole("admin"),
      "/acme/settings/members",
    );
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: /Invite people/ }));
    expect(screen.getByRole("dialog").className).toContain("max-h-[90vh]");
    expect(screen.getByTestId("invite-teams").className).toContain("overflow-y-auto");
    expect(screen.getByRole("form", { name: "Invite people" }).className).toContain(
      "overflow-y-auto",
    );
  });

  it("validates addresses client-side and shows server errors", async () => {
    const h = handlers();
    const gql = mockGraphql({
      ...h,
      SettingsInviteUsers: () => new Error("A user with this email already exists"),
    });
    await renderWithApp(
      <MembersSettingsPage ws="acme" />,
      clientWithRole("user"),
      "/acme/settings/members",
    );
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: /Invite people/ }));
    // Admin-Rolle ist für normale Mitglieder nicht wählbar.
    expect(screen.queryByRole("option", { name: "Admin" })).not.toBeInTheDocument();
    await user.type(screen.getByLabelText("Email addresses"), "kaputt");
    await user.click(screen.getByRole("button", { name: "Send invitations" }));
    expect(await screen.findByText(/Not a valid email address: kaputt/)).toBeInTheDocument();
    expect(gql.of("SettingsInviteUsers")).toHaveLength(0);
    await user.clear(screen.getByLabelText("Email addresses"));
    await user.type(screen.getByLabelText("Email addresses"), "bob@acme.example");
    await user.click(screen.getByRole("button", { name: "Send invitations" }));
    expect(await screen.findByText(/already exists/)).toBeInTheDocument();
  });

  it("requires a team for guests", async () => {
    const gql = mockGraphql(handlers());
    await renderWithApp(
      <MembersSettingsPage ws="acme" />,
      clientWithRole("admin"),
      "/acme/settings/members",
    );
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: /Invite people/ }));
    await user.type(screen.getByLabelText("Email addresses"), "gast@acme.example");
    await user.selectOptions(screen.getByLabelText("Role"), "guest");
    await user.click(screen.getByRole("button", { name: "Send invitations" }));
    expect(await screen.findByText(/at least one team/)).toBeInTheDocument();
    expect(gql.of("SettingsInviteUsers")).toHaveLength(0);
  });

  it("hides invitation management for guests without querying it", async () => {
    const gql = mockGraphql(handlers());
    await renderWithApp(
      <MembersSettingsPage ws="acme" />,
      clientWithRole("guest"),
      "/acme/settings/members",
    );
    expect(await screen.findByText(/do not have permission to invite/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Invite people/ })).not.toBeInTheDocument();
    expect(gql.of("SettingsInvitations")).toHaveLength(0);
  });

  it("shows the no-permission note when the server answers forbidden", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async (_u: unknown, init?: RequestInit) => {
        const q = String((JSON.parse(String(init?.body)) as { query: string }).query);
        if (q.includes("SettingsInvitations"))
          return new Response(
            JSON.stringify({
              errors: [{ message: "Forbidden", extensions: { code: "forbidden" } }],
            }),
          );
        return new Response(JSON.stringify({ data: { users: [] } }));
      }),
    );
    await renderWithApp(
      <MembersSettingsPage ws="acme" />,
      clientWithRole("user"),
      "/acme/settings/members",
    );
    expect(await screen.findByText(/do not have permission to invite/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Invite people/ })).not.toBeInTheDocument();
  });

  it("shows plain names without emails to non-admins and never queries the admin list", async () => {
    const gql = mockGraphql(handlers());
    await renderWithApp(
      <MembersSettingsPage ws="acme" />,
      clientWithRole("user"),
      "/acme/settings/members",
    );
    expect(await screen.findByText("carol")).toBeInTheDocument();
    expect(screen.queryByText("carol@acme.example")).not.toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "Security" })).not.toBeInTheDocument();
    expect(gql.of("SettingsAdminMembers")).toHaveLength(0);
  });

  it("shows email, role and status to admins and changes a role", async () => {
    const gql = mockGraphql(handlers());
    await renderWithApp(
      <MembersSettingsPage ws="acme" />,
      clientWithRole("owner", "user-alice"),
      "/acme/settings/members",
    );
    expect(await screen.findByText("bob@acme.example")).toBeInTheDocument();
    expect(screen.getByLabelText("Status of carol@acme.example")).toHaveTextContent("suspended");
    expect(screen.getByRole("link", { name: "Security" })).toBeInTheDocument();
    // Eigene Rolle ist gesperrt.
    expect(screen.getByLabelText("Role of alice@acme.example")).toBeDisabled();
    const user = userEvent.setup();
    await user.selectOptions(screen.getByLabelText("Role of bob@acme.example"), "admin");
    await waitFor(() => expect(gql.of("SettingsUserRoleUpdate")).toHaveLength(1));
    expect(gql.of("SettingsUserRoleUpdate")[0]!.variables).toEqual({
      userId: "user-bob",
      role: "admin",
    });
  });

  it("only owners can pick the owner role; admins cannot touch owners", async () => {
    mockGraphql(handlers());
    await renderWithApp(
      <MembersSettingsPage ws="acme" />,
      clientWithRole("admin"),
      "/acme/settings/members",
    );
    await screen.findByText("bob@acme.example");
    const bobRole = screen.getByLabelText("Role of bob@acme.example");
    expect(within(bobRole).queryByRole("option", { name: "Owner" })).not.toBeInTheDocument();
    expect(screen.getByLabelText("Role of alice@acme.example")).toBeDisabled();
    expect(screen.getByRole("button", { name: "Suspend alice@acme.example" })).toBeDisabled();
  });

  it("suspends after confirmation, unsuspends directly and shows server errors", async () => {
    const gql = mockGraphql(handlers());
    await renderWithApp(
      <MembersSettingsPage ws="acme" />,
      clientWithRole("owner", "user-alice"),
      "/acme/settings/members",
    );
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Suspend bob@acme.example" }));
    expect(gql.of("SettingsUserSuspend")).toHaveLength(0);
    await user.click(await screen.findByRole("button", { name: "Suspend member" }));
    await waitFor(() => expect(gql.of("SettingsUserSuspend")).toHaveLength(1));
    expect(gql.of("SettingsUserSuspend")[0]!.variables).toEqual({ userId: "user-bob" });
    await user.click(screen.getByRole("button", { name: "Unsuspend carol@acme.example" }));
    await waitFor(() => expect(gql.of("SettingsUserUnsuspend")).toHaveLength(1));
  });

  it("shows the server error when a role change is rejected", async () => {
    mockGraphql({
      ...handlers(),
      SettingsUserRoleUpdate: () => new Error("The last owner cannot be demoted"),
    });
    await renderWithApp(
      <MembersSettingsPage ws="acme" />,
      clientWithRole("owner", "user-alice"),
      "/acme/settings/members",
    );
    const user = userEvent.setup();
    await user.selectOptions(await screen.findByLabelText("Role of bob@acme.example"), "admin");
    expect(await screen.findByText(/last owner cannot be demoted/)).toBeInTheDocument();
  });

  it("hints at the allowed invite domains in the invite dialog", async () => {
    mockGraphql(handlers());
    await renderWithApp(
      <MembersSettingsPage ws="acme" />,
      clientWithRole("user"),
      "/acme/settings/members",
    );
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: /Invite people/ }));
    expect(await screen.findByTestId("invite-domains-hint")).toHaveTextContent("acme.example");
  });
});
