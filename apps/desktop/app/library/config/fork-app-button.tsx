"use client";

import {
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	type IApp,
	IAppVisibility,
} from "@flow-like/flow-like-ui";
import { ForkAppCard } from "@flow-like/flow-like-ui/components/settings/forking/fork-app-card";
import { GitForkIcon, Loader2Icon } from "lucide-react";
import { useApplyForkBundle } from "../../../lib/use-apply-fork-bundle";
import { useOfflineToOnlineFork } from "../../../lib/use-offline-to-online-fork";

interface ForkAppButtonProps {
	localApp: IApp;
	appName: string;
}

/**
 * Desktop's online-source fork entry point. The shared card lets the
 * user choose an online or local destination; this wrapper only owns
 * the Tauri-side bundle-apply step for local destination forks.
 */
export function ForkAppButton({
	localApp,
	appName,
}: Readonly<ForkAppButtonProps>) {
	const applyBundle = useApplyForkBundle();
	const { forkOfflineAppOnline, isForking } = useOfflineToOnlineFork();
	if (localApp.visibility === IAppVisibility.Offline) {
		return (
			<Card>
				<CardHeader className="flex flex-row items-start justify-between gap-4 space-y-0">
					<div className="space-y-1">
						<CardTitle className="flex items-center gap-2">
							<GitForkIcon className="w-4 h-4" />
							Create an online copy
						</CardTitle>
						<CardDescription>
							Upload a fresh, secret-stripped copy of this local app to your
							account. The local app remains unchanged.
						</CardDescription>
					</div>
				</CardHeader>
				<CardContent>
					<Button
						onClick={() => forkOfflineAppOnline(localApp.id, appName)}
						disabled={isForking}
						className="gap-2"
					>
						{isForking ? (
							<Loader2Icon className="w-4 h-4 animate-spin" />
						) : (
							<GitForkIcon className="w-4 h-4" />
						)}
						Create online copy
					</Button>
				</CardContent>
			</Card>
		);
	}

	return (
		<ForkAppCard
			appId={localApp.id}
			appName={appName}
			target="offline"
			targets={["offline", "online"]}
			onForkStarted={applyBundle}
		/>
	);
}
