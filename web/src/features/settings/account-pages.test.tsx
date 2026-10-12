import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { mockGraphql, newClient, renderWithApp } from "@/features/issues/test-fixtures";
import { getTheme, resetThemeState } from "@/lib/theme";

import { AccountAppearancePage } from "./account-appearance";
import { AccountNotificationsPage } from "./account-notifications";
import { AccountProfilePage } from "./account-profile";
import { avatarProblem, describeAgent, type Viewer } from "./account-queries";
import { AccountSecurityPage } from "./account-security";

afterEach(() => {
  vi.unstubAllGlobals();
  localStorage.clear();
  resetThemeState();
});

const VIEWER: Viewer = {
  id: "user-bob",
  name: "Bob Example",
  displayName: "bob",
  email: "bob@acme.example",
  role: "user",
  avatarUrl: null,
  timezone: null,
  createdAt: "2026-01-01T00:00:00Z",
};

describe("helpers", () => {
  it("describes user agents", () => {
    expect(
      describeAgent(
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36",
      ),
    ).toBe("Chrome on Linux");
    expect(describeAgent("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) Firefox/130.0")).toBe(
      "Firefox on macOS",
    );
    expect(describeAgent(null)).toBe("Unknown device");
  });

  it("rejects non-images and files over 5 MB", () => {
    expect(avatarProblem({ type: "image/png", size: 1000 })).toBeNull();
    expect(avatarProblem({ type: "application/pdf", size: 1000 })).toMatch(/PNG/);
    expect(avatarProblem({ type: "image/png", size: 6 * 1024 * 1024 })).toMatch(/5 MB/);
  });
});

describe("AccountProfilePage", () => {
  const handlers = (viewer: Viewer = VIEWER) => ({
    AccountViewer: () => ({ viewer }),
    AccountViewerUpdate: (v: Record<string, unknown>) => ({
      viewerUpdate: { viewer: { ...viewer, ...(v.input as object) } },
    }),
  });
  const render = () =>
    renderWithApp(<AccountProfilePage ws="acme" />, newClient(), "/acme/settings/account/profile");

  it("shows the Account group in the navigation", async () => {
    mockGraphql(handlers());
    await render();
    await screen.findByLabelText("Full name");
    const nav = screen.getByRole("navigation", { name: "Settings sections" });
    expect(within(nav).getByText("Account")).toBeInTheDocument();
    expect(within(nav).getByRole("link", { name: "Profile" })).toHaveAttribute(
      "aria-current",
      "page",
    );
    for (const label of ["Password & sessions", "Notifications", "Appearance"]) {
      expect(within(nav).getByRole("link", { name: label })).toBeInTheDocument();
    }
  });

  it("saves only the changed fields and clears the time zone with null", async () => {
    const gql = mockGraphql(handlers({ ...VIEWER, timezone: "Europe/Berlin" }));
    await render();
    const user = userEvent.setup();
    const name = await screen.findByLabelText("Full name");
    expect(screen.getByLabelText("Email")).toBeDisabled();
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    await user.clear(name);
    await user.type(name, "Robert Example");
    await user.selectOptions(screen.getByLabelText("Time zone"), "");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(gql.of("AccountViewerUpdate")).toHaveLength(1));
    expect(gql.of("AccountViewerUpdate")[0]!.variables).toEqual({
      input: { name: "Robert Example", timezone: null },
    });
  });

  it("validates the display name before saving", async () => {
    const gql = mockGraphql(handlers());
    await render();
    const user = userEvent.setup();
    const handle = await screen.findByLabelText("Display name");
    await user.clear(handle);
    await user.type(handle, "Bad Name");
    expect(screen.getByRole("alert")).toHaveTextContent("a-z");
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    expect(gql.of("AccountViewerUpdate")).toHaveLength(0);
  });

  it("shows server errors, for example a taken display name", async () => {
    mockGraphql({
      ...handlers(),
      AccountViewerUpdate: () => new Error("This display name is already taken"),
    });
    await render();
    const user = userEvent.setup();
    const handle = await screen.findByLabelText("Display name");
    await user.clear(handle);
    await user.type(handle, "alice");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText(/already taken/)).toBeInTheDocument();
  });

  it("uploads an avatar through fileUpload and the signed PUT", async () => {
    const gql = mockGraphql({
      ...handlers(),
      AccountFileUpload: () => ({
        fileUpload: {
          uploadFile: {
            uploadUrl: "http://lineal.test/uploads/abc",
            assetUrl: "http://lineal.test/assets/org/abc/me.png",
            headers: [{ key: "content-type", value: "image/png" }],
          },
        },
      }),
    });
    const puts: Array<{ url: string; headers: unknown }> = [];
    const base = globalThis.fetch;
    vi.stubGlobal("fetch", (input: RequestInfo | URL, init?: RequestInit) => {
      if (init?.method === "PUT") {
        puts.push({ url: String(input), headers: init.headers });
        return Promise.resolve(new Response(null, { status: 200 }));
      }
      return base(input, init);
    });
    await render();
    const user = userEvent.setup();
    const input = await screen.findByLabelText("Upload profile picture");
    await user.upload(input, new File(["x"], "me.png", { type: "image/png" }));
    await waitFor(() => expect(gql.of("AccountViewerUpdate")).toHaveLength(1));
    expect(puts).toEqual([
      { url: "http://lineal.test/uploads/abc", headers: { "content-type": "image/png" } },
    ]);
    expect(gql.of("AccountViewerUpdate")[0]!.variables).toEqual({
      input: { avatarUrl: "http://lineal.test/assets/org/abc/me.png" },
    });
  });

  it("refuses wrong file types without uploading", async () => {
    const gql = mockGraphql(handlers());
    await render();
    const user = userEvent.setup({ applyAccept: false });
    const input = await screen.findByLabelText("Upload profile picture");
    await user.upload(input, new File(["x"], "doc.pdf", { type: "application/pdf" }));
    expect(await screen.findByText(/PNG, JPEG, GIF or WebP image\./)).toBeInTheDocument();
    expect(gql.of("AccountFileUpload")).toHaveLength(0);
  });

  it("removes the avatar with an explicit null", async () => {
    const gql = mockGraphql(handlers({ ...VIEWER, avatarUrl: "http://x.test/a.png" }));
    await render();
    await userEvent.click(await screen.findByRole("button", { name: "Remove" }));
    await waitFor(() => expect(gql.of("AccountViewerUpdate")).toHaveLength(1));
    expect(gql.of("AccountViewerUpdate")[0]!.variables).toEqual({ input: { avatarUrl: null } });
  });
});

