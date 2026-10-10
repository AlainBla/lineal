import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { useCallback } from "react";

import { SearchPage } from "@/features/search/search-page";

export const Route = createFileRoute("/$ws/search/")({
  validateSearch: (s: Record<string, unknown>): { q?: string } => ({
    q: typeof s.q === "string" && s.q !== "" ? s.q : undefined,
  }),
  component: Page,
});

function Page() {
  const { ws } = Route.useParams();
  const { q } = Route.useSearch();
  const navigate = useNavigate();
  // Suchbegriff in der URL halten (Deep-Link, Zurück-Navigation), ohne Verlaufseinträge anzuhäufen.
  const onTermChange = useCallback(
    (term: string) => {
      void navigate({ to: ".", search: { q: term || undefined }, replace: true } as never);
    },
    [navigate],
  );
  return <SearchPage ws={ws} initialTerm={q ?? ""} onTermChange={onTermChange} />;
}
