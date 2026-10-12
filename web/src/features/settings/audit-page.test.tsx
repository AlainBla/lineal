import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { SESSION_QUERY_KEY } from "@/app/auth";
import { mockGraphql, newClient, renderWithApp, SESSION } from "@/features/issues/test-fixtures";

import {
  auditActionLabel,
  auditCredentialLabel,
  periodSince,
  relativeTime,
  type AuditEntry,
} from "./audit-queries";
import { AuditLogPage } from "./audit-page";

afterEach(() => vi.unstubAllGlobals());

const NOW = new Date("2026-10-12T12:00:00Z");

function entry(over: Partial<AuditEntry> & { id: string }): AuditEntry {
  return {
    occurredAt: "2026-10-12T11:55:00Z",
    actorKind: "user",
    actorId: "user-alice",
    actorName: "Alice",
    credentialKind: "session",
    action: "user.role_changed",
    targetType: "user",
    targetId: "user-bob-0000-1111",
    details: '{"from":"user","to":"admin"}',
    ip: "203.0.113.7",
    requestId: "req_1",
    ...over,
  };
}

const PAGE_1 = {
  nodes: [
    entry({ id: "e1", action: "user.role_changed", actorName: "Alice", ip: "203.0.113.7" }),
    entry({
      id: "e2",
      action: "auth.login_failed",
      actorName: "Bob",
      actorId: "user-bob",
      credentialKind: null,
      targetId: null,
      ip: null,
      occurredAt: "2026-10-12T09:00:00Z",
    }),
  ],
  pageInfo: { hasNextPage: true, endCursor: "cursor-1" },
};

const PAGE_2 = {
  nodes: [
    entry({
      id: "e3",
      action: "webhook.created",
      actorKind: "system",
      actorId: null,
      actorName: null,
      credentialKind: "api_key",
      occurredAt: "2026-10-01T09:00:00Z",
    }),
  ],
  pageInfo: { hasNextPage: false, endCursor: "cursor-2" },
};

const handlers = () => ({
  SettingsMemberList: () => ({
    users: [
      { id: "user-alice", name: "Alice", displayName: "alice" },
      { id: "user-bob", name: "Bob", displayName: "bob" },
    ],
  }),
  SettingsAuditLog: (v: Record<string, unknown>) => ({
    auditLog: v.after === "cursor-1" ? PAGE_2 : PAGE_1,
  }),
});

function clientWithRole(role: string) {
  const client = newClient();
  client.setQueryData(SESSION_QUERY_KEY, { ...SESSION, user: { ...SESSION.user, role } });
  return client;
}

