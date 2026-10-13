import { AlertTriangle, CircleDashed } from "lucide-react";
import type { ReactNode } from "react";

import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";

import { GraphQLRequestError } from "@/lib/graphql";

import { errorText } from "./error-text";

/** Wiederholen hilft bei Berechtigungs- und Nicht-gefunden-Fehlern nicht. */
export function isRetryable(error: unknown): boolean {
  if (!(error instanceof GraphQLRequestError)) return true;
  return !(error.code === "not_found" || error.code === "forbidden" || error.status === 403);
}

/** Ladeskelett einer Issue-Liste. */
export function ListSkeleton({ rows = 8 }: { rows?: number }) {
  return (
    <div role="status" aria-label="Loading issues" className="flex flex-col">
      {Array.from({ length: rows }, (_, i) => (
        <div key={i} className="flex h-9 items-center gap-3 border-b border-border px-4">
          <Skeleton className="size-3.5 rounded-sm" />
          <Skeleton className="h-3 w-14" />
          <Skeleton className="size-3.5 rounded-full" />
          <Skeleton className="h-3 max-w-md flex-1" />
        </div>
      ))}
    </div>
  );
}

export function BoardSkeleton() {
  return (
    <div role="status" aria-label="Loading board" className="flex gap-3 p-4">
      {Array.from({ length: 4 }, (_, i) => (
        <div key={i} className="flex w-72 shrink-0 flex-col gap-2">
          <Skeleton className="h-5 w-28" />
          <Skeleton className="h-20 w-full" />
          <Skeleton className="h-20 w-full" />
        </div>
      ))}
    </div>
  );
}

export function ErrorState({
  error,
  onRetry,
  title = "Could not load issues",
}: {
  error: unknown;
  onRetry?: () => void;
  title?: string;
}) {
  return (
    <div role="alert" className="flex flex-col items-center gap-3 px-6 py-16 text-center">
      <AlertTriangle className="size-5 text-destructive" aria-hidden />
      <h2 className="text-sm font-medium">{title}</h2>
      <p className="max-w-sm text-sm text-muted-foreground">{errorText(error)}</p>
      {onRetry && isRetryable(error) && (
        <Button variant="outline" size="sm" onClick={onRetry}>
          Try again
        </Button>
      )}
    </div>
  );
}

export function EmptyState({
  title,
  description,
  action,
}: {
  title: string;
  description?: string;
  action?: ReactNode;
}) {
  return (
    <div className="flex flex-col items-center gap-3 px-6 py-16 text-center">
      <div className="flex size-10 items-center justify-center rounded-lg border border-border-strong bg-surface text-muted-foreground">
        <CircleDashed className="size-5" aria-hidden />
      </div>
      <h2 className="text-sm font-medium">{title}</h2>
      {description && <p className="max-w-sm text-sm text-muted-foreground">{description}</p>}
      {action}
    </div>
  );
}
