import { QueryClientProvider, type QueryClient } from "@tanstack/react-query";
import type { ReactNode } from "react";

import { queryClient } from "@/app/query-client";
import { ThemeProvider } from "@/app/theme-provider";
import { TooltipProvider } from "@/components/ui/tooltip";

/** Wurzel-Provider der Anwendung: TanStack Query, Theme/Motion, Tooltips. */
export function Providers({
  children,
  client = queryClient,
}: {
  children: ReactNode;
  client?: QueryClient;
}) {
  return (
    <QueryClientProvider client={client}>
      <ThemeProvider>
        <TooltipProvider>{children}</TooltipProvider>
      </ThemeProvider>
    </QueryClientProvider>
  );
}
