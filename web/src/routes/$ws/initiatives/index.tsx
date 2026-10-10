import { createFileRoute, redirect } from "@tanstack/react-router";

export const Route = createFileRoute("/$ws/initiatives/")({
  beforeLoad: ({ params }) => {
    throw redirect({ to: `/${params.ws}/initiatives/active` as "/" });
  },
});
