import { QueryClient } from "@tanstack/react-query";

import { GraphQLRequestError } from "@/lib/graphql";

/**
 * Wiederholungen nur bei vorübergehenden Fehlern (Netz, 5xx): 4xx wie 401/403/404/429 ändern sich
 * beim zweiten Versuch nicht (429 wiederholt bereits der GraphQL-Client nach Retry-After).
 */
export function shouldRetry(failureCount: number, error: unknown): boolean {
  if (failureCount >= 1) return false;
  if (error instanceof GraphQLRequestError) return error.status === 0 || error.status >= 500;
  return true;
}

export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: { queries: { staleTime: 30_000, retry: shouldRetry } },
  });
}

/** Gemeinsamer Client der Anwendung; der Auth-Guard der Routen nutzt denselben Cache. */
export const queryClient = createQueryClient();
