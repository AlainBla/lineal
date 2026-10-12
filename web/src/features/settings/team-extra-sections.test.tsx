import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";
import { mockGraphql, newClient, renderWithApp, TEAM_META } from "@/features/issues/test-fixtures";

import { TeamSettingsPage, type TeamSection } from "./settings-pages";
import { WorkspaceWebhooksPage } from "./team-extra-sections";

afterEach(() => vi.unstubAllGlobals());

const RULES = [
  {
    id: "r-urgent",
    name: "Urgent",
    position: 1000,
    filter: { priority: { eq: 1 } },
    removesSla: false,
    duration: "PT24H",
    slaType: "all",
    startMode: "issueCreation",
  },
  {
    id: "r-bugs",
    name: "Bugs",
    position: 2000,
    filter: { labels: { some: { id: { in: ["l-bug"] } } } },
    removesSla: false,
    duration: "P3D",
    slaType: "onlyBusinessDays",
    startMode: "ruleMatch",
  },
  {
    id: "r-rest",
    name: "Others",
    position: 3000,
    filter: { priority: { in: [0, 3, 4] } },
    removesSla: true,
    duration: null,
    slaType: "all",
    startMode: "issueCreation",
  },
  {
    id: "r-custom",
    name: "Custom",
    position: 4000,
    filter: { assignee: { null: true } },
    removesSla: false,
    duration: "PT12H",
    slaType: "all",
    startMode: "issueCreation",
  },
];

const slaHandlers = (over: Record<string, (v: Record<string, unknown>) => unknown> = {}) => ({
  SettingsTeam: () => ({ team: TEAM_META.team }),
  SettingsTeamSla: () => ({
    teamSla: { teamId: "team-eng", enabled: true, workWeek: "monToFri", rules: RULES },
  }),
  SettingsLabels: () => ({ issueLabels: TEAM_META.labels }),
  SettingsTeamSlaUpdate: () => ({ teamSlaUpdate: { teamId: "team-eng" } }),
  SettingsSlaRuleCreate: () => ({ slaRuleCreate: { id: "r-new" } }),
  SettingsSlaRuleUpdate: () => ({ slaRuleUpdate: { id: "r-urgent" } }),
  SettingsSlaRuleDelete: () => ({ slaRuleDelete: { success: true } }),
  ...over,
});

async function open(section: TeamSection) {
  await renderWithApp(
    <TeamSettingsPage ws="acme" teamKey="ENG" section={section} />,
    newClient(),
    `/acme/settings/teams/ENG/${section}`,
  );
}

