"use client";

import type { IApp } from "@tm9657/flow-like-ui";
import { ForkAppCard } from "@tm9657/flow-like-ui/components/settings/forking/fork-app-card";
import { useApplyForkBundle } from "../../../lib/use-apply-fork-bundle";

interface ForkAppButtonProps {
	localApp: IApp;
	appName: string;
}

/**
 * Desktop's offline fork entry point. The shared {@link ForkAppCard}
 * renders the card+dialog UI and drives the begin-fork call; this
 * wrapper only owns the Tauri-side bundle-apply step that has no
 * web equivalent.
 */
export function ForkAppButton({
	localApp,
	appName,
}: Readonly<ForkAppButtonProps>) {
	const applyBundle = useApplyForkBundle();
	return (
		<ForkAppCard
			appId={localApp.id}
			appName={appName}
			target="offline"
			onForkStarted={applyBundle}
		/>
	);
}
