import { createFileRoute, redirect } from "@tanstack/react-router";

export const Route = createFileRoute("/$ws/settings/teams/$key/")({
  beforeLoad: ({ params }) => {
    throw redirect({ to: `/${params.ws}/settings/teams/${params.key}/general` as "/" });
  },
});
