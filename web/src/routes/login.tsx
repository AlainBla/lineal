import { createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/login")({
  component: () => <main className="p-8">Login (WP05)</main>,
});
