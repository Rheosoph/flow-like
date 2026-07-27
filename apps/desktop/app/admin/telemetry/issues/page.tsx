"use client";

import { AdminTelemetryIssuesPage, Skeleton } from "@flow-like/flow-like-ui";
import { Suspense } from "react";

export default function AdminTelemetryIssuesRoute() {
	return (
		<Suspense
			fallback={
				<main className="flex h-full w-full items-center justify-center bg-background">
					<Skeleton className="h-32 w-64" />
				</main>
			}
		>
			<AdminTelemetryIssuesPage />
		</Suspense>
	);
}
