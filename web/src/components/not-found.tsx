import { Link } from "@tanstack/react-router";
import { FileQuestion } from "lucide-react";

import { useSession } from "@/app/auth";

/**
 * Einheitliche 404-Seite (unbekannte URL, unbekanntes Team/Issue): Titel, Erklärung und Link zurück
 * in den Workspace. Bewusst ohne „Try again“, Wiederholen hilft hier nicht.
 */
export function NotFound({
  title = "Page not found",
  description = "This page does not exist, or you do not have access to it.",
}: {
  title?: string;
  description?: string;
}) {
  const session = useSession();
  const home = session ? `/${session.workspace.urlKey}/my-issues` : "/";
  return (
    <div
      role="alert"
      className="flex h-full flex-col items-center justify-center gap-3 p-8 text-center"
    >
      <div className="flex size-10 items-center justify-center rounded-lg border border-border-strong bg-surface text-muted-foreground">
        <FileQuestion className="size-5" aria-hidden />
      </div>
      <h1 className="text-base font-semibold tracking-tight">{title}</h1>
      <p className="max-w-sm text-sm text-muted-foreground">{description}</p>
      <Link to={home as "/"} className="text-sm text-primary hover:underline">
        Go to my issues
      </Link>
    </div>
  );
}
