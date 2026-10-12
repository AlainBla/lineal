import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { SESSION_QUERY_KEY } from "@/app/auth";
import { mockGraphql, newClient, renderWithApp, SESSION } from "@/features/issues/test-fixtures";

import { SecuritySettingsPage } from "./security-page";
import { parseDomains, ROLE_QUESTIONS } from "./security-queries";

afterEach(() => vi.unstubAllGlobals());

const SETTINGS = {
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
  allowedInviteDomains: ["acme.example"],
};

function clientWithRole(role: string) {
  const client = newClient();
  client.setQueryData(SESSION_QUERY_KEY, { ...SESSION, user: { ...SESSION.user, role } });
  return client;
}

const handlers = () => ({
  SettingsSecurity: () => ({ workspaceSecuritySettings: SETTINGS }),
  SettingsSecurityUpdate: (v: Record<string, unknown>) => ({
    workspaceSecuritySettingsUpdate: {
      success: true,
      securitySettings: {
        ...SETTINGS,
        ...(v.input as Record<string, unknown>),
      },
    },
  }),
});

describe("parseDomains", () => {
  it("splits on commas, spaces and lines, strips @ and dedupes", () => {
    expect(parseDomains("acme.example, @Other.example\nACME.example  x.io;")).toEqual([
      "acme.example",
      "Other.example",
      "x.io",
    ]);
    expect(parseDomains("  ")).toEqual([]);
  });
});

describe("SecuritySettingsPage", () => {
  it("shows every right with its current role and the domain list", async () => {
    mockGraphql(handlers());
    await renderWithApp(
      <SecuritySettingsPage ws="acme" />,
      clientWithRole("owner"),
      "/acme/settings/security",
    );
    const select = await screen.findByLabelText("Who can invite members?");
    expect(select).toHaveValue("user");
    expect(Object.keys(ROLE_QUESTIONS)).toHaveLength(10);
    expect(screen.getByLabelText("Allowed invite domains")).toHaveValue("acme.example");
    expect(screen.getByRole("link", { name: "Security" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("button", { name: "Save…" })).toBeDisabled();
  });

  it("saves only the changed fields after confirmation", async () => {
    const gql = mockGraphql(handlers());
    await renderWithApp(
      <SecuritySettingsPage ws="acme" />,
      clientWithRole("owner"),
      "/acme/settings/security",
    );
    const user = userEvent.setup();
    await user.selectOptions(await screen.findByLabelText("Who can invite members?"), "admin");
    const domains = screen.getByLabelText("Allowed invite domains");
    await user.clear(domains);
    await user.type(domains, "acme.example, partner.example");
    await user.click(screen.getByRole("button", { name: "Save…" }));
    // Bestätigung zeigt die Änderungen; noch nichts gesendet.
    expect(await screen.findByText("Save security settings?")).toBeInTheDocument();
    expect(screen.getByRole("list", { name: "Changes" })).toHaveTextContent("Admins and owners");
    expect(gql.of("SettingsSecurityUpdate")).toHaveLength(0);
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() => expect(gql.of("SettingsSecurityUpdate")).toHaveLength(1));
    expect(gql.of("SettingsSecurityUpdate")[0]!.variables).toEqual({
      input: {
        invitationsRole: "admin",
        allowedInviteDomains: ["acme.example", "partner.example"],
      },
    });
    await waitFor(() =>
      expect(screen.queryByText("Save security settings?")).not.toBeInTheDocument(),
    );
  });

  it("shows server errors in the confirmation and keeps the draft", async () => {
    mockGraphql({
      ...handlers(),
      SettingsSecurityUpdate: () => new Error("`localhost` is not a valid domain"),
    });
    await renderWithApp(
      <SecuritySettingsPage ws="acme" />,
      clientWithRole("owner"),
      "/acme/settings/security",
    );
    const user = userEvent.setup();
    const domains = await screen.findByLabelText("Allowed invite domains");
    await user.clear(domains);
    await user.type(domains, "localhost");
    await user.click(screen.getByRole("button", { name: "Save…" }));
    await user.click(await screen.findByRole("button", { name: "Save changes" }));
    expect(await screen.findByText(/not a valid domain/)).toBeInTheDocument();
    expect(screen.getByLabelText("Allowed invite domains")).toHaveValue("localhost");
  });

  it("locks the admin-management right for admins who are not owners", async () => {
    mockGraphql(handlers());
    await renderWithApp(
      <SecuritySettingsPage ws="acme" />,
      clientWithRole("admin"),
      "/acme/settings/security",
    );
    expect(await screen.findByLabelText("Who can grant or remove the admin role?")).toBeDisabled();
    expect(screen.getByLabelText("Who can create teams?")).toBeEnabled();
  });

  it("shows only a note and does not query for regular members", async () => {
    const gql = mockGraphql(handlers());
    await renderWithApp(
      <SecuritySettingsPage ws="acme" />,
      clientWithRole("user"),
      "/acme/settings/security",
    );
    expect(await screen.findByText(/Only workspace owners and admins/)).toBeInTheDocument();
    expect(gql.of("SettingsSecurity")).toHaveLength(0);
    expect(screen.queryByRole("link", { name: "Security" })).not.toBeInTheDocument();
  });
});
