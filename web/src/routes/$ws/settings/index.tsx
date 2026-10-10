import { createFileRoute, redirect } from "@tanstack/react-router";

export const Route = createFileRoute("/$ws/settings/")({
  beforeLoad: ({ params }) => {
    throw redirect({ to: `/${params.ws}/settings/workspace` as "/" });
  },
});
