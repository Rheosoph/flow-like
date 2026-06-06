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

	const handleRegistryModelOpen = useCallback(
		(provider: string, modelId: string) => {
			const next = new URLSearchParams();
			next.set("tab", "registry");
			next.set("provider", provider);
			next.set("modelId", modelId);
			router.replace(`/admin/ai-act?${next.toString()}`);
		},
		[router],
	);

	return (
		<AdminAiActInventoryPage
			initialAppId={params.get("id")}
			initialTab={tab}
			initialRegistryProvider={params.get("provider")}
			initialRegistryModelId={params.get("modelId")}
			onAppChange={handleAppChange}
			onRegistryModelOpen={handleRegistryModelOpen}
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