describe("AuditLogPage", () => {
  it("shows labelled rows with actor, target, IP and an absolute time tooltip", async () => {
    mockGraphql(handlers());
    await renderWithApp(
      <AuditLogPage ws="acme" />,
      clientWithRole("owner"),
      "/acme/settings/audit",
    );
    const table = await screen.findByRole("table", { name: "Audit log" });
    const rows = within(table).getAllByRole("row").slice(1);
    expect(rows).toHaveLength(2);
    expect(within(rows[0]!).getByText("Role changed")).toBeInTheDocument();
    expect(within(rows[0]!).getByText("Alice")).toBeInTheDocument();
    expect(within(rows[0]!).getByText("203.0.113.7")).toBeInTheDocument();
    expect(within(rows[0]!).getByText("user · user-bob")).toBeInTheDocument();
    expect(within(rows[1]!).getByText("Failed sign-in attempt")).toBeInTheDocument();
    // Zeitpunkt: relative Angabe mit absoluter Angabe im Tooltip.
    const time = rows[0]!.querySelector("time");
    expect(time).not.toBeNull();
    expect(time).toHaveAttribute("dateTime", "2026-10-12T11:55:00Z");
    expect(time).toHaveAttribute("title", new Date("2026-10-12T11:55:00Z").toLocaleString());
    expect(screen.getByRole("link", { name: "Audit log" })).toHaveAttribute("aria-current", "page");
  });

  it("loads the next page with the cursor and appends the rows", async () => {
    const gql = mockGraphql(handlers());
    await renderWithApp(
      <AuditLogPage ws="acme" />,
      clientWithRole("owner"),
      "/acme/settings/audit",
    );
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Load more" }));
    await waitFor(() => expect(gql.of("SettingsAuditLog")).toHaveLength(2));
    expect(gql.of("SettingsAuditLog")[1]!.variables).toMatchObject({
      first: 50,
      after: "cursor-1",
    });
    const table = screen.getByRole("table", { name: "Audit log" });
    expect(await within(table).findByText("Webhook created")).toBeInTheDocument();
    expect(within(table).getByText("via API key")).toBeInTheDocument();
    expect(within(table).getByText("System")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Load more" })).not.toBeInTheDocument();
  });

  it("sends the chosen action, actor and period as filters", async () => {
    const gql = mockGraphql(handlers());
    await renderWithApp(
      <AuditLogPage ws="acme" />,
      clientWithRole("owner"),
      "/acme/settings/audit",
    );
    const user = userEvent.setup();
    await screen.findByRole("table", { name: "Audit log" });
    await user.selectOptions(screen.getByLabelText("Action"), "auth.login_failed");
    await waitFor(() =>
      expect(gql.of("SettingsAuditLog").at(-1)?.variables).toMatchObject({
        action: "auth.login_failed",
        actorId: null,
        since: null,
      }),
    );
    await screen.findByRole("option", { name: "Bob" });
    await user.selectOptions(screen.getByLabelText("Actor"), "user-bob");
    await user.selectOptions(screen.getByLabelText("Period"), "7d");
    await waitFor(() =>
      expect(gql.of("SettingsAuditLog").at(-1)?.variables).toMatchObject({
        action: "auth.login_failed",
        actorId: "user-bob",
      }),
    );
    const since = gql.of("SettingsAuditLog").at(-1)?.variables.since as string;
    expect(Math.abs(Date.now() - 7 * 86_400_000 - new Date(since).getTime())).toBeLessThan(60_000);
  });

  it("shows only a hint to members and does not query the log", async () => {
    const gql = mockGraphql(handlers());
    await renderWithApp(<AuditLogPage ws="acme" />, clientWithRole("user"), "/acme/settings/audit");
    expect(
      await screen.findByText("Only workspace owners and admins can view the audit log."),
    ).toBeInTheDocument();
    expect(gql.of("SettingsAuditLog")).toHaveLength(0);
    expect(screen.queryByRole("table", { name: "Audit log" })).not.toBeInTheDocument();
  });
});

describe("audit helpers", () => {
  it("labels known actions and keeps unknown ones visible", () => {
    expect(auditActionLabel("api_key.revoked")).toBe("API key revoked");
    expect(auditActionLabel("future.thing")).toBe("future.thing");
    expect(auditCredentialLabel("cli")).toBe("command line");
    expect(auditCredentialLabel(null)).toBeNull();
  });

  it("formats relative times in both directions", () => {
    expect(relativeTime("2026-10-12T11:55:00Z", NOW)).toBe("5 minutes ago");
    expect(relativeTime("2026-10-12T09:00:00Z", NOW)).toBe("3 hours ago");
    expect(relativeTime("2026-10-11T12:00:00Z", NOW)).toBe("yesterday");
    expect(relativeTime("2026-10-12T12:00:30Z", NOW)).toBe("in 30 seconds");
  });

  it("computes the period start and leaves 'all' open", () => {
    expect(periodSince("all", NOW)).toBeNull();
    expect(periodSince("24h", NOW)).toBe("2026-10-11T12:00:00.000Z");
    expect(periodSince("30d", NOW)).toBe("2026-09-12T12:00:00.000Z");
  });
});
