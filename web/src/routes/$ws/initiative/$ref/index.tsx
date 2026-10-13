import { createFileRoute, redirect } from "@tanstack/react-router";

export const Route = createFileRoute("/$ws/initiative/$ref/")({
  beforeLoad: ({ params, location }) => {
    throw redirect({
      to: `/${params.ws}/initiative/${params.ref}/overview` as "/",
      hash: location.hash,
    });
  },
});
