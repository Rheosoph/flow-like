"use client";

import {
	AppReviewsSection,
	useBackend,
	useInvalidateInvoke,
	useInvoke,
} from "@flow-like/flow-like-ui";
import { ProjectDashboard } from "@flow-like/flow-like-ui/components/settings/dashboard";
import { useRouter, useSearchParams } from "next/navigation";
import { useCallback } from "react";
import {
	AppAccessSection,
	AppComplianceSection,
} from "./visibility-status-switcher";

export default function LibraryConfigPage() {
	const backend = useBackend();
	const router = useRouter();
	const invalidate = useInvalidateInvoke();
	const searchParams = useSearchParams();
	const id = searchParams.get("id") ?? "";

	const app = useInvoke(
		backend.appState.getApp,
		backend.appState,
		[id],
		id.length > 0,
	);
	const metadata = useInvoke(
		backend.appState.getAppMeta,
		backend.appState,
		[id],
		id.length > 0,
	);

	const refreshApp = useCallback(async () => {
		await app.refetch();
		await invalidate(backend.appState.getApps, []);
	}, [app, invalidate, backend.appState]);

	if (!id || !app.data) {
		return (
			<ProjectDashboard appId={id} onDeleted={() => router.push("/library")} />
		);
	}

	return (
		<ProjectDashboard
			appId={id}
			onDeleted={() => router.push("/library")}
			slots={{
				access: (
					<AppAccessSection
						localApp={app.data}
						appName={metadata.data?.name ?? id}
						canEdit
						refreshApp={refreshApp}
					/>
				),
				compliance: <AppComplianceSection localApp={app.data} canEdit />,
				reviews: <AppReviewsSection appId={id} onReviewChanged={refreshApp} />,
			}}
		/>
	);
}