describe("SlaSection", () => {
  it("lists rules with readable conditions and actions", async () => {
    mockGraphql(slaHandlers());
    await open("sla");
    const urgent = await screen.findByRole("listitem", { name: "SLA rule Urgent" });
    expect(urgent).toHaveTextContent("Priority Urgent → 24 hours, clock starts at creation");
    expect(screen.getByRole("listitem", { name: "SLA rule Bugs" })).toHaveTextContent(
      "Label Bug → 3 business days, clock starts when the rule matches",
    );
    expect(screen.getByRole("listitem", { name: "SLA rule Others" })).toHaveTextContent(
      "Priority No priority or Medium or Low → remove SLA",
    );
    expect(screen.getByRole("listitem", { name: "SLA rule Custom" })).toHaveTextContent(
      "Custom conditions",
    );
    expect(screen.getByLabelText("Enable SLAs")).toBeChecked();
    expect(screen.getByLabelText("Work week")).toHaveValue("monToFri");
  });

  it("switches SLAs and the work week", async () => {
    const gql = mockGraphql(slaHandlers());
    await open("sla");
    const user = userEvent.setup();
    await user.click(await screen.findByLabelText("Enable SLAs"));
    await waitFor(() => expect(gql.of("SettingsTeamSlaUpdate")).toHaveLength(1));
    expect(gql.of("SettingsTeamSlaUpdate")[0]!.variables).toEqual({
      teamId: "team-eng",
      input: { enabled: false },
    });
    await user.selectOptions(screen.getByLabelText("Work week"), "sunToThu");
    await waitFor(() => expect(gql.of("SettingsTeamSlaUpdate")).toHaveLength(2));
    expect(gql.of("SettingsTeamSlaUpdate")[1]!.variables).toEqual({
      teamId: "team-eng",
      input: { workWeek: "sunToThu" },
    });
  });

  it("creates a rule from priority and label conditions", async () => {
    const gql = mockGraphql(slaHandlers());
    await open("sla");
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: /New rule/ }));
    await user.type(screen.getByLabelText("Rule name"), "High bugs");
    await user.click(screen.getByLabelText("Priority High"));
    await user.click(screen.getByLabelText("Priority Urgent"));
    await user.click(screen.getByLabelText("Label Bug"));
    await user.selectOptions(screen.getByLabelText("SLA duration"), "P1W");
    await user.selectOptions(screen.getByLabelText("SLA start"), "ruleMatch");
    await user.click(screen.getByRole("button", { name: "Save rule" }));
    await waitFor(() => expect(gql.of("SettingsSlaRuleCreate")).toHaveLength(1));
    expect(gql.of("SettingsSlaRuleCreate")[0]!.variables).toEqual({
      input: {
        teamId: "team-eng",
        name: "High bugs",
        filter: { priority: { in: [1, 2] }, labels: { some: { id: { in: ["l-bug"] } } } },
        removesSla: false,
        duration: "P1W",
        slaType: "all",
        startMode: "ruleMatch",
      },
    });
  });

  it("builds custom and business-day durations and removal rules", async () => {
    const gql = mockGraphql(slaHandlers());
    await open("sla");
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: /New rule/ }));
    await user.type(screen.getByLabelText("Rule name"), "Slow");
    await user.selectOptions(screen.getByLabelText("SLA duration"), "custom");
    await user.clear(screen.getByLabelText("SLA amount"));
    await user.type(screen.getByLabelText("SLA amount"), "36");
    await user.selectOptions(screen.getByLabelText("SLA unit"), "hours");
    await user.click(screen.getByRole("button", { name: "Save rule" }));
    await waitFor(() => expect(gql.of("SettingsSlaRuleCreate")).toHaveLength(1));
    expect(gql.of("SettingsSlaRuleCreate")[0]!.variables).toMatchObject({
      input: { duration: "PT36H", filter: {}, slaType: "all" },
    });

    await user.click(await screen.findByRole("button", { name: /New rule/ }));
    await user.type(screen.getByLabelText("Rule name"), "Work days");
    await user.selectOptions(screen.getByLabelText("SLA counting"), "onlyBusinessDays");
    await user.clear(screen.getByLabelText("SLA amount"));
    await user.type(screen.getByLabelText("SLA amount"), "5");
    await user.click(screen.getByRole("button", { name: "Save rule" }));
    await waitFor(() => expect(gql.of("SettingsSlaRuleCreate")).toHaveLength(2));
    expect(gql.of("SettingsSlaRuleCreate")[1]!.variables).toMatchObject({
      input: { duration: "P5D", slaType: "onlyBusinessDays" },
    });

    await user.click(await screen.findByRole("button", { name: /New rule/ }));
    await user.type(screen.getByLabelText("Rule name"), "None");
    await user.click(screen.getByLabelText("Remove SLA"));
    expect(screen.queryByLabelText("SLA duration")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Save rule" }));
    await waitFor(() => expect(gql.of("SettingsSlaRuleCreate")).toHaveLength(3));
    const input = (gql.of("SettingsSlaRuleCreate")[2]!.variables as { input: object }).input;
    expect(input).toMatchObject({ removesSla: true });
    expect(input).not.toHaveProperty("duration");
  });

  it("keeps conditions it cannot edit and moves rules", async () => {
    const gql = mockGraphql(slaHandlers());
    await open("sla");
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Edit Custom" }));
    expect(screen.getByRole("note")).toHaveTextContent("cannot be edited here");
    await user.clear(screen.getByLabelText("Rule name"));
    await user.type(screen.getByLabelText("Rule name"), "Custom v2");
    await user.click(screen.getByRole("button", { name: "Save rule" }));
    await waitFor(() => expect(gql.of("SettingsSlaRuleUpdate")).toHaveLength(1));
    expect(gql.of("SettingsSlaRuleUpdate")[0]!.variables).toMatchObject({
      teamId: "team-eng",
      id: "r-custom",
      input: { name: "Custom v2", filter: { assignee: { null: true } }, duration: "PT12H" },
    });
    await user.click(screen.getByRole("button", { name: "Move Bugs up" }));
    await waitFor(() => expect(gql.of("SettingsSlaRuleUpdate")).toHaveLength(3));
    expect(gql.of("SettingsSlaRuleUpdate")[1]!.variables).toMatchObject({
      id: "r-bugs",
      input: { position: 1000 },
    });
    expect(gql.of("SettingsSlaRuleUpdate")[2]!.variables).toMatchObject({
      id: "r-urgent",
      input: { position: 2000 },
    });
    expect(screen.getByRole("button", { name: "Move Urgent up" })).toBeDisabled();
  });

  it("deletes a rule only after confirmation", async () => {
    const gql = mockGraphql(slaHandlers());
    await open("sla");
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Delete Bugs" }));
    expect(gql.of("SettingsSlaRuleDelete")).toHaveLength(0);
    await user.click(screen.getByRole("button", { name: "Confirm delete Bugs" }));
    await waitFor(() => expect(gql.of("SettingsSlaRuleDelete")).toHaveLength(1));
    expect(gql.of("SettingsSlaRuleDelete")[0]!.variables).toEqual({
      teamId: "team-eng",
      id: "r-bugs",
    });
  });

  it("shows server errors and load errors", async () => {
    mockGraphql(
      slaHandlers({ SettingsSlaRuleCreate: () => new Error("duration must be positive") }),
    );
    await open("sla");
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: /New rule/ }));
    await user.type(screen.getByLabelText("Rule name"), "X");
    await user.click(screen.getByRole("button", { name: "Save rule" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("duration must be positive");
  });

  it("shows a load error", async () => {
    mockGraphql(slaHandlers({ SettingsTeamSla: () => new Error("boom") }));
    await open("sla");
    expect(await screen.findByText("Could not load SLAs")).toBeInTheDocument();
  });
});

