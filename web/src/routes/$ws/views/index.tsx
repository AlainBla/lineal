import { createFileRoute, redirect } from "@tanstack/react-router";

export const Route = createFileRoute("/$ws/views/")({
  beforeLoad: ({ params }) => {
    throw redirect({ to: `/${params.ws}/views/issues` as "/" });
  },
});
