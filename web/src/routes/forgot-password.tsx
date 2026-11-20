import { createFileRoute } from "@tanstack/react-router";

import { ForgotPasswordPage } from "@/features/password-reset/forgot-password-page";

/** Öffentlich: der Auth-Guard im Root lässt `/forgot-password` ohne Session durch. */
export const Route = createFileRoute("/forgot-password")({ component: ForgotPasswordPage });
