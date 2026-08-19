"use client";

import { useTranslation } from "@flow-like/locales";
import { GitForkIcon } from "lucide-react";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
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
import {
	ForkAppDialog,
	type IBeginForkResponse,
	normalizeForkTargetOptions,
} from "./fork-app-dialog";

export interface ForkAppCardProps {
	appId: string;
	appName: string;
	/**
	 * Which fork target to wire up:
	 * - "online" → server keeps the new app on the user's account
	 *   (web default). After the fork begins the card navigates to the
	 *   new app's config page.
	 * - "offline" → server materializes a signed bundle the host pulls
	 *   to disk (desktop). The host MUST supply `onForkStarted` so the
	 *   bundle can actually land somewhere.
	 */
	target: IForkPreviewTarget;
	/**
	 * Optional destination choices. Use this in desktop where a fork
	 * can land either online or on the local device. The `target` prop
	 * remains the initial choice.
	 */
	targets?: readonly IForkPreviewTarget[];
	/**
	 * Fires after `beginFork` resolves. Required for `offline` to apply
	 * the bundle locally. For `online`, the callback runs first and the
	 * card still navigates to the fork's config page afterward.
	 */
	onForkStarted?: (response: IBeginForkResponse) => void;
}

export function ForkAppCard({
	appId,
	appName,
	target,
	targets,
	onForkStarted,
}: Readonly<ForkAppCardProps>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
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

	return (
		<Card>
			<CardHeader className="flex flex-row items-start justify-between gap-4 space-y-0">
				<div className="space-y-1">
					<CardTitle className="flex items-center gap-2">
						<GitForkIcon className="w-4 h-4" />
						{t('createAFork', 'Create a fork')}
					</CardTitle>
					<CardDescription>
						{targetOptions.length > 1
							? t('chooseWhereToCreateAPersonalCopyOfThisApp', 'Choose where to create a personal copy of this app.')
							: selectedTarget === "offline"
								? t('makeALocalCopyOfThisAppOnThisDevice', 'Make a local copy of this app on this device.')
								: t('makeAPersonalCopyOfThisAppOnYourAccount', 'Make a personal copy of this app on your account.')}{" "}
						{t('secretsAreStrippedAndOauthBindingsAreClearedSoTheForkNeverCarriesTheSourcesCredentials', "Secrets are stripped and OAuth bindings are cleared so the fork never carries the source's credentials.")}
					</CardDescription>
				</div>
			</CardHeader>
			<CardContent>
				<Button onClick={() => setOpen(true)} className="gap-2">
					<GitForkIcon className="w-4 h-4" />
					{t('previewFork', 'Preview & fork')}
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
			</CardContent>
		</Card>
	);
}
