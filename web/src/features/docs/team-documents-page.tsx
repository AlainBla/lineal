import { useEffect } from "react";

import { useSession } from "@/app/auth";
import { EmptyState } from "@/features/issues/states";

import { DocumentsSection } from "./documents-section";

/** Dokumente eines Teams (`/team/<KEY>/documents`, docs/06 §1.4/§1.5). */
export function TeamDocumentsPage({ ws, teamKey }: { ws: string; teamKey: string }) {
  const session = useSession();
  const team = session?.teams.find((t) => t.key === teamKey);
  useEffect(() => {
    const prev = document.title;
    document.title = `${teamKey} · Documents · Lineal`;
    return () => {
      document.title = prev;
    };
  }, [teamKey]);
  if (!team) return <EmptyState title="Team not found" />;
  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex h-10 shrink-0 items-center gap-3 border-b border-border px-3">
        <h1 className="truncate text-sm font-medium">{team.name} · Documents</h1>
      </header>
      <div className="min-h-0 flex-1 overflow-auto">
        <DocumentsSection ws={ws} scope={{ teamId: team.id }} teamId={team.id} />
      </div>
    </div>
  );
}
