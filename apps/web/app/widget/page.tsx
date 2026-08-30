"use client";

import { WidgetBuilderSurface } from "@flow-like/flow-like-ui";
import type { Version } from "@flow-like/flow-like-ui";
import { useRouter, useSearchParams } from "next/navigation";
import { useMemo } from "react";

export default function WidgetEditorPage() {
	const searchParams = useSearchParams();
	const router = useRouter();

	const { widgetId, appId, version } = useMemo(() => {
		const widgetId = searchParams.get("id") ?? "";
		const appId = searchParams.get("app") ?? "";
		let version: Version | undefined;
		const versionStr = searchParams.get("version");
		if (versionStr) {
			const parts = versionStr.split("_").map(Number);
			if (parts.length === 3) {
				version = parts as Version;
			}
		}
		return { widgetId, appId, version };
	}, [searchParams]);

	return (
		<WidgetBuilderSurface
			appId={appId}
			widgetId={widgetId}
			version={version}
			onClose={() => router.push(`/library/config/widgets?id=${appId}`)}
			onSwitchVersion={(versionStr) =>
				router.push(`/widget?id=${widgetId}&app=${appId}&version=${versionStr}`)
			}
		/>
	);
}
