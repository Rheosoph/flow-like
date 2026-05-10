"use client";

import { GitForkIcon } from "lucide-react";
import { useRouter } from "next/navigation";
import { useCallback, useState } from "react";
import type {
	IForkPreviewTarget,
	IOnlineForkBody,
} from "../../../lib/schema/app/fork";
import { useBackend } from "../../../state/backend-state";
import { Button } from "../../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../../ui/card";
import { ForkAppDialog, type IBeginForkResponse } from "./fork-app-dialog";

export interface ForkAppCardProps {
	appId: string;
	appName: string;
	/**
	 * Which fork target to wire up:
	 * - "online" → server keeps the new app on the user's account
	 *   (web default). After the fork begins the card navigates to the
	 *   new app's config unless `onForkStarted` is provided.
	 * - "offline" → server materializes a signed bundle the host pulls
	 *   to disk (desktop). The host MUST supply `onForkStarted` so the
	 *   bundle can actually land somewhere.
	 */
	target: IForkPreviewTarget;
	/**
	 * Fires after `beginFork` resolves. Required for `offline` to apply
	 * the bundle locally; optional for `online` (default action is
	 * `router.push("/library/config?id=<new_app_id>")`).
	 */
	onForkStarted?: (response: IBeginForkResponse) => void;
}

export function ForkAppCard({
	appId,
	appName,
	target,
	onForkStarted,
}: Readonly<ForkAppCardProps>) {
	const backend = useBackend();
	const router = useRouter();
	const [open, setOpen] = useState(false);

	const loadPreview = useCallback(
		() => backend.appState.getForkPreview(appId, target),
		[backend.appState, appId, target],
	);

	const beginFork = useCallback(
		(body: IOnlineForkBody): Promise<IBeginForkResponse> => {
			if (target === "offline") {
				return backend.appState.beginOfflineFork(appId, body);
			}
			return backend.appState.onlineFork(appId, body);
		},
		[backend.appState, appId, target],
	);

	const handleForkStarted = useCallback(
		(response: IBeginForkResponse) => {
			if (onForkStarted) {
				onForkStarted(response);
				return;
			}
			if (target === "online") {
				router.push(`/library/config?id=${response.new_app_id}`);
			}
		},
		[onForkStarted, target, router],
	);

	return (
		<Card>
			<CardHeader className="flex flex-row items-start justify-between gap-4 space-y-0">
				<div className="space-y-1">
					<CardTitle className="flex items-center gap-2">
						<GitForkIcon className="w-4 h-4" />
						Create a fork
					</CardTitle>
					<CardDescription>
						Make a personal copy of this app on your account. Secrets are
						stripped and OAuth bindings are cleared so the fork never carries
						the source's credentials.
					</CardDescription>
				</div>
			</CardHeader>
			<CardContent>
				<Button onClick={() => setOpen(true)} className="gap-2">
					<GitForkIcon className="w-4 h-4" />
					Preview & fork
				</Button>
				<ForkAppDialog
					appId={appId}
					appName={appName}
					open={open}
					onOpenChange={setOpen}
					loadPreview={loadPreview}
					beginFork={beginFork}
					onForkStarted={handleForkStarted}
				/>
			</CardContent>
		</Card>
	);
}
