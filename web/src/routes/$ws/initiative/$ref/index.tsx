import { createFileRoute, redirect } from "@tanstack/react-router";

export const Route = createFileRoute("/$ws/initiative/$ref/")({
  beforeLoad: ({ params }) => {
    throw redirect({ to: `/${params.ws}/initiative/${params.ref}/overview` as "/" });
  },
});
