/** „Recently deleted teams“: gelöschte Teams, solange sie wiederherstellbar sind (nur Admins). */
import { Button } from "@/components/ui/button";
import { errorText } from "@/features/issues/error-text";
import { showToast } from "@/features/issues/toast-store";

import { useDeletedTeams, useRestoreTeam } from "./team-nesting-queries";
import { Section } from "./ui";

const date = (iso: string) => new Date(iso).toISOString().slice(0, 10);

export function DeletedTeamsSection() {
  const q = useDeletedTeams(true);
  const restore = useRestoreTeam();
  const rows = q.data ?? [];
  // Ohne gelöschte Teams (oder ohne Recht) bleibt der Abschnitt weg.
  if (q.isError || rows.length === 0) return null;
  return (
    <Section
      title="Recently deleted teams"
      description="Deleted teams can be restored until the date shown. After that they are removed for good."
    >
      <ul
        aria-label="Recently deleted teams"
        className="divide-y divide-border rounded-lg border border-border"
      >
        {rows.map((t) => (
          <li key={t.id} className="flex items-center gap-3 px-3 py-2 text-sm">
            <div className="min-w-0 flex-1">
              <p className="font-medium">{t.name}</p>
              <p className="text-xs text-muted-foreground">
                {t.key} · restorable until {date(t.restorableUntil)}
                {t.subTeamCount > 0
                  ? ` · with ${t.subTeamCount} sub-team${t.subTeamCount === 1 ? "" : "s"}`
                  : ""}
              </p>
            </div>
            <Button
              size="sm"
              variant="outline"
              aria-label={`Restore ${t.name}`}
              disabled={restore.isPending}
              onClick={() =>
                restore.mutate(t.id, {
                  onSuccess: () => showToast({ message: `${t.name} restored` }),
                })
              }
            >
              Restore
            </Button>
          </li>
        ))}
      </ul>
      {restore.error != null && (
        <p role="alert" className="text-xs text-destructive">
          {errorText(restore.error)}
        </p>
      )}
    </Section>
  );
}
