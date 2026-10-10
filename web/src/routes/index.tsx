import { createFileRoute } from "@tanstack/react-router";

import { useSession } from "@/app/auth";
import { Keys } from "@/components/ui/kbd";

export const Route = createFileRoute("/")({
  component: Home,
});

function Home() {
  const session = useSession();
  return (
    <div className="mx-auto flex max-w-xl flex-col gap-4 px-6 py-16">
      <h1 className="text-xl font-semibold tracking-tight">Lineal</h1>
      <p className="text-muted-foreground">
        Welcome{session ? `, ${session.user.name}` : ""}. Pick something from the sidebar or press{" "}
        <Keys keys="mod+k" className="align-middle" /> to run any command.
      </p>
    </div>
  );
}
