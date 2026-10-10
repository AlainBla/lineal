import { createFileRoute } from "@tanstack/react-router";

import { Button } from "@/components/ui/button";

export const Route = createFileRoute("/")({
  component: Home,
});

function Home() {
  return (
    <main className="flex min-h-screen flex-col items-center justify-center gap-4">
      <h1 className="text-3xl font-semibold tracking-tight">Lineal</h1>
      <p className="text-sm text-muted-foreground">Issue tracking, rebuilt.</p>
      <Button variant="outline" asChild>
        <a href="/graphql">GraphQL</a>
      </Button>
    </main>
  );
}
