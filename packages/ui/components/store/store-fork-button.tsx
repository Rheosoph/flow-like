"use client";

import { GitForkIcon } from "lucide-react";
import { useRouter } from "next/navigation";
import { useCallback, useState } from "react";
import type {
	IForkPreviewTarget,
	IOnlineForkBody,
} from "../../lib/schema/app/fork";
import { useBackend } from "../../state/backend-state";
import {
	ForkAppDialog,
	type IBeginForkResponse,
} from "../settings/forking/fork-app-dialog";
import { Button } from "../ui/button";

export interface StoreForkButtonProps {
	appId: string;
	appName: string;
	target: IForkPreviewTarget;
	onForkStarted?: (response: IBeginForkResponse) => void;
	size?: "default" | "sm" | "lg" | "icon";
	variant?:
		| "default"
		| "outline"
		| "ghost"
		| "secondary"
		| "destructive"
		| "link";
	label?: string;
}

export function StoreForkButton({
	appId,
	appName,
	target,
	onForkStarted,
	size = "sm",
	variant = "outline",
	label = "Fork",
}: Readonly<StoreForkButtonProps>) {
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
		<>
			<Button size={size} variant={variant} onClick={() => setOpen(true)}>
				<GitForkIcon className="h-3.5 w-3.5 mr-1.5" />
				{label}
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
		</>
	);
}
