/**
 * Gemeinsame Label-Verwaltung für Team-Labels (`Issue labels` im Team) und Workspace-Labels
 * (`/{ws}/settings/labels`): anlegen (auch Gruppen), umbenennen, Farbe ändern, retire/restore.
 */
import { Plus } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { errorText } from "@/features/issues/error-text";
import type { IssueLabel } from "@/gql/graphql";

import { LabelBulkDialog, type LabelBulkMode } from "./label-bulk-dialog";
import { useLabelMutations } from "./queries";
import { useLabelRestore } from "./template-queries";
import { Section, selectClass } from "./ui";

export const COLOR_RE = /^#[0-9A-Fa-f]{6}$/;

function Dot({ color }: { color: string }) {
  return (
    <span
      aria-hidden
      className="size-2.5 shrink-0 rounded-full"
      style={{ background: COLOR_RE.test(color) ? color : undefined }}
    />
  );
}

function LabelRow({
  label,
  indent,
  retired,
  onRetire,
  onRestore,
  onSave,
  onBulk,
}: {
  label: IssueLabel;
  indent: boolean;
  retired: boolean;
  onRetire: () => void;
  onRestore: () => void;
  onSave: (input: { name?: string; color?: string }) => void;
  onBulk?: (mode: LabelBulkMode) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(label.name);
  const [color, setColor] = useState(label.color);
  const valid = name.trim() !== "" && COLOR_RE.test(color);
  return (
    <li
      className={`flex items-center gap-2 px-3 py-1.5 text-sm ${indent ? "pl-8" : ""} ${retired ? "text-muted-foreground" : ""}`}
    >
      <Dot color={label.color} />
      {editing ? (
        <form
          aria-label={`Edit ${label.name}`}
          className="flex flex-1 items-center gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            if (!valid) return;
            onSave({
              ...(name.trim() !== label.name ? { name: name.trim() } : {}),
              ...(color !== label.color ? { color } : {}),
            });
            setEditing(false);
          }}
        >
          <Input
            aria-label="Label name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            autoFocus
          />
          <Input
            aria-label="Label color"
            value={color}
            onChange={(e) => setColor(e.target.value)}
            className="w-24"
          />
          <Button type="submit" size="sm" disabled={!valid}>
            Save
          </Button>
        </form>
      ) : (
        <span className="flex-1">
          {label.name}
          {label.isGroup && <span className="ml-2 text-xs text-muted-foreground">Group</span>}
        </span>
      )}
      {!retired && (
        <Button
          variant="ghost"
          size="sm"
          aria-label={`Rename ${label.name}`}
          onClick={() => {
            setEditing(true);
            setName(label.name);
            setColor(label.color);
          }}
        >
          Edit
        </Button>
      )}
      {onBulk && !label.isGroup && (
        <Button
          variant="ghost"
          size="sm"
          aria-label={`Merge ${label.name}`}
          onClick={() => onBulk("merge")}
        >
          Merge
        </Button>
      )}
      {onBulk && (
        <Button
          variant="ghost"
          size="sm"
          aria-label={`Delete ${label.name}`}
          onClick={() => onBulk("delete")}
        >
          Delete
        </Button>
      )}
      {retired ? (
        <Button variant="ghost" size="sm" aria-label={`Restore ${label.name}`} onClick={onRestore}>
          Restore
        </Button>
      ) : (
        <Button variant="ghost" size="sm" aria-label={`Retire ${label.name}`} onClick={onRetire}>
          Retire
        </Button>
      )}
    </li>
  );
}

/** Reihenfolge: Gruppen mit ihren Kindern, danach Labels ohne Gruppe. */
export function orderLabels(labels: IssueLabel[]): Array<{ label: IssueLabel; indent: boolean }> {
  const ids = new Set(labels.map((l) => l.id));
  const roots = labels.filter((l) => !l.parentId || !ids.has(l.parentId));
  const out: Array<{ label: IssueLabel; indent: boolean }> = [];
  for (const r of roots) {
    out.push({ label: r, indent: false });
    for (const c of labels.filter((l) => l.parentId === r.id)) out.push({ label: c, indent: true });
  }
  return out;
}

