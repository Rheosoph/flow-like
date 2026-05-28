"use client";

import {
	AdminAppRequestDetail,
	AdminPublicationsPage,
	Skeleton,
} from "@flow-like/flow-like-ui";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useCallback } from "react";

function RequestDetailWrapper() {
	const searchParams = useSearchParams();
	const router = useRouter();
	const requestId = searchParams.get("id") ?? "";

	const handleBack = useCallback(() => router.back(), [router]);

	return <AdminAppRequestDetail requestId={requestId} onBack={handleBack} />;
}

function ListContent() {
	const router = useRouter();

	const handleNavigateToPackage = useCallback(
		(packageId: string) => {
			router.push(`/admin/packages?id=${encodeURIComponent(packageId)}`);
		},
		[router],
	);

	const handleSelectRequest = useCallback(
		(requestId: string) => {
			router.push(`/admin/governance?id=${encodeURIComponent(requestId)}`);
		},
		[router],
	);

	return (
		<AdminPublicationsPage
			onNavigateToPackage={handleNavigateToPackage}
			onSelectRequest={handleSelectRequest}
		/>
	);
}

function PageContent() {
	const searchParams = useSearchParams();
	const requestId = searchParams.get("id");

	if (requestId) {
		return (
			<Suspense fallback={<Skeleton className="h-full w-full" />}>
				<RequestDetailWrapper />
			</Suspense>
		);
	}

	return <ListContent />;
}

export default function AdminGovernancePage() {
	return (
		<Suspense fallback={<Skeleton className="h-full w-full" />}>
			<PageContent />
		</Suspense>
	);
}
