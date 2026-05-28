"use client";

import { GitForkIcon } from "lucide-react";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
import { useInvoke } from "../../hooks/use-invoke";
import type {
	IForkPreviewResponse,
	IForkPreviewTarget,
	IOnlineForkBody,
} from "../../lib/schema/app/fork";
import { useBackend } from "../../state/backend-state";
import {
	ForkAppDialog,
	type IBeginForkResponse,
	normalizeForkTargetOptions,
} from "../settings/forking/fork-app-dialog";
import { Button } from "../ui/button";

export interface StoreForkButtonProps {
	appId: string;
	appName: string;
	target: IForkPreviewTarget;
	targets?: readonly IForkPreviewTarget[];
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
	hideUnlessAvailable?: boolean;
}

export function StoreForkButton({
	appId,
	appName,
	target,
	targets,
	onForkStarted,
	size = "sm",
	variant = "outline",
	label = "Fork",
	hideUnlessAvailable = false,
}: Readonly<StoreForkButtonProps>) {
	const backend = useBackend();
	const auth = useAuth();
	const router = useRouter();
	const [open, setOpen] = useState(false);
	const [selectedTarget, setSelectedTarget] =
		useState<IForkPreviewTarget>(target);
	const targetOptions = useMemo(
		() => normalizeForkTargetOptions(target, targets),
		[target, targets],
	);

	useEffect(() => {
		if (!targetOptions.some((option) => option.value === selectedTarget)) {
			setSelectedTarget(target);
		}
	}, [selectedTarget, target, targetOptions]);

	const loadPreview = useCallback(
		() => backend.appState.getForkPreview(appId, selectedTarget),
		[backend.appState, appId, selectedTarget],
	);
	const availability = useInvoke<
		IForkPreviewResponse,
		[appId: string, target: IForkPreviewTarget]
	>(
		backend.appState.getForkPreview,
		backend.appState,
		[appId, selectedTarget],
		hideUnlessAvailable,
		[
			auth?.isAuthenticated ? "authenticated" : "anonymous",
			auth?.user?.profile?.sub,
		],
		30_000,
	);

	const beginFork = useCallback(
		(body: IOnlineForkBody): Promise<IBeginForkResponse> => {
			if (selectedTarget === "offline") {
				return backend.appState.beginOfflineFork(appId, body);
			}
			return backend.appState.onlineFork(appId, body);
		},
		[backend.appState, appId, selectedTarget],
	);

	const handleForkStarted = useCallback(
		(response: IBeginForkResponse) => {
			onForkStarted?.(response);
			if (selectedTarget === "online") {
				router.push(`/library/config?id=${response.new_app_id}`);
			}
		},
		[onForkStarted, selectedTarget, router],
	);

	if (hideUnlessAvailable) {
		const preview = availability.data;
		if (
			!preview?.allow_forking ||
			!preview.user_can_fork ||
			!preview.within_limits
		) {
			return null;
		}
	}

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
				target={selectedTarget}
				targetOptions={targetOptions}
				onTargetChange={setSelectedTarget}
				loadPreview={loadPreview}
				beginFork={beginFork}
				onForkStarted={handleForkStarted}
			/>
		</>
	);
}
