import { createFileRoute, redirect } from "@tanstack/react-router";

export const Route = createFileRoute("/$ws/project/$ref/")({
  beforeLoad: ({ params, location }) => {
    throw redirect({
      to: `/${params.ws}/project/${params.ref}/overview` as "/",
      hash: location.hash,
    });
  },
});
