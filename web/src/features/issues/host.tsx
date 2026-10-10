import { CreateIssueModal } from "./create-issue-modal";
import { Toaster } from "./toaster";

/**
 * Globale Überlagerungen der Issue-Screens: Create-Issue-Modal (öffnet über `createIssueOpen`, Taste `c`)
 * und Toasts. Wird einmal in der Shell eingehängt.
 */
export function IssuesHost() {
  return (
    <>
      <CreateIssueModal />
      <Toaster />
    </>
  );
}
