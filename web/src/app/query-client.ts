import { QueryClient } from "@tanstack/react-query";

export function createQueryClient(): QueryClient {
  return new QueryClient({ defaultOptions: { queries: { staleTime: 30_000, retry: 1 } } });
}

/** Gemeinsamer Client der Anwendung; der Auth-Guard der Routen nutzt denselben Cache. */
export const queryClient = createQueryClient();
