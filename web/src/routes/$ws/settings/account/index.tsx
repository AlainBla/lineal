import { createFileRoute, redirect } from "@tanstack/react-router";

export const Route = createFileRoute("/$ws/settings/account/")({
  beforeLoad: ({ params }) => {
    throw redirect({ to: `/${params.ws}/settings/account/profile` as "/" });
  },
});
