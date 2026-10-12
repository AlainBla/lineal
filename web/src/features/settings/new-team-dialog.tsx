import { useNavigate } from "@tanstack/react-router";
import { useMemo, useState } from "react";

import { useSession } from "@/app/auth";
import { teamPaths } from "@/app/nav";
import { setUiState, useUiState } from "@/app/ui-state";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { errorText } from "@/features/issues/error-text";
import { useShortcutContext } from "@/lib/shortcuts";

import { allowedVisibilities, normalizeKey, suggestTeamKey, validateKey } from "./team-key";
import { useCreateTeam, useTeamsOverview } from "./teams-queries";
import { Field, selectClass } from "./ui";

const ICON_RE = /^[A-Za-z0-9_:-]{1,64}$/;
const COLOR_RE = /^#[0-9A-Fa-f]{6}$/;

function NewTeamForm({ onClose }: { onClose: () => void }) {
  const session = useSession();
  const navigate = useNavigate();
  const teams = useTeamsOverview();
  const create = useCreateTeam();
  const all = useMemo(() => teams.data ?? [], [teams.data]);
  const active = useMemo(() => all.filter((t) => !t.retiredAt), [all]);
  const takenKeys = useMemo(
    () => [...all.map((t) => t.key), ...(session?.teams ?? []).map((t) => t.key)],
    [all, session],
  );

  const [name, setName] = useState("");
  const [key, setKey] = useState("");
  const [keyTouched, setKeyTouched] = useState(false);
  const [icon, setIcon] = useState("");
  const [color, setColor] = useState("");
  const [parentId, setParentId] = useState("");
  const [visibility, setVisibility] = useState("public");
  const [copyFrom, setCopyFrom] = useState("");

  const parent = active.find((t) => t.id === parentId);
  const visibilities = allowedVisibilities(parent);
  const effectiveKey = keyTouched ? key : name.trim() ? suggestTeamKey(name, takenKeys) : "";
  const keyError = effectiveKey === "" ? null : validateKey(effectiveKey);
  const localKeyTaken = takenKeys.some((k) => k.toUpperCase() === effectiveKey);
  const iconError = icon !== "" && !ICON_RE.test(icon) ? "Use letters, digits, _ : - only." : null;
  const colorError = color !== "" && !COLOR_RE.test(color) ? "Use the form #RRGGBB." : null;
  const valid = name.trim() !== "" && effectiveKey !== "" && !keyError && !iconError && !colorError;

  const changeParent = (id: string) => {
    setParentId(id);
    const allowed = allowedVisibilities(active.find((t) => t.id === id));
    // Unter einem privaten Parent ist `restricted` der Standard (docs/02 §5.2).
    if (!allowed.includes(visibility)) setVisibility(allowed[0]!);
  };

  return (
    <form
      className="flex flex-col gap-3 p-4"
      onSubmit={(e) => {
        e.preventDefault();
        if (!valid || create.isPending) return;
        create.mutate(
          {
            name: name.trim(),
            key: effectiveKey,
            ...(icon ? { icon } : {}),
            ...(color ? { color } : {}),
            ...(parentId ? { parentId } : {}),
            visibility,
            ...(copyFrom ? { copySettingsFromTeamId: copyFrom } : {}),
          },
          {
            onSuccess: (team) => {
              onClose();
              if (team && session)
                void navigate({ to: teamPaths(session.workspace.urlKey, team.key).home as "/" });
            },
          },
        );
      }}
    >
      <Field label="Name">
        <Input
          aria-label="Team name"
          value={name}
          autoFocus
          maxLength={80}
          onChange={(e) => setName(e.target.value)}
        />
      </Field>
      <Field label="Identifier">
        <Input
          aria-label="Team identifier"
          value={effectiveKey}
          maxLength={7}
          aria-invalid={keyError != null}
          onChange={(e) => {
            setKeyTouched(true);
            setKey(normalizeKey(e.target.value));
          }}
        />
        {keyError ? (
          <span role="alert" className="text-xs text-destructive">
            {keyError}
          </span>
        ) : localKeyTaken ? (
          <span role="alert" className="text-xs text-destructive">
            This identifier is already used by another team.
          </span>
        ) : (
          <span className="text-xs text-muted-foreground">
            Prefix of all issue numbers, for example {effectiveKey || "ENG"}-123.
          </span>
        )}
      </Field>
      <div className="grid grid-cols-2 gap-3">
        <Field label="Icon (optional)">
          <Input
            aria-label="Team icon"
            value={icon}
            placeholder="e.g. rocket"
            onChange={(e) => setIcon(e.target.value)}
          />
          {iconError && (
            <span role="alert" className="text-xs text-destructive">
              {iconError}
            </span>
          )}
        </Field>
        <Field label="Color (optional)">
          <Input
            aria-label="Team color"
            value={color}
            placeholder="#5e6ad2"
            onChange={(e) => setColor(e.target.value)}
          />
          {colorError && (
            <span role="alert" className="text-xs text-destructive">
              {colorError}
            </span>
          )}
        </Field>
      </div>
      <Field label="Parent team (optional)">
        <select
          aria-label="Parent team"
          className={selectClass}
          value={parentId}
          onChange={(e) => changeParent(e.target.value)}
        >
          <option value="">No parent</option>
          {active.map((t) => (
            <option key={t.id} value={t.id}>
              {`${t.name} (${t.key})`}
            </option>
          ))}
        </select>
      </Field>
      <Field label="Visibility">
        <select
          aria-label="Visibility"
          className={selectClass}
          value={visibility}
          onChange={(e) => setVisibility(e.target.value)}
        >
          {visibilities.map((v) => (
            <option key={v} value={v}>
              {v === "public"
                ? "Public – everyone in the workspace"
                : v === "private"
                  ? "Private – members only"
                  : "Restricted – members and members of the parent team"}
            </option>
          ))}
        </select>
        {parent && parent.visibility !== "public" && (
          <span className="text-xs text-muted-foreground">
            A sub-team cannot be more open than its parent ({parent.visibility}).
          </span>
        )}
      </Field>
      <Field label="Copy settings from team (optional)">
        <select
          aria-label="Copy settings from team"
          className={selectClass}
          value={copyFrom}
          onChange={(e) => setCopyFrom(e.target.value)}
        >
          <option value="">Start with the default workflow</option>
          {active.map((t) => (
            <option key={t.id} value={t.id}>
              {`${t.name} (${t.key})`}
            </option>
          ))}
        </select>
        <span className="text-xs text-muted-foreground">
          Copies statuses, labels, templates and automations – no issues or members.
        </span>
      </Field>
      <div className="flex items-center gap-3 pt-1">
        <Button type="submit" size="sm" disabled={!valid || create.isPending}>
          Create team
        </Button>
        <Button type="button" variant="ghost" size="sm" onClick={onClose}>
          Cancel
        </Button>
        {create.isError && (
          <span role="alert" className="text-xs text-destructive">
            {errorText(create.error)}
          </span>
        )}
      </div>
    </form>
  );
}

export function NewTeamDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  useShortcutContext("modal", open);
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>New team</DialogTitle>
          <DialogDescription>
            A team owns issues, workflow statuses, cycles and labels.
          </DialogDescription>
        </DialogHeader>
        {open && <NewTeamForm onClose={() => onOpenChange(false)} />}
      </DialogContent>
    </Dialog>
  );
}

/** Verbunden mit dem globalen UI-Zustand (Sidebar „+“, Command-Menü, Teams-Seite). */
export function ConnectedNewTeamDialog() {
  const { createTeamOpen } = useUiState();
  return (
    <NewTeamDialog open={createTeamOpen} onOpenChange={(o) => setUiState({ createTeamOpen: o })} />
  );
}
