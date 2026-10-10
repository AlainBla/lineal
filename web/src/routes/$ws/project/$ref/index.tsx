import { createFileRoute, redirect } from "@tanstack/react-router";

export const Route = createFileRoute("/$ws/project/$ref/")({
  beforeLoad: ({ params }) => {
    throw redirect({ to: `/${params.ws}/project/${params.ref}/overview` as "/" });
  },
});
