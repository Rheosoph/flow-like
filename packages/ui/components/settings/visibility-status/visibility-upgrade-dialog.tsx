"use client";

import { ArrowRightIcon, LockIcon } from "lucide-react";
import { useCallback, useState } from "react";
import { toast } from "sonner";
import { useInvalidateInvoke } from "../../../hooks";
import { useBackend } from "../../../state/backend-state";
import type { IAppVisibility } from "../../../types";
import {
	AlertDialog,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "../../ui/alert-dialog";
import { Button } from "../../ui/button";
import { VISIBILITY_META } from "./visibility-meta";

export interface VisibilityUpgradeDialogProps {
	appId: string;
	open: boolean;
	onOpenChange: (open: boolean) => void;
	/** Section the user tried to reach, e.g. "Team". */
	feature: string;
	/** One sentence on what the section needs the new visibility for. */
	reason: string;
	current: IAppVisibility;
	target: IAppVisibility;
	/** Fired after the switch landed, e.g. to refresh a local visibility cache. */
	onChanged?: (visibility: IAppVisibility) => void | Promise<void>;
}

/**
 * Confirm dialog behind a locked configuration section: instead of hiding the
 * section for apps that are not shared yet, the nav keeps it visible and this
 * dialog offers the one visibility change that unlocks it.
 */
export function VisibilityUpgradeDialog({
	appId,
	open,
	onOpenChange,
	feature,
	reason,
	current,
	target,
	onChanged,
}: Readonly<VisibilityUpgradeDialogProps>) {
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const [pending, setPending] = useState(false);

	const currentMeta = VISIBILITY_META[current];
	const targetMeta = VISIBILITY_META[target];

	const confirm = useCallback(async () => {
		setPending(true);
		try {
			await backend.appState.changeAppVisibility(appId, target);
			await invalidate(backend.appState.getApp, [appId]);
			await invalidate(backend.appState.getApps, []);
			await onChanged?.(target);
			toast.success(`Visibility changed to ${targetMeta.title}`, {
				icon: <targetMeta.Icon className="w-4 h-4" />,
			});
			onOpenChange(false);
		} catch (error) {
			toast.error(
				error instanceof Error
					? error.message
					: "Could not change the visibility",
			);
		} finally {
			setPending(false);
		}
	}, [
		appId,
		backend.appState,
		invalidate,
		onChanged,
		onOpenChange,
		target,
		targetMeta,
	]);

	return (
		<AlertDialog
			open={open}
			onOpenChange={(next) => {
				if (pending) return;
				onOpenChange(next);
			}}
		>
			<AlertDialogContent className="sm:max-w-md">
				<AlertDialogHeader>
					<div className="flex items-center gap-3">
						<div className="p-2 rounded-full bg-primary/10">
							<LockIcon className="h-5 w-5 text-primary" />
						</div>
						<AlertDialogTitle className="text-left">
							{feature} needs {targetMeta.title}
						</AlertDialogTitle>
					</div>
					<AlertDialogDescription className="text-left text-muted-foreground">
						{reason}
					</AlertDialogDescription>
				</AlertDialogHeader>

				<div className="space-y-3 py-2">
					<div className="flex items-center justify-center gap-2 p-3 bg-muted rounded-lg">
						<div className={`w-2 h-2 rounded-full ${currentMeta.color}`} />
						<span className="text-sm font-medium">{currentMeta.title}</span>
						<ArrowRightIcon className="w-4 h-4 text-muted-foreground" />
						<div className={`w-2 h-2 rounded-full ${targetMeta.color}`} />
						<span className="text-sm font-medium">{targetMeta.title}</span>
					</div>
					<p className="text-xs text-muted-foreground">
						{targetMeta.description}. You can switch back to {currentMeta.title}{" "}
						at any time — that removes everyone you invited.
					</p>
				</div>

				<AlertDialogFooter className="flex-col sm:flex-row gap-2">
					<Button
						variant="outline"
						className="w-full sm:w-auto"
						disabled={pending}
						onClick={() => onOpenChange(false)}
					>
						Cancel
					</Button>
					<Button
						className="w-full sm:w-auto"
						disabled={pending}
						onClick={confirm}
					>
						{pending ? "Switching…" : `Switch to ${targetMeta.title}`}
					</Button>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
