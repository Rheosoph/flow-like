"use client";

import { AdminLogsPage, Skeleton } from "@tm9657/flow-like-ui";
import { Suspense } from "react";

export default function AdminLogsRoute() {
	return (
		<Suspense
			fallback={
				<main className="flex h-full w-full items-center justify-center bg-background">
					<Skeleton className="h-32 w-64" />
				</main>
			}
		>
			<AdminLogsPage />
		</Suspense>
	);
}
