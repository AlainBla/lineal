/**
 * Team-Konfiguration in den Settings (WP27, docs/02 §2.5): Estimates, Automations und Templates.
 * Bei Sub-Teams mit geerbter Schätzung sind die Estimates schreibgeschützt (Hinweis statt Formular).
 */
import { useState } from "react";

import type { Team } from "@/gql/graphql";

import { useUpdateTeam, useWorkflowStates } from "./queries";
import { SaveRow } from "./team-sections";
import { TemplatesManager } from "./templates-manager";
import { Field, Section, selectClass } from "./ui";

// ---------- Estimates ----------

export const ESTIMATE_SCALES = [
  { value: "notUsed", label: "Not in use", example: "" },
  { value: "exponential", label: "Exponential", example: "1, 2, 4, 8, 16" },
  { value: "fibonacci", label: "Fibonacci", example: "1, 2, 3, 5, 8" },
  { value: "linear", label: "Linear", example: "1, 2, 3, 4, 5" },
  { value: "tShirt", label: "T-shirt sizes", example: "XS, S, M, L, XL" },
] as const;

const SCALE_VALUES: Record<string, { base: number[]; ext: number[] }> = {
  exponential: { base: [1, 2, 4, 8, 16], ext: [32, 64] },
  fibonacci: { base: [1, 2, 3, 5, 8], ext: [13, 21] },
  linear: { base: [1, 2, 3, 4, 5], ext: [6, 7] },
  tShirt: { base: [1, 2, 3, 5, 8], ext: [13, 21] },
};
const TSHIRT = new Map([
  [1, "XS"],
  [2, "S"],
  [3, "M"],
  [5, "L"],
  [8, "XL"],
  [13, "XXL"],
  [21, "XXXL"],
]);

/** Positive Schätzwerte der Skala mit Anzeigenamen (für das Default-Estimate). */
export function scaleOptions(type: string, extended: boolean) {
  const s = SCALE_VALUES[type];
  if (!s) return [];
  return [...s.base, ...(extended ? s.ext : [])].map((v) => ({
    value: v,
    label: type === "tShirt" ? `${TSHIRT.get(v) ?? v}` : String(v),
  }));
}

function InheritedNotice({ what }: { what: string }) {
  return (
    <p
      role="note"
      className="rounded-md border border-border bg-surface-hover px-3 py-2 text-sm text-muted-foreground"
    >
      {what} are inherited from the parent team and can only be changed there.
    </p>
  );
}

export function EstimatesSection({ team }: { team: Team }) {
  const update = useUpdateTeam(team.id);
  const inherited = team.inheritIssueEstimation;
  const [type, setType] = useState(team.issueEstimationType);
  const [extended, setExtended] = useState(team.issueEstimationExtended);
  const [allowZero, setAllowZero] = useState(team.issueEstimationAllowZero);
  const [dflt, setDflt] = useState(team.defaultIssueEstimate);
  const unused = type === "notUsed";
  const options = scaleOptions(type, extended);
  // Ein Default außerhalb der (neuen) Skala wird beim Speichern auf „No estimate“ zurückgesetzt.
  const effectiveDefault = options.some((o) => o.value === dflt) ? dflt : 0;
  const dirty =
    type !== team.issueEstimationType ||
    extended !== team.issueEstimationExtended ||
    allowZero !== team.issueEstimationAllowZero ||
    effectiveDefault !== team.defaultIssueEstimate;
  return (
    <Section title="Estimates" description="How issues of this team are sized.">
      {inherited && <InheritedNotice what="Estimate settings" />}
      <form
        className="flex flex-col gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          if (inherited) return;
          update.mutate({
            ...(type !== team.issueEstimationType ? { issueEstimationType: type } : {}),
            ...(extended !== team.issueEstimationExtended
              ? { issueEstimationExtended: extended }
              : {}),
            ...(allowZero !== team.issueEstimationAllowZero
              ? { issueEstimationAllowZero: allowZero }
              : {}),
            ...(effectiveDefault !== team.defaultIssueEstimate
              ? { defaultIssueEstimate: effectiveDefault }
              : {}),
          });
        }}
      >
        <Field label="Estimation scale">
          <select
            aria-label="Estimation scale"
            className={selectClass}
            value={type}
            disabled={inherited}
            onChange={(e) => setType(e.target.value)}
          >
            {ESTIMATE_SCALES.map((s) => (
              <option key={s.value} value={s.value}>
                {s.example ? `${s.label} (${s.example})` : s.label}
              </option>
            ))}
          </select>
        </Field>
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            aria-label="Extended estimates"
            checked={extended}
            disabled={inherited || unused}
            onChange={(e) => setExtended(e.target.checked)}
          />
          Extended scale (two more values)
        </label>
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            aria-label="Allow zero estimates"
            checked={allowZero}
            disabled={inherited || unused}
            onChange={(e) => setAllowZero(e.target.checked)}
          />
          Allow zero as an estimate
        </label>
        <Field label="Default estimate for new issues">
          <select
            aria-label="Default estimate"
            className={selectClass}
            value={effectiveDefault}
            disabled={inherited || unused}
            onChange={(e) => setDflt(Number(e.target.value))}
          >
            <option value={0}>No estimate</option>
            {options.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </Field>
        {!inherited && <SaveRow pending={update.isPending} error={update.error} dirty={dirty} />}
      </form>
    </Section>
  );
}

