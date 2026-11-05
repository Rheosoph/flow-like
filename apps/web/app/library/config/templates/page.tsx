"use client";

import { TemplatesPage } from "@flow-like/flow-like-ui";
import { useRouter, useSearchParams } from "next/navigation";
import { useCallback } from "react";

export default function Page() {
	const router = useRouter();
	const searchParams = useSearchParams();
	const appId = searchParams.get("id") ?? "";
	const templateId = searchParams.get("templateId");

	const openFlow = useCallback(
		(boardId: string) => router.push(`/flow?id=${boardId}&app=${appId}`),
		[appId, router],
	);

	return (
		<TemplatesPage
			appId={appId}
			templateId={templateId}
			onOpenFlow={openFlow}
		/>
	);
}