const SESSIONS = [
  {
    id: "s1",
    userAgent: "Mozilla/5.0 (X11; Linux x86_64) Chrome/126.0 Safari/537.36",
    ipAddress: "203.0.113.7",
    createdAt: "2026-10-12T08:00:00Z",
    lastUsedAt: null,
    expiresAt: "2026-10-26T08:00:00Z",
    current: true,
  },
  {
    id: "s2",
    userAgent: "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0) Safari/604.1",
    ipAddress: null,
    createdAt: "2026-10-10T08:00:00Z",
    lastUsedAt: null,
    expiresAt: "2026-10-24T08:00:00Z",
    current: false,
  },
];

describe("AccountSecurityPage", () => {
  const render = () =>
    renderWithApp(
      <AccountSecurityPage ws="acme" />,
      newClient(),
      "/acme/settings/account/security",
    );

  it("changes the password after local checks and reports the signed-out sessions", async () => {
    const gql = mockGraphql({
      AccountSessions: () => ({ sessions: SESSIONS }),
      AccountPasswordChange: () => ({ passwordChange: { success: true, revokedSessions: 1 } }),
    });
    await render();
    const user = userEvent.setup();
    const submit = await screen.findByRole("button", { name: "Change password" });
    await user.type(screen.getByLabelText("Current password"), "old-password");
    await user.type(screen.getByLabelText("New password"), "short");
    expect(screen.getByRole("alert")).toHaveTextContent("at least 8");
    expect(submit).toBeDisabled();
    await user.clear(screen.getByLabelText("New password"));
    await user.type(screen.getByLabelText("New password"), "brand-new-pw");
    await user.type(screen.getByLabelText("Repeat new password"), "brand-new-px");
    expect(screen.getByText("The passwords do not match.")).toBeInTheDocument();
    expect(submit).toBeDisabled();
    await user.clear(screen.getByLabelText("Repeat new password"));
    await user.type(screen.getByLabelText("Repeat new password"), "brand-new-pw");
    await user.click(submit);
    await waitFor(() => expect(gql.of("AccountPasswordChange")).toHaveLength(1));
    expect(gql.of("AccountPasswordChange")[0]!.variables).toEqual({
      currentPassword: "old-password",
      newPassword: "brand-new-pw",
    });
    // Felder werden geleert.
    await waitFor(() => expect(screen.getByLabelText("Current password")).toHaveValue(""));
  });

  it("shows the server error for a wrong current password and keeps the form", async () => {
    mockGraphql({
      AccountSessions: () => ({ sessions: SESSIONS }),
      AccountPasswordChange: () => new Error("The current password is incorrect."),
    });
    await render();
    const user = userEvent.setup();
    await user.type(await screen.findByLabelText("Current password"), "nope-nope");
    await user.type(screen.getByLabelText("New password"), "brand-new-pw");
    await user.type(screen.getByLabelText("Repeat new password"), "brand-new-pw");
    await user.click(screen.getByRole("button", { name: "Change password" }));
    expect(await screen.findByText("The current password is incorrect.")).toBeInTheDocument();
    expect(screen.getByLabelText("New password")).toHaveValue("brand-new-pw");
  });

  it("lists sessions, marks this device and signs out one or all others", async () => {
    const gql = mockGraphql({
      AccountSessions: () => ({ sessions: SESSIONS }),
      AccountSessionRevoke: () => ({ sessionRevoke: { success: true } }),
      AccountSessionsRevokeOthers: () => ({ sessionsRevokeOthers: { count: 1 } }),
    });
    await render();
    const list = await screen.findByRole("list", { name: "Active sessions" });
    const items = within(list).getAllByRole("listitem");
    expect(items).toHaveLength(2);
    expect(items[0]).toHaveTextContent("Chrome on Linux");
    expect(items[0]).toHaveTextContent("This device");
    expect(within(items[0]!).queryByRole("button")).not.toBeInTheDocument();
    expect(items[1]).toHaveTextContent("Safari on iOS");
    const user = userEvent.setup();
    await user.click(within(items[1]!).getByRole("button", { name: /Sign out/ }));
    await waitFor(() => expect(gql.of("AccountSessionRevoke")).toHaveLength(1));
    expect(gql.of("AccountSessionRevoke")[0]!.variables).toEqual({ id: "s2" });
    await user.click(screen.getByRole("button", { name: "Sign out all other sessions" }));
    await waitFor(() => expect(gql.of("AccountSessionsRevokeOthers")).toHaveLength(1));
  });

  it("disables signing out others when there are none", async () => {
    mockGraphql({ AccountSessions: () => ({ sessions: [SESSIONS[0]] }) });
    await render();
    expect(
      await screen.findByRole("button", { name: "Sign out all other sessions" }),
    ).toBeDisabled();
  });
});