// ---------- Automations ----------

const CLOSE_PERIODS = [1, 3, 6, 9, 12];

const monthsLabel = (m: number) => `${m} month${m === 1 ? "" : "s"}`;

export function AutomationsSection({ team }: { team: Team }) {
  const update = useUpdateTeam(team.id);
  const states = useWorkflowStates(team.id);
  const canceled = (states.data ?? []).filter((s) => s.type === "canceled");
  const [closePeriod, setClosePeriod] = useState(team.autoClosePeriod ?? 0);
  const [closeState, setCloseState] = useState(team.autoCloseStateId ?? "");
  const [archivePeriod, setArchivePeriod] = useState(team.autoArchivePeriod);
  const [closeParents, setCloseParents] = useState(team.autoCloseParentIssues);
  const [closeChildren, setCloseChildren] = useState(team.autoCloseChildIssues);
  const dirty =
    closePeriod !== (team.autoClosePeriod ?? 0) ||
    (closePeriod !== 0 && closeState !== (team.autoCloseStateId ?? "")) ||
    archivePeriod !== team.autoArchivePeriod ||
    closeParents !== team.autoCloseParentIssues ||
    closeChildren !== team.autoCloseChildIssues;
  return (
    <Section
      title="Automations"
      description="Close and archive issues automatically, and keep parents and sub-issues in sync."
    >
      <form
        className="flex flex-col gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          update.mutate({
            ...(closePeriod !== (team.autoClosePeriod ?? 0)
              ? { autoClosePeriod: closePeriod }
              : {}),
            // Leerer Wert = zurück auf den Default (erster „canceled“-Status).
            ...(closePeriod !== 0 && closeState !== (team.autoCloseStateId ?? "")
              ? { autoCloseStateId: closeState }
              : {}),
            ...(archivePeriod !== team.autoArchivePeriod
              ? { autoArchivePeriod: archivePeriod }
              : {}),
            ...(closeParents !== team.autoCloseParentIssues
              ? { autoCloseParentIssues: closeParents }
              : {}),
            ...(closeChildren !== team.autoCloseChildIssues
              ? { autoCloseChildIssues: closeChildren }
              : {}),
          });
        }}
      >
        <Field label="Auto-close issues after">
          <select
            aria-label="Auto-close period"
            className={selectClass}
            value={closePeriod}
            onChange={(e) => setClosePeriod(Number(e.target.value))}
          >
            <option value={0}>Off</option>
            {CLOSE_PERIODS.map((m) => (
              <option key={m} value={m}>
                {monthsLabel(m)} without activity
              </option>
            ))}
          </select>
        </Field>
        <Field label="Close as (canceled status)">
          <select
            aria-label="Auto-close status"
            className={selectClass}
            value={closeState}
            disabled={closePeriod === 0 || states.isLoading}
            onChange={(e) => setCloseState(e.target.value)}
          >
            <option value="">Team default</option>
            {canceled.map((s) => (
              <option key={s.id} value={s.id}>
                {s.name}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Auto-archive closed issues after">
          <select
            aria-label="Auto-archive period"
            className={selectClass}
            value={archivePeriod}
            onChange={(e) => setArchivePeriod(Number(e.target.value))}
          >
            {CLOSE_PERIODS.map((m) => (
              <option key={m} value={m}>
                {monthsLabel(m)}
              </option>
            ))}
          </select>
        </Field>
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            aria-label="Auto-close parent issues"
            checked={closeParents}
            onChange={(e) => setCloseParents(e.target.checked)}
          />
          Close a parent issue when all its sub-issues are closed
        </label>
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            aria-label="Auto-close sub-issues"
            checked={closeChildren}
            onChange={(e) => setCloseChildren(e.target.checked)}
          />
          Close all sub-issues when the parent issue is closed
        </label>
        <SaveRow pending={update.isPending} error={update.error} dirty={dirty} />
      </form>
    </Section>
  );
}

// ---------- Templates ----------

export function TemplatesSection({ team }: { team: Team }) {
  return <TemplatesManager team={team} />;
}
