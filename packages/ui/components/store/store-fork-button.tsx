"use client";

import { GitForkIcon } from "lucide-react";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
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

function isForkAvailable(
	preview: IForkPreviewResponse | null | undefined,
): preview is IForkPreviewResponse {
	return Boolean(
		preview?.allow_forking && preview.user_can_fork && preview.within_limits,
	);
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
	const targetsKey = targets?.join("|") ?? "";
	const targetOptions = useMemo(
		() =>
			normalizeForkTargetOptions(
				target,
				targetsKey
					? (targetsKey.split("|") as IForkPreviewTarget[])
					: undefined,
			),
		[target, targetsKey],
	);
	const targetValues = useMemo(
		() => targetOptions.map((option) => option.value),
		[targetOptions],
	);
	const targetValuesKey = targetValues.join("|");
	const [availabilityByTarget, setAvailabilityByTarget] = useState<
		Partial<Record<IForkPreviewTarget, IForkPreviewResponse | null>>
	>({});
	const [availabilitySettled, setAvailabilitySettled] = useState(false);

	useEffect(() => {
		if (!targetOptions.some((option) => option.value === selectedTarget)) {
			setSelectedTarget(target);
		}
	}, [selectedTarget, target, targetOptions]);

	const loadPreview = useCallback(
		() => backend.appState.getForkPreview(appId, selectedTarget),
		[backend.appState, appId, selectedTarget],
	);

	useEffect(() => {
		if (!hideUnlessAvailable) return;

		let cancelled = false;
		setAvailabilitySettled(false);
		setAvailabilityByTarget({});

		Promise.all(
			targetValues.map(async (previewTarget) => {
				try {
					const preview = await backend.appState.getForkPreview(
						appId,
						previewTarget,
					);
					return [previewTarget, preview] as const;
				} catch (error) {
					console.warn(
						`Failed to load fork availability for ${previewTarget}:`,
						error,
					);
					return [previewTarget, null] as const;
				}
			}),
		).then((entries) => {
			if (cancelled) return;

			const nextAvailability = Object.fromEntries(entries) as Partial<
				Record<IForkPreviewTarget, IForkPreviewResponse | null>
			>;
			setAvailabilityByTarget(nextAvailability);
			setAvailabilitySettled(true);

			const firstAvailableTarget = entries.find(([, preview]) =>
				isForkAvailable(preview),
			)?.[0];

			setSelectedTarget((current) => {
				if (!firstAvailableTarget) return current;
				return isForkAvailable(nextAvailability[current])
					? current
					: firstAvailableTarget;
			});
		});

		return () => {
			cancelled = true;
		};
	}, [
		appId,
		backend.appState,
		hideUnlessAvailable,
		targetValues,
		targetValuesKey,
		auth?.isAuthenticated,
		auth?.user?.profile?.sub,
	]);

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
		const hasAvailableTarget =
			availabilitySettled &&
			Object.values(availabilityByTarget).some(isForkAvailable);
		if (!hasAvailableTarget) {
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
