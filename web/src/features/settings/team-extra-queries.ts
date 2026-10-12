/**
 * Daten der Team-Settings „SLA“, „Webhooks“ und „Recurring issues“ (WP30) sowie der
 * Workspace-Seite „Webhooks“. Alle Mutationen invalidieren den jeweiligen Query-Key.
 */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import type {
  IssueLabel,
  RecurringIssue,
  SlaRule,
  TeamSla,
  Webhook,
  WebhookDelivery,
} from "@/gql/graphql";
import { gqlRequest } from "@/lib/graphql";

export const extraKeys = {
  sla: (teamId: string) => ["settings", "sla", teamId] as const,
  recurring: (teamId: string) => ["settings", "recurring", teamId] as const,
  webhooks: ["settings", "webhooks"] as const,
  deliveries: (id: string) => ["settings", "webhook-deliveries", id] as const,
  projectLabels: ["settings", "project-labels"] as const,
};

// ---------- SLA ----------

const SLA_FIELDS = `teamId enabled workWeek rules { id name position filter removesSla duration slaType startMode }`;

export function useTeamSla(teamId: string) {
  return useQuery({
    queryKey: extraKeys.sla(teamId),
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ teamSla: TeamSla }, { teamId: string }>(
          `query SettingsTeamSla($teamId: String!) { teamSla(teamId: $teamId) { ${SLA_FIELDS} } }`,
          { teamId },
          { signal },
        )
      ).teamSla,
  });
}

export interface SlaRuleInput {
  name: string;
  filter: unknown;
  removesSla: boolean;
  duration?: string;
  slaType: "all" | "onlyBusinessDays";
  startMode: "issueCreation" | "ruleMatch";
}

export function useSlaMutations(teamId: string) {
  const qc = useQueryClient();
  const done = () => void qc.invalidateQueries({ queryKey: extraKeys.sla(teamId) });
  return {
    settings: useMutation({
      mutationFn: async (input: { enabled?: boolean; workWeek?: string }) =>
        gqlRequest(
          `mutation SettingsTeamSlaUpdate($teamId: String!, $input: TeamSlaUpdateInput!) {
            teamSlaUpdate(teamId: $teamId, input: $input) { ${SLA_FIELDS} }
          }`,
          { teamId, input },
        ),
      onSuccess: done,
    }),
    create: useMutation({
      mutationFn: async (input: SlaRuleInput) =>
        gqlRequest(
          `mutation SettingsSlaRuleCreate($input: SlaRuleCreateInput!) {
            slaRuleCreate(input: $input) { id }
          }`,
          { input: { teamId, ...input } },
        ),
      onSuccess: done,
    }),
    update: useMutation({
      mutationFn: async (v: { id: string; input: Partial<SlaRuleInput> & { position?: number } }) =>
        gqlRequest(
          `mutation SettingsSlaRuleUpdate($teamId: String!, $id: String!, $input: SlaRuleUpdateInput!) {
            slaRuleUpdate(teamId: $teamId, id: $id, input: $input) { id }
          }`,
          { teamId, ...v },
        ),
      onSuccess: done,
    }),
    remove: useMutation({
      mutationFn: async (id: string) =>
        gqlRequest(
          `mutation SettingsSlaRuleDelete($teamId: String!, $id: String!) {
            slaRuleDelete(teamId: $teamId, id: $id) { success }
          }`,
          { teamId, id },
        ),
      onSuccess: done,
    }),
  };
}

export type { SlaRule };

// ---------- Recurring issues ----------

const RECURRING_FIELDS = `id teamId templateId templateName frequency interval weekday dayOfMonth minuteOfDay startDate paused nextRunAt lastRunAt lastError lastErrorAt createdAt`;

export function useRecurringIssues(teamId: string) {
  return useQuery({
    queryKey: extraKeys.recurring(teamId),
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ recurringIssues: RecurringIssue[] }, { teamId: string }>(
          `query SettingsRecurringIssues($teamId: String!) { recurringIssues(teamId: $teamId) { ${RECURRING_FIELDS} } }`,
          { teamId },
          { signal },
        )
      ).recurringIssues,
  });
}

export interface RecurringInput {
  templateId: string;
  frequency: "daily" | "weekly" | "monthly";
  interval?: number;
  weekday?: number;
  dayOfMonth?: number;
  minuteOfDay: number;
  startDate?: string;
}

