"use client";

import { PageBuilderSurface } from "@flow-like/flow-like-ui";
import { useRouter, useSearchParams } from "next/navigation";
import { useMemo } from "react";

export default function PageBuilderPage() {
	const searchParams = useSearchParams();
	const router = useRouter();

	const { pageId, appId, boardId } = useMemo(() => {
		const pageId = searchParams.get("id") ?? "";
		const appId = searchParams.get("app") ?? "";
		const boardId = searchParams.get("board") ?? undefined;
		return { pageId, appId, boardId };
	}, [searchParams]);

	return (
		<PageBuilderSurface
			appId={appId}
			pageId={pageId}
			boardId={boardId}
			onClose={() => router.push(`/library/config/pages?id=${appId}`)}
			onOpenFlow={() => router.push(`/flow?id=${boardId}&app=${appId}`)}
			onOpenBoard={() => router.push(`/board?id=${boardId}`)}
			onPageChange={(newPageId) => {
				window.location.href = `/page-builder?id=${newPageId}&app=${appId}${boardId ? `&board=${boardId}` : ""}`;
			}}
		/>
	);
}
