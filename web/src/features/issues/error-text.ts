import { GraphQLRequestError } from "@/lib/graphql";

export function errorText(err: unknown): string {
  if (err instanceof GraphQLRequestError) {
    if (err.code === "not_found") return "This page does not exist or you do not have access.";
    if (err.code === "unauthenticated") return "Your session has expired. Sign in again.";
    return err.message;
  }
  return err instanceof Error ? err.message : "Something went wrong.";
}