export function useRecurringMutations(teamId: string) {
  const qc = useQueryClient();
  const done = () => void qc.invalidateQueries({ queryKey: extraKeys.recurring(teamId) });
  return {
    create: useMutation({
      mutationFn: async (input: RecurringInput) =>
        gqlRequest(
          `mutation SettingsRecurringCreate($input: RecurringIssueCreateInput!) {
            recurringIssueCreate(input: $input) { id }
          }`,
          { input: { teamId, ...input } },
        ),
      onSuccess: done,
    }),
    update: useMutation({
      mutationFn: async (v: { id: string; input: { paused?: boolean } }) =>
        gqlRequest(
          `mutation SettingsRecurringUpdate($id: String!, $input: RecurringIssueUpdateInput!) {
            recurringIssueUpdate(id: $id, input: $input) { id }
          }`,
          v,
        ),
      onSuccess: done,
    }),
    remove: useMutation({
      mutationFn: async (id: string) =>
        gqlRequest(
          `mutation SettingsRecurringDelete($id: String!) {
            recurringIssueDelete(id: $id) { success }
          }`,
          { id },
        ),
      onSuccess: done,
    }),
  };
}

// ---------- Projekt-Labels (Projekt-Templates) ----------

export function useProjectLabels(enabled: boolean) {
  return useQuery({
    queryKey: extraKeys.projectLabels,
    enabled,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ projectLabels: IssueLabel[] }>(
          `query SettingsProjectLabels {
            projectLabels(includeGroups: true, first: 250) { id name color isGroup retiredAt }
          }`,
          undefined,
          { signal },
        )
      ).projectLabels,
  });
}

// ---------- Webhooks ----------

const WEBHOOK_FIELDS = `id label url enabled teamId allPublicTeams resourceTypes consecutiveFailures disabledReason lastSuccessAt createdAt`;
const DELIVERY_FIELDS = `id webhookId resourceType action attempt status responseCode durationMs nextAttemptAt createdAt`;

export function useWebhooks() {
  return useQuery({
    queryKey: extraKeys.webhooks,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ webhooks: Webhook[] }>(
          `query SettingsWebhooks { webhooks { ${WEBHOOK_FIELDS} } }`,
          undefined,
          { signal },
        )
      ).webhooks,
  });
}

export function useWebhookDeliveries(id: string, enabled: boolean) {
  return useQuery({
    queryKey: extraKeys.deliveries(id),
    enabled,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ webhookDeliveries: WebhookDelivery[] }, { id: string }>(
          `query SettingsWebhookDeliveries($id: String!) { webhookDeliveries(webhookId: $id) { ${DELIVERY_FIELDS} } }`,
          { id },
          { signal },
        )
      ).webhookDeliveries,
  });
}

export interface WebhookCreate {
  label?: string;
  url: string;
  teamId?: string;
  allPublicTeams: boolean;
  resourceTypes: string[];
}

export interface WebhookSecret {
  webhook: Webhook;
  /** Klartext – nur einmal sichtbar, nie im Cache. */
  secret: string;
}

export function useWebhookMutations() {
  const qc = useQueryClient();
  const done = () => void qc.invalidateQueries({ queryKey: extraKeys.webhooks });
  return {
    // Das Ergebnis (mit Klartext-Secret) wird nicht im Mutations-Cache gehalten.
    create: useMutation({
      gcTime: 0,
      mutationFn: async (input: WebhookCreate) =>
        (
          await gqlRequest<{ webhookCreate: WebhookSecret }, { input: WebhookCreate }>(
            `mutation SettingsWebhookCreate($input: WebhookCreateInput!) {
              webhookCreate(input: $input) { success secret webhook { ${WEBHOOK_FIELDS} } }
            }`,
            { input },
          )
        ).webhookCreate,
      onSuccess: done,
    }),
    rotate: useMutation({
      gcTime: 0,
      mutationFn: async (id: string) =>
        (
          await gqlRequest<{ webhookRotateSecret: WebhookSecret }, { id: string }>(
            `mutation SettingsWebhookRotate($id: String!) {
              webhookRotateSecret(id: $id) { success secret webhook { ${WEBHOOK_FIELDS} } }
            }`,
            { id },
          )
        ).webhookRotateSecret,
      onSuccess: done,
    }),
    update: useMutation({
      mutationFn: async (v: {
        id: string;
        input: { label?: string; url?: string; enabled?: boolean; resourceTypes?: string[] };
      }) =>
        gqlRequest(
          `mutation SettingsWebhookUpdate($id: String!, $input: WebhookUpdateInput!) {
            webhookUpdate(id: $id, input: $input) { success }
          }`,
          v,
        ),
      onSuccess: done,
    }),
    remove: useMutation({
      mutationFn: async (id: string) =>
        gqlRequest(
          `mutation SettingsWebhookDelete($id: String!) { webhookDelete(id: $id) { success } }`,
          { id },
        ),
      onSuccess: done,
    }),
    retry: useMutation({
      mutationFn: async (v: { webhookId: string; deliveryId: string }) =>
        gqlRequest(
          `mutation SettingsWebhookRetry($webhookId: String!, $deliveryId: String!) {
            webhookDeliveryRetry(webhookId: $webhookId, deliveryId: $deliveryId) { success }
          }`,
          v,
        ),
      onSuccess: (_d, v) =>
        void qc.invalidateQueries({ queryKey: extraKeys.deliveries(v.webhookId) }),
    }),
  };
}
