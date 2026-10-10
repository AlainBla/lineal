import { createFileRoute, redirect } from "@tanstack/react-router";

export const Route = createFileRoute("/$ws/my-issues/")({
  beforeLoad: ({ params }) => {
    throw redirect({ to: `/${params.ws}/my-issues/assigned` as "/" });
  },
});
