import { useNavigate } from "@tanstack/react-router";
import { useState } from "react";

import { useSession } from "@/app/auth";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { errorText } from "@/features/issues/error-text";
import { useShortcutContext } from "@/lib/shortcuts";

import type { DisplayOptions } from "./display-options";
import { FilterBar } from "./filter-bar";
import { buildFilter, type FilterChip } from "./filter-model";
import { useCreateView, useViewIssues } from "./queries";
import { useFilterSources } from "./use-filter-sources";

/** Preferences, die beim Anlegen als Standard der View gespeichert werden (docs/05 §2.2 Nr. 4). */
export function displayToPreferences(o: DisplayOptions): Record<string, unknown> {
  return {
    layout: o.layout,
    grouping: o.grouping,
    ordering: o.ordering,
    orderDirection: o.orderDirection,
    showSubIssues: o.showSubIssues,
    showCompletedIssues: o.showCompletedIssues,
    showEmptyGroups: o.showEmptyGroups,
  };
}

/** View-Builder (docs/08 §2.10): Name, Save-to, Filterleiste, Live-Vorschau. */
export function ViewBuilderDialog({
  ws,
  initialChips = [],
  initialOptions,
  teamId,
  onClose,
}: {
  ws: string;
  initialChips?: FilterChip[];
  initialOptions?: DisplayOptions;
  teamId?: string;
  onClose: () => void;
}) {
  useShortcutContext("modal");
  const session = useSession();
  const navigate = useNavigate();
  const create = useCreateView();
  const { sources } = useFilterSources();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  // Gäste dürfen nur persönliche Views anlegen (Server: guest_scope).
  const isGuest = session?.user.role === "guest";
  const [saveTo, setSaveTo] = useState<string>(isGuest ? "personal" : (teamId ?? "personal"));
  const [chips, setChips] = useState<FilterChip[]>(initialChips);
  const filter = buildFilter(chips);
  const preview = useViewIssues({ filter });

  const submit = () => {
    create.mutate(
      {
        name: name.trim(),
        description: description.trim(),
        modelName: "Issue",
        filterData: filter,
        saveTo,
        preferences: initialOptions ? displayToPreferences(initialOptions) : undefined,
      },
      {
        onSuccess: (v) => {
          onClose();
          void navigate({ to: `/${ws}/view/${v.slugId}` as "/" });
        },
      },
    );
  };

  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-2xl" aria-describedby={undefined}>
        <DialogTitle>New view</DialogTitle>
        <DialogDescription className="sr-only">
          Save filters and display options as a custom view.
        </DialogDescription>
        <form
          className="flex flex-col gap-3"
          onSubmit={(e) => {
            e.preventDefault();
            if (name.trim()) submit();
          }}
        >
          <Input
            aria-label="View name"
            placeholder="View name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            autoFocus
          />
          <Input
            aria-label="Description"
            placeholder="Description (optional)"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
          />
          <label className="flex items-center gap-2 text-sm">
            Save to
            <select
              aria-label="Save to"
              value={saveTo}
              onChange={(e) => setSaveTo(e.target.value)}
              className="h-8 rounded-md border border-input bg-transparent px-2 text-sm"
            >
              <option value="personal">Personal</option>
              {!isGuest && <option value="workspace">Workspace</option>}
              {!isGuest &&
                (session?.teams ?? []).map((t) => (
                  <option key={t.id} value={t.id}>
                    {t.name}
                  </option>
                ))}
            </select>
          </label>
          <FilterBar chips={chips} onChange={setChips} sources={sources} />
          <p className="text-xs text-muted-foreground" aria-live="polite">
            {preview.isLoading
              ? "Loading preview…"
              : preview.isError
                ? "Preview unavailable"
                : `${preview.data?.issues.length ?? 0}${preview.data?.truncated ? "+" : ""} matching issues`}
          </p>
          {create.isError && (
            <p role="alert" className="text-xs text-destructive">
              {errorText(create.error)}
            </p>
          )}
          <div className="flex justify-end gap-2">
            <Button type="button" variant="ghost" size="sm" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" size="sm" disabled={!name.trim() || create.isPending}>
              Create view
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
