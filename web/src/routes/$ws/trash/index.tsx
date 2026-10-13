import { createFileRoute, redirect } from "@tanstack/react-router";

export const Route = createFileRoute("/$ws/trash/")({
  beforeLoad: ({ params }) => {
    throw redirect({ to: `/${params.ws}/trash/issues` as "/" });
  },
});
