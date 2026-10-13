import { FileText, Trash2 } from "lucide-react";
import { useEffect } from "react";

import { useSession } from "@/app/auth";
import { Button } from "@/components/ui/button";

import {
  draftScope,
  openCreateIssue,
  removeSavedDraft,
  saveCreateDraft,
  useSavedDrafts,
  type SavedDraft,
} from "./create-store";
import { relativeTime } from "./model";
import { EmptyState } from "./states";

/** Klartext-Auszug einer Markdown-Beschreibung. */
function snippet(md: string): string {
  return md
    .replace(/[#>*_`[\]()!-]/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 140);
}

/**
 * Drafts (`/{ws}/drafts`, docs/08 §2.3): die abgelegten Entwürfe des angemeldeten Nutzers. Sie liegen
 * nur in `sessionStorage`, sind an Nutzer und Workspace gebunden und verschwinden beim Abmelden.
 * „Open“ lädt den Entwurf wieder ins Create-Modal und nimmt ihn aus der Liste.
 */
export function DraftsPage({ ws }: { ws: string }) {
  void ws;
  const session = useSession();
  const scope = session ? draftScope(session.user.id, session.workspace.id) : null;
  const drafts = useSavedDrafts(scope);

  useEffect(() => {
    const prev = document.title;
    document.title = "Drafts · Lineal";
    return () => {
      document.title = prev;
    };
  }, []);

  const open = (d: SavedDraft) => {
    if (!scope) return;
    saveCreateDraft(scope, { title: d.title, description: d.description });
    removeSavedDraft(scope, d.id);
    openCreateIssue(d.teamKey ? { teamKey: d.teamKey } : {});
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex h-10 shrink-0 items-center gap-2 border-b border-border px-4 text-sm">
        <h1 className="font-medium">Drafts</h1>
        {drafts.length > 0 && (
          <span className="text-xs text-muted-foreground">{drafts.length}</span>
        )}
      </header>
      <div className="min-h-0 flex-1 overflow-auto">
        {drafts.length === 0 ? (
          <EmptyState
            title="No drafts"
            description="Close the new-issue dialog with content and choose Save draft to keep it here."
          />
        ) : (
          <ul aria-label="Drafts" className="flex flex-col">
            {drafts.map((d) => (
              <li
                key={d.id}
                className="group flex items-center gap-3 border-b border-border px-4 py-2 text-sm"
              >
                <FileText className="size-4 shrink-0 text-muted-foreground" aria-hidden />
                <button
                  type="button"
                  aria-label={`Open draft ${d.title || "Untitled"}`}
                  className="flex min-w-0 flex-1 flex-col items-start text-left"
                  onClick={() => open(d)}
                >
                  <span className="max-w-full truncate font-medium">{d.title || "Untitled"}</span>
                  {d.description && (
                    <span className="max-w-full truncate text-xs text-muted-foreground">
                      {snippet(d.description)}
                    </span>
                  )}
                </button>
                {d.teamKey && <span className="text-xs text-muted-foreground">{d.teamKey}</span>}
                <span className="text-xs text-subtle-foreground">{relativeTime(d.savedAt)}</span>
                <Button
                  variant="ghost"
                  size="icon"
                  className="size-7"
                  aria-label={`Delete draft ${d.title || "Untitled"}`}
                  onClick={() => scope && removeSavedDraft(scope, d.id)}
                >
                  <Trash2 className="size-3.5" />
                </Button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
