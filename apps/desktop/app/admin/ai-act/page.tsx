"use client";

import { AdminAiActInventoryPage } from "@flow-like/flow-like-ui";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useCallback } from "react";

function AiActPage() {
	const router = useRouter();
	const params = useSearchParams();
	const tab = params.get("tab");

	const handleAppChange = useCallback(
		(appId: string | null) => {
			const next = new URLSearchParams();
			if (appId) next.set("id", appId);
			if (tab) next.set("tab", tab);
			const query = next.toString();
			router.replace(query ? `/admin/ai-act?${query}` : "/admin/ai-act");
		},
		[router, tab],
	);

	return (
		<AdminAiActInventoryPage
			initialAppId={params.get("id")}
			initialTab={tab}
			onAppChange={handleAppChange}
		/>
	);
}

export default function Page() {
	return (
		<Suspense fallback={null}>
			<AiActPage />
		</Suspense>
	);
}
