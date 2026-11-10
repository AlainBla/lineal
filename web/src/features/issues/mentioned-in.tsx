import { Link } from "@tanstack/react-router";

import type { Issue } from "@/gql/graphql";

import { useIssueExtras } from "./hooks";
import { StatusIcon } from "./icons";
import { issuePath } from "./model";

/**
 * „Mentioned in“: Issues, die dieses Issue in Beschreibung oder Kommentar erwähnen
 * (`Issue.mentionedIn`, docs/03 §4.1). Erwähnungen legen keine Relation an. Ohne Rückverweise
 * bleibt der Abschnitt ausgeblendet.
 */
export function MentionedInSection({ issue, ws }: { issue: Issue; ws: string }) {
  const extras = useIssueExtras(issue.id);
  const items = extras.data?.mentionedIn ?? [];
  if (items.length === 0) return null;
  return (
    <section aria-label="Mentioned in" className="flex flex-col gap-1.5">
      <h2 className="text-sm font-medium">Mentioned in</h2>
      <ul className="flex flex-col gap-0.5">
        {items.map((m) => (
          <li key={`${m.id}:${m.commentId ?? "description"}`}>
            <Link
              to={issuePath(ws, m.identifier) as "/"}
              className="flex items-center gap-2 rounded px-1 text-sm hover:bg-surface-hover"
            >
              <StatusIcon type={m.stateType} color={m.stateColor} title={m.stateName} />
              <span className="shrink-0 text-xs whitespace-nowrap text-muted-foreground">
                {m.identifier}
              </span>
              <span className="truncate">{m.title}</span>
              <span className="ml-auto shrink-0 text-xs text-muted-foreground">
                {m.source === "comment" ? "in a comment" : "in the description"}
              </span>
            </Link>
          </li>
        ))}
      </ul>
    </section>
  );
}
