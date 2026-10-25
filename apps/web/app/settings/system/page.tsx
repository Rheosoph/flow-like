"use client";

import { getPublicApiUrl } from "@/lib/public-config";
import { DiagnosticsPage } from "@flow-like/flow-like-ui/components/settings/diagnostics/diagnostics-page";
import { useSyncExternalStore } from "react";

const BUILD = {
	version: process.env.NEXT_PUBLIC_FLOW_LIKE_VERSION,
	commit: process.env.NEXT_PUBLIC_FLOW_LIKE_COMMIT,
	builtAt: process.env.NEXT_PUBLIC_FLOW_LIKE_BUILT_AT,
	mode: process.env.NODE_ENV,
};

const subscribeNever = () => () => {};

function readApiUrl() {
	try {
		return getPublicApiUrl();
	} catch {
		return null;
	}
}

export default function SystemPage() {
	const apiBaseUrl = useSyncExternalStore(
		subscribeNever,
		readApiUrl,
		() => null,
	);
	return (
		<DiagnosticsPage
			build={BUILD}
			apiBaseUrl={apiBaseUrl}
			storageHref="/settings/storage"
		/>
	);
}