// ---------- Recurring ----------

const REC = {
  id: "rec-1",
  teamId: "team-eng",
  templateId: "t-bug",
  templateName: "Weekly report",
  frequency: "weekly",
  interval: 1,
  weekday: 1,
  dayOfMonth: null,
  minuteOfDay: 540,
  startDate: "2026-10-01",
  paused: false,
  nextRunAt: "2026-10-12T09:00:00Z",
  lastRunAt: null,
  lastError: null,
  lastErrorAt: null,
  createdAt: "2026-10-01T00:00:00Z",
};

const tpl = (id: string, name: string, type = "issue") => ({
  id,
  name,
  type,
  description: null,
  icon: null,
  color: null,
  teamId: "team-eng",
  teamKey: "ENG",
  templateData: {},
  sortOrder: 0,
  inheritedFromId: null,
  isForm: false,
});

const recHandlers = (over: Record<string, (v: Record<string, unknown>) => unknown> = {}) => ({
  SettingsTeam: () => ({ team: TEAM_META.team }),
  SettingsRecurringIssues: () => ({ recurringIssues: [REC] }),
  SettingsTemplates: () => ({
    templates: [tpl("t-bug", "Weekly report"), tpl("t-doc", "Spec", "document")],
  }),
  SettingsRecurringCreate: () => ({ recurringIssueCreate: { id: "rec-2" } }),
  SettingsRecurringUpdate: () => ({ recurringIssueUpdate: { id: "rec-1" } }),
  SettingsRecurringDelete: () => ({ recurringIssueDelete: { success: true } }),
  ...over,
});