export function LabelManager({
  title,
  description,
  teamId,
  labels,
}: {
  title: string;
  description: string;
  /** Fehlt: Workspace-Labels. */
  teamId?: string;
  /** Labels genau dieses Geltungsbereichs (Team bzw. Workspace), aktive und zurückgezogene. */
  labels: IssueLabel[];
}) {
  const m = useLabelMutations();
  const restore = useLabelRestore();
  const [name, setName] = useState("");
  const [group, setGroup] = useState(false);
  const [color, setColor] = useState("#5e6ad2");
  const [parentId, setParentId] = useState("");
  const [bulk, setBulk] = useState<{ label: IssueLabel; mode: LabelBulkMode } | null>(null);
  const active = labels.filter((l) => !l.retiredAt);
  const retired = labels.filter((l) => l.retiredAt);
  const groups = active.filter((l) => l.isGroup);
  const valid = name.trim() !== "" && COLOR_RE.test(color);
  const error = m.create.error ?? m.update.error ?? m.retire.error ?? restore.error;
  return (
    <Section title={title} description={description}>
      <ul className="divide-y divide-border rounded-lg border border-border">
        {active.length === 0 && (
          <li className="px-3 py-2 text-sm text-muted-foreground">No labels yet.</li>
        )}
        {orderLabels(active).map(({ label, indent }) => (
          <LabelRow
            key={label.id}
            label={label}
            indent={indent}
            retired={false}
            onRetire={() => m.retire.mutate(label.id)}
            onRestore={() => undefined}
            onSave={(input) => m.update.mutate({ id: label.id, input })}
            onBulk={(mode) => setBulk({ label, mode })}
          />
        ))}
      </ul>
      <form
        aria-label="Create label"
        className="flex flex-wrap items-center gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          if (!valid) return;
          m.create.mutate(
            {
              name: name.trim(),
              color,
              ...(teamId ? { teamId } : {}),
              ...(group ? { isGroup: true } : {}),
              ...(!group && parentId ? { parentId } : {}),
            },
            { onSuccess: () => setName("") },
          );
        }}
      >
        <Input
          aria-label="New label name"
          placeholder="New label"
          value={name}
          onChange={(e) => setName(e.target.value)}
          className="max-w-48"
        />
        <Input
          aria-label="Label color"
          value={color}
          onChange={(e) => setColor(e.target.value)}
          className="w-24"
        />
        <label className="flex items-center gap-1 text-sm">
          <input type="checkbox" checked={group} onChange={(e) => setGroup(e.target.checked)} />{" "}
          Group
        </label>
        {!group && groups.length > 0 && (
          <select
            aria-label="Label group"
            className={selectClass}
            value={parentId}
            onChange={(e) => setParentId(e.target.value)}
          >
            <option value="">No group</option>
            {groups.map((g) => (
              <option key={g.id} value={g.id}>
                {g.name}
              </option>
            ))}
          </select>
        )}
        <Button type="submit" size="sm" disabled={!valid || m.create.isPending}>
          <Plus /> Create
        </Button>
      </form>
      {error != null && (
        <p role="alert" className="text-xs text-destructive">
          {errorText(error)}
        </p>
      )}
      {retired.length > 0 && (
        <div className="flex flex-col gap-1">
          <h3 className="text-xs font-medium text-muted-foreground">Retired</h3>
          <ul className="divide-y divide-border rounded-lg border border-border">
            {orderLabels(retired).map(({ label, indent }) => (
              <LabelRow
                key={label.id}
                label={label}
                indent={indent}
                retired
                onRetire={() => undefined}
                onRestore={() => restore.mutate(label.id)}
                onSave={() => undefined}
                onBulk={(mode) => setBulk({ label, mode })}
              />
            ))}
          </ul>
        </div>
      )}
      {bulk && (
        <LabelBulkDialog label={bulk.label} mode={bulk.mode} onClose={() => setBulk(null)} />
      )}
    </Section>
  );
}
