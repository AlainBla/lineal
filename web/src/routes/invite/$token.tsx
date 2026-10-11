import { createFileRoute, useNavigate } from "@tanstack/react-router";

import { SESSION_QUERY_KEY, useSession } from "@/app/auth";
import { queryClient } from "@/app/query-client";
import { InvitePage } from "@/features/invite/invite-page";

/** Öffentlich: der Auth-Guard im Root lässt `/invite/…` ohne Session durch. */
export const Route = createFileRoute("/invite/$token")({ component: Page });

function Page() {
  const { token } = Route.useParams();
  const session = useSession();
  const navigate = useNavigate();
  return (
    <InvitePage
      token={token}
      session={session}
      onAccepted={(workspace) => {
        queryClient.removeQueries({ queryKey: SESSION_QUERY_KEY });
        void navigate({ to: (workspace ? `/${workspace}/inbox` : "/") as "/" });
      }}
    />
  );
}