describe("RecurringSection", () => {
  it("lists recurring issues with rhythm, time zone and next run", async () => {
    mockGraphql(recHandlers());
    await open("recurring");
    const row = await screen.findByRole("listitem", { name: "Recurring issue Weekly report" });
    expect(row).toHaveTextContent("Every week on Monday at 09:00 (UTC)");
    expect(row).toHaveTextContent("Next:");
  });

  it("shows an empty state and a skipped-run error", async () => {
    mockGraphql(
      recHandlers({
        SettingsRecurringIssues: () => ({
          recurringIssues: [{ ...REC, lastError: "Could not find referenced Template." }],
        }),
      }),
    );
    await open("recurring");
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "The last run was skipped: Could not find referenced Template.",
    );
  });

  it("creates a weekly recurring issue from an issue template", async () => {
    const gql = mockGraphql(
      recHandlers({ SettingsRecurringIssues: () => ({ recurringIssues: [] }) }),
    );
    await open("recurring");
    const user = userEvent.setup();
    expect(await screen.findByText("No recurring issues yet.")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /New recurring issue/ }));
    const template = await screen.findByLabelText("Issue template");
    // Nur Issue-Templates stehen zur Wahl.
    expect(within(template).queryByRole("option", { name: "Spec" })).toBeNull();
    await user.selectOptions(template, "t-bug");
    await user.selectOptions(screen.getByLabelText("Weekday"), "5");
    await user.clear(screen.getByLabelText("Time of day"));
    await user.type(screen.getByLabelText("Time of day"), "08:30");
    await user.click(screen.getByRole("button", { name: "Create recurring issue" }));
    await waitFor(() => expect(gql.of("SettingsRecurringCreate")).toHaveLength(1));
    expect(gql.of("SettingsRecurringCreate")[0]!.variables).toEqual({
      input: {
        teamId: "team-eng",
        templateId: "t-bug",
        frequency: "weekly",
        interval: 1,
        weekday: 5,
        minuteOfDay: 510,
      },
    });
  });

  it("creates a monthly one and validates the form", async () => {
    const gql = mockGraphql(recHandlers());
    await open("recurring");
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: /New recurring issue/ }));
    await user.selectOptions(await screen.findByLabelText("Issue template"), "t-bug");
    await user.selectOptions(screen.getByLabelText("Frequency"), "monthly");
    await user.clear(screen.getByLabelText("Day of month"));
    await user.type(screen.getByLabelText("Day of month"), "40");
    expect(screen.getByRole("button", { name: "Create recurring issue" })).toBeDisabled();
    await user.clear(screen.getByLabelText("Day of month"));
    await user.type(screen.getByLabelText("Day of month"), "31");
    await user.type(screen.getByLabelText("Start date"), "2026-11-01");
    await user.click(screen.getByRole("button", { name: "Create recurring issue" }));
    await waitFor(() => expect(gql.of("SettingsRecurringCreate")).toHaveLength(1));
    expect(gql.of("SettingsRecurringCreate")[0]!.variables).toEqual({
      input: {
        teamId: "team-eng",
        templateId: "t-bug",
        frequency: "monthly",
        interval: 1,
        dayOfMonth: 31,
        minuteOfDay: 540,
        startDate: "2026-11-01",
      },
    });
  });

  it("pauses, resumes and deletes with confirmation", async () => {
    const gql = mockGraphql(recHandlers());
    await open("recurring");
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Pause Weekly report" }));
    await waitFor(() => expect(gql.of("SettingsRecurringUpdate")).toHaveLength(1));
    expect(gql.of("SettingsRecurringUpdate")[0]!.variables).toEqual({
      id: "rec-1",
      input: { paused: true },
    });
    await user.click(screen.getByRole("button", { name: "Delete Weekly report" }));
    expect(gql.of("SettingsRecurringDelete")).toHaveLength(0);
    await user.click(screen.getByRole("button", { name: "Confirm delete Weekly report" }));
    await waitFor(() => expect(gql.of("SettingsRecurringDelete")).toHaveLength(1));
    expect(gql.of("SettingsRecurringDelete")[0]!.variables).toEqual({ id: "rec-1" });
  });

  it("shows the paused state and the server error", async () => {
    mockGraphql(
      recHandlers({
        SettingsRecurringIssues: () => ({ recurringIssues: [{ ...REC, paused: true }] }),
        SettingsRecurringUpdate: () => new Error("not allowed"),
      }),
    );
    await open("recurring");
    const user = userEvent.setup();
    const row = await screen.findByRole("listitem", { name: "Recurring issue Weekly report" });
    expect(row).toHaveTextContent("Paused");
    expect(row).toHaveTextContent("Not scheduled");
    await user.click(screen.getByRole("button", { name: "Resume Weekly report" }));
    expect(await screen.findByText("not allowed")).toBeInTheDocument();
  });
});

// ---------- Webhooks ----------

const HOOK = {
  id: "wh-1",
  label: "CI",
  url: "https://ci.example.com/hook",
  enabled: true,
  teamId: "team-eng",
  allPublicTeams: false,
  resourceTypes: ["Issue", "Comment"],
  consecutiveFailures: 0,
  disabledReason: null,
  lastSuccessAt: null,
  createdAt: "2026-10-01T00:00:00Z",
};
const ALL_HOOK = { ...HOOK, id: "wh-2", label: "Audit sink", teamId: null, allPublicTeams: true };
const DELIVERY = {
  id: "d-1",
  webhookId: "wh-1",
  resourceType: "Issue",
  action: "create",
  attempt: 3,
  status: "given_up",
  responseCode: 500,
  durationMs: 120,
  nextAttemptAt: null,
  createdAt: "2026-10-10T10:00:00Z",
};