const SETTINGS = {
  emailEnabled: true,
  emailFormat: "digest",
  emailOnlyPriority: false,
  emailHoldOutsideWorkHours: true,
  emailImmediateUrgent: true,
  dueDateReminders: false,
  priorityCategories: ["assignments", "mentions"],
  channels: [{ category: "triage", channel: "email", enabled: true }],
};

describe("AccountNotificationsPage", () => {
  const handlers = () => ({
    AccountNotificationSettings: () => ({ notificationSettings: SETTINGS }),
    AccountNotificationSettingsUpdate: (v: Record<string, unknown>) => ({
      notificationSettingsUpdate: { ...SETTINGS, ...(v.input as object) },
    }),
    AccountNotificationSubscriptions: () => ({
      notificationSubscriptions: [
        {
          id: "sub1",
          targetType: "team",
          targetId: "team-eng",
          contextViewType: "active",
          notificationSubscriptionTypes: ["issueNewComment"],
          includeSubInitiativeUpdates: false,
          active: true,
        },
      ],
    }),
    AccountNotificationSubscriptionUpdate: () => ({
      notificationSubscriptionUpdate: { id: "sub1", active: false },
    }),
  });
  const render = () =>
    renderWithApp(
      <AccountNotificationsPage ws="acme" />,
      newClient(),
      "/acme/settings/account/notifications",
    );

  it("shows the inbox as always on and email with the documented defaults", async () => {
    mockGraphql(handlers());
    await render();
    const table = await screen.findByRole("table", { name: "Notification channels" });
    expect(within(table).getAllByRole("row")).toHaveLength(12); // Kopf + 11 Kategorien
    expect(within(table).getByLabelText("Assignments in-app")).toBeDisabled();
    expect(within(table).getByLabelText("Assignments in-app")).toBeChecked();
    expect(within(table).getByLabelText("Assignments email")).toBeChecked();
    expect(within(table).getByLabelText("Mentions email")).toBeChecked();
    expect(within(table).getByLabelText("Reactions email")).not.toBeChecked();
    expect(within(table).getByLabelText("Triage email")).toBeChecked(); // gespeicherte Zeile
  });

  it("saves a single email channel change", async () => {
    const gql = mockGraphql(handlers());
    await render();
    const user = userEvent.setup();
    await user.click(await screen.findByLabelText("Reactions email"));
    await waitFor(() => expect(gql.of("AccountNotificationSettingsUpdate")).toHaveLength(1));
    expect(gql.of("AccountNotificationSettingsUpdate")[0]!.variables).toEqual({
      input: { channels: [{ category: "reactions", channel: "email", enabled: true }] },
    });
    await waitFor(() => expect(screen.getByLabelText("Reactions email")).toBeChecked());
  });

  it("changes the email format and priority categories", async () => {
    const gql = mockGraphql(handlers());
    await render();
    const user = userEvent.setup();
    await user.selectOptions(await screen.findByLabelText("Email format"), "immediate");
    await waitFor(() => expect(gql.of("AccountNotificationSettingsUpdate")).toHaveLength(1));
    expect(gql.of("AccountNotificationSettingsUpdate")[0]!.variables).toEqual({
      input: { emailFormat: "immediate" },
    });
    const group = screen.getByRole("group", { name: "Priority categories" });
    await user.click(within(group).getByLabelText("Triage"));
    await waitFor(() => expect(gql.of("AccountNotificationSettingsUpdate")).toHaveLength(2));
    expect(gql.of("AccountNotificationSettingsUpdate")[1]!.variables).toEqual({
      input: { priorityCategories: ["assignments", "mentions", "triage"] },
    });
  });

  it("disables the email controls when email is switched off", async () => {
    mockGraphql({
      ...handlers(),
      AccountNotificationSettings: () => ({
        notificationSettings: { ...SETTINGS, emailEnabled: false },
      }),
    });
    await render();
    expect(await screen.findByLabelText("Mentions email")).toBeDisabled();
    expect(screen.getByLabelText("Email format")).toBeDisabled();
  });

  it("keeps the other fields when a subscription is switched off", async () => {
    const gql = mockGraphql(handlers());
    await render();
    const list = await screen.findByRole("list", { name: "Subscriptions" });
    expect(list).toHaveTextContent("Engineering");
    await userEvent.click(within(list).getByLabelText("Notify me about Engineering"));
    await waitFor(() => expect(gql.of("AccountNotificationSubscriptionUpdate")).toHaveLength(1));
    expect(gql.of("AccountNotificationSubscriptionUpdate")[0]!.variables).toEqual({
      input: {
        targetType: "team",
        targetId: "team-eng",
        contextViewType: "active",
        notificationSubscriptionTypes: ["issueNewComment"],
        includeSubInitiativeUpdates: false,
        active: false,
      },
    });
  });
});

describe("AccountAppearancePage", () => {
  it("switches the theme", async () => {
    await renderWithApp(
      <AccountAppearancePage ws="acme" />,
      newClient(),
      "/acme/settings/account/appearance",
    );
    const user = userEvent.setup();
    expect(screen.getByLabelText(/^Dark/)).toBeChecked();
    await user.click(screen.getByLabelText(/^Light/));
    expect(getTheme()).toBe("light");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
    await user.click(screen.getByLabelText(/^System/));
    expect(getTheme()).toBe("system");
    expect(localStorage.getItem("lineal-theme")).toBe("system");
  });
});
