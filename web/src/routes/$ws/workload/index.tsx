import { createFileRoute } from "@tanstack/react-router";

import { WorkloadPage } from "@/features/overview/workload-page";

export const Route = createFileRoute("/$ws/workload/")({ component: WorkloadPage });