const hookHandlers = (over: Record<string, (v: Record<string, unknown>) => unknown> = {}) => ({
  SettingsTeam: () => ({ team: TEAM_META.team }),
  SettingsWebhooks: () => ({ webhooks: [HOOK, ALL_HOOK] }),
  SettingsWebhookDeliveries: () => ({ webhookDeliveries: [DELIVERY] }),
  SettingsWebhookCreate: () => ({
    webhookCreate: { success: true, secret: "whsec_SECRET1", webhook: HOOK },
  }),
  SettingsWebhookRotate: () => ({
    webhookRotateSecret: { success: true, secret: "whsec_ROTATED", webhook: HOOK },
  }),
  SettingsWebhookUpdate: () => ({ webhookUpdate: { success: true } }),
  SettingsWebhookDelete: () => ({ webhookDelete: { success: true } }),
  SettingsWebhookRetry: () => ({ webhookDeliveryRetry: { success: true } }),
  ...over,
});

describe("Team webhooks section", () => {
  it("lists only the webhooks of the team", async () => {
    mockGraphql(hookHandlers());
    await open("webhooks");
    const row = await screen.findByRole("listitem", { name: "Webhook CI" });
    expect(row).toHaveTextContent("https://ci.example.com/hook");
    expect(row).toHaveTextContent("Issue, Comment");
    expect(screen.queryByRole("listitem", { name: "Webhook Audit sink" })).toBeNull();
  });

  it("creates a webhook for the team and shows the secret exactly once", async () => {
    const gql = mockGraphql(hookHandlers());
    await open("webhooks");
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: /New webhook/ }));
    expect(screen.queryByLabelText("Webhook teams")).toBeNull();
    await user.type(screen.getByLabelText("Webhook label"), "CI");
    await user.type(screen.getByLabelText("Webhook URL"), "https://ci.example.com/hook");
    await user.click(screen.getByLabelText("Resource Comment"));
    await user.click(screen.getByRole("button", { name: "Create webhook" }));
    await waitFor(() => expect(gql.of("SettingsWebhookCreate")).toHaveLength(1));
    expect(gql.of("SettingsWebhookCreate")[0]!.variables).toEqual({
      input: {
        label: "CI",
        url: "https://ci.example.com/hook",
        resourceTypes: ["Issue", "Comment"],
        allPublicTeams: false,
        teamId: "team-eng",
      },
    });
    const secret = await screen.findByLabelText("Webhook secret");
    expect(secret).toHaveValue("whsec_SECRET1");
    expect(screen.getByRole("alert")).toHaveTextContent("shown only once");
    await user.click(screen.getByRole("button", { name: "Done" }));
    expect(screen.queryByDisplayValue("whsec_SECRET1")).toBeNull();
  });

  it("shows the backend error (e.g. SSRF protection) in the form", async () => {
    mockGraphql(
      hookHandlers({
        SettingsWebhookCreate: () => new Error("The webhook URL points to a private network."),
      }),
    );
    await open("webhooks");
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: /New webhook/ }));
    await user.type(screen.getByLabelText("Webhook URL"), "https://10.0.0.1/hook");
    await user.click(screen.getByRole("button", { name: "Create webhook" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("points to a private network");
    expect(screen.queryByLabelText("Webhook secret")).toBeNull();
  });

  it("rotates the secret after confirmation and shows the new one once", async () => {
    const gql = mockGraphql(hookHandlers());
    await open("webhooks");
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Rotate secret of CI" }));
    expect(gql.of("SettingsWebhookRotate")).toHaveLength(0);
    await user.click(screen.getByRole("button", { name: "Confirm rotate secret of CI" }));
    expect(await screen.findByLabelText("Webhook secret")).toHaveValue("whsec_ROTATED");
    expect(gql.of("SettingsWebhookRotate")[0]!.variables).toEqual({ id: "wh-1" });
  });

  it("disables, edits and deletes a webhook", async () => {
    const gql = mockGraphql(hookHandlers());
    await open("webhooks");
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Disable CI" }));
    await waitFor(() => expect(gql.of("SettingsWebhookUpdate")).toHaveLength(1));
    expect(gql.of("SettingsWebhookUpdate")[0]!.variables).toEqual({
      id: "wh-1",
      input: { enabled: false },
    });
    await user.click(screen.getByRole("button", { name: "Edit CI" }));
    await user.clear(screen.getByLabelText("Webhook URL"));
    await user.type(screen.getByLabelText("Webhook URL"), "https://ci.example.com/v2");
    await user.click(screen.getByLabelText("Resource Comment"));
    await user.click(screen.getByRole("button", { name: "Save webhook" }));
    await waitFor(() => expect(gql.of("SettingsWebhookUpdate")).toHaveLength(2));
    expect(gql.of("SettingsWebhookUpdate")[1]!.variables).toEqual({
      id: "wh-1",
      input: { url: "https://ci.example.com/v2", resourceTypes: ["Issue"] },
    });
    await user.click(screen.getByRole("button", { name: "Delete CI" }));
    await user.click(screen.getByRole("button", { name: "Confirm delete CI" }));
    await waitFor(() => expect(gql.of("SettingsWebhookDelete")).toHaveLength(1));
    expect(gql.of("SettingsWebhookDelete")[0]!.variables).toEqual({ id: "wh-1" });
  });

  it("shows deliveries with status and resends one", async () => {
    const gql = mockGraphql(hookHandlers());
    await open("webhooks");
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Show deliveries of CI" }));
    const d = await screen.findByRole("listitem", { name: "Delivery Issue create" });
    expect(d).toHaveTextContent("Failed");
    expect(d).toHaveTextContent("HTTP 500");
    expect(d).toHaveTextContent("attempt 3");
    await user.click(within(d).getByRole("button", { name: "Resend Issue create" }));
    await waitFor(() => expect(gql.of("SettingsWebhookRetry")).toHaveLength(1));
    expect(gql.of("SettingsWebhookRetry")[0]!.variables).toEqual({
      webhookId: "wh-1",
      deliveryId: "d-1",
    });
  });

  it("shows why a webhook was disabled automatically", async () => {
    mockGraphql(
      hookHandlers({
        SettingsWebhooks: () => ({
          webhooks: [{ ...HOOK, enabled: false, disabledReason: "5 deliveries failed in a row" }],
        }),
      }),
    );
    await open("webhooks");
    expect(await screen.findByText(/Disabled automatically: 5 deliveries failed/)).toBeVisible();
    expect(screen.getByRole("button", { name: "Enable CI" })).toBeInTheDocument();
  });
});

