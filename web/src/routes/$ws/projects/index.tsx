import { createFileRoute, redirect } from "@tanstack/react-router";

export const Route = createFileRoute("/$ws/projects/")({
  beforeLoad: ({ params }) => {
    throw redirect({ to: `/${params.ws}/projects/all` as "/" });
  },
});
