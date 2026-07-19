"use client";

import { AdminSuitePublicationsPage, Skeleton } from "@flow-like/flow-like-ui";
import { Suspense } from "react";

export default function AdminGovernanceSuitesPage() {
	return (
		<Suspense fallback={<Skeleton className="h-full w-full" />}>
			<AdminSuitePublicationsPage />
		</Suspense>
	);
}