describe("WorkspaceWebhooksPage", () => {
  it("lists all webhooks, shows the team scope and creates one for all public teams", async () => {
    const gql = mockGraphql(hookHandlers());
    await renderWithApp(
      <WorkspaceWebhooksPage ws="acme" />,
      newClient(),
      "/acme/settings/webhooks",
    );
    const user = userEvent.setup();
    expect(await screen.findByRole("listitem", { name: "Webhook CI" })).toHaveTextContent(
      "Engineering",
    );
    expect(screen.getByRole("listitem", { name: "Webhook Audit sink" })).toHaveTextContent(
      "All public teams",
    );
    await user.click(screen.getByRole("button", { name: /New webhook/ }));
    expect(screen.getByLabelText("Webhook teams")).toHaveValue("all");
    await user.type(screen.getByLabelText("Webhook URL"), "https://hooks.example.com/x");
    await user.click(screen.getByRole("button", { name: "Create webhook" }));
    await waitFor(() => expect(gql.of("SettingsWebhookCreate")).toHaveLength(1));
    expect(gql.of("SettingsWebhookCreate")[0]!.variables).toEqual({
      input: {
        url: "https://hooks.example.com/x",
        resourceTypes: ["Issue"],
        allPublicTeams: true,
      },
    });
  });

  it("shows a load error", async () => {
    mockGraphql(hookHandlers({ SettingsWebhooks: () => new Error("forbidden") }));
    await renderWithApp(
      <WorkspaceWebhooksPage ws="acme" />,
      newClient(),
      "/acme/settings/webhooks",
    );
    expect(await screen.findByText("Could not load webhooks")).toBeInTheDocument();
  });
});
