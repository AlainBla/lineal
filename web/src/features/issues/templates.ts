/**
 * Issue-Templates im Create-Modal (docs/06 §2): Auswahl, Vorbelegung und Formular-Templates.
 * `templateData` ist frei strukturiert (JSON); hier werden nur die für die Vorbelegung nötigen Felder
 * gelesen. Das Anwenden (Merge, Platzhalter, Formularauswertung) übernimmt der Server bei `issueCreate`.
 */
import { useQuery } from "@tanstack/react-query";

import type { Template } from "@/gql/graphql";
import { gqlRequest } from "@/lib/graphql";

export type FormFieldKind =
  | "text"
  | "longText"
  | "dropdown"
  | "checkboxes"
  | "date"
  | "instructions"
  | "title"
  | "priority"
  | "dueDate"
  | "labelGroup";

export interface FormField {
  id: string;
  kind: FormFieldKind;
  label: string;
  required: boolean;
  options: string[];
  labelGroupId?: string;
  text?: string;
}

export type FormAnswerValue = string | string[];

export type IssueTemplate = Pick<
  Template,
  "id" | "name" | "description" | "teamId" | "teamKey" | "isForm" | "templateData"
>;

export const templateKeys = {
  list: (teamId: string) => ["templates", "issue", teamId] as const,
  defaults: (teamId: string) => ["templates", "defaults", teamId] as const,
};

const TEMPLATES_QUERY = `query IssueTemplates($teamId: String) {
  templates(teamId: $teamId, type: issue) { id name description teamId teamKey isForm templateData }
}`;

export function useIssueTemplates(teamId: string | undefined) {
  return useQuery({
    queryKey: templateKeys.list(teamId ?? ""),
    enabled: !!teamId,
    staleTime: 60_000,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ templates: IssueTemplate[] }, { teamId: string }>(
          TEMPLATES_QUERY,
          { teamId: teamId! },
          { signal },
        )
      ).templates,
  });
}

export function useTeamDefaultTemplates(teamId: string | undefined) {
  return useQuery({
    queryKey: templateKeys.defaults(teamId ?? ""),
    enabled: !!teamId,
    staleTime: 60_000,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<
          { teamDefaultTemplates: { forMembersId: string | null; forNonMembersId: string | null } },
          { teamId: string }
        >(
          `query TeamDefaults($teamId: String!) {
            teamDefaultTemplates(teamId: $teamId) { forMembersId forNonMembersId }
          }`,
          { teamId: teamId! },
          { signal },
        )
      ).teamDefaultTemplates,
  });
}

const rec = (v: unknown): Record<string, unknown> =>
  v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : {};

const KINDS: readonly string[] = [
  "text",
  "longText",
  "dropdown",
  "checkboxes",
  "date",
  "instructions",
  "title",
  "priority",
  "dueDate",
  "labelGroup",
];

/** Felder eines Form-Templates in Formularreihenfolge; ungültige Einträge entfallen. */
export function formFields(t: Pick<IssueTemplate, "templateData">): FormField[] {
  const raw = rec(t.templateData).formFields;
  const list = Array.isArray(raw) ? raw : [];
  return list.flatMap((f): FormField[] => {
    const o = rec(f);
    if (typeof o.id !== "string" || typeof o.kind !== "string" || !KINDS.includes(o.kind)) {
      return [];
    }
    return [
      {
        id: o.id,
        kind: o.kind as FormFieldKind,
        label: typeof o.label === "string" ? o.label : o.id,
        required: o.required === true,
        options: Array.isArray(o.options) ? o.options.filter((x) => typeof x === "string") : [],
        labelGroupId: typeof o.labelGroupId === "string" ? o.labelGroupId : undefined,
        text: typeof o.text === "string" ? o.text : undefined,
      },
    ];
  });
}

/** Werte, mit denen ein (Nicht-Form-)Template das Modal vorbelegt. */
export interface TemplatePrefill {
  title?: string;
  /** Beschreibung ohne `{{placeholder: …}}`-Marken. */
  description?: string;
  stateId?: string;
  priority?: number;
  assigneeId?: string;
  projectId?: string;
  estimate?: number;
  labelIds?: string[];
}

const PLACEHOLDER = /\{\{placeholder:[^}]*\}\}/g;

export function templatePrefill(t: Pick<IssueTemplate, "templateData">): TemplatePrefill {
  const d = rec(t.templateData);
  const str = (v: unknown) => (typeof v === "string" && v !== "" ? v : undefined);
  const num = (v: unknown) => (typeof v === "number" ? v : undefined);
  return {
    title: str(d.title),
    description: str(d.description)?.replace(PLACEHOLDER, "").trim() || undefined,
    stateId: str(d.stateId),
    priority: num(d.priority),
    assigneeId: str(d.assigneeId),
    projectId: str(d.projectId),
    estimate: num(d.estimate),
    labelIds: Array.isArray(d.labelIds)
      ? d.labelIds.filter((x): x is string => typeof x === "string")
      : undefined,
  };
}

/** Fehlende Pflichtfelder (IDs) eines Formulars. */
export function missingRequired(
  fields: FormField[],
  answers: Record<string, FormAnswerValue>,
): string[] {
  return fields
    .filter((f) => f.required && f.kind !== "instructions")
    .filter((f) => {
      const v = answers[f.id];
      return v === undefined || (Array.isArray(v) ? v.length === 0 : v.trim() === "");
    })
    .map((f) => f.id);
}

/** Antworten für `issueCreate(formAnswers)`: nur ausgefüllte, nicht-statische Felder. */
export function toFormAnswers(
  fields: FormField[],
  answers: Record<string, FormAnswerValue>,
): Array<{ fieldId: string; value: FormAnswerValue }> {
  return fields.flatMap((f) => {
    const v = answers[f.id];
    if (f.kind === "instructions" || v === undefined) return [];
    if (Array.isArray(v) ? v.length === 0 : v.trim() === "") return [];
    // Priorität als Zahl, alles andere unverändert.
    const value = f.kind === "priority" && typeof v === "string" ? Number(v) : v;
    return [{ fieldId: f.id, value: value as FormAnswerValue }];
  });
}
