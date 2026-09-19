"use client";

import {
	Button,
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { Radio, ShieldAlert } from "lucide-react";
import type { LocalSinkConsentRememberScope } from "./local-sink-consent";

type LocalSinkConsentDialogProps = {
	eventName?: string;
	eventType?: string;
	/** The explicit "Don't register" choice: the event is saved without a trigger. */
	onDecline: () => void;
	/** Closed without a choice (Escape, outside click): the save is abandoned. */
	onDismiss: () => void;
	onConfirm: (rememberFor: LocalSinkConsentRememberScope) => void;
	open: boolean;
};

export function LocalSinkConsentDialog({
	eventName,
	eventType,
	onDecline,
	onDismiss,
	onConfirm,
	open,
}: LocalSinkConsentDialogProps) {
	const { t } = useTranslation("common");

	return (
		<Dialog open={open} onOpenChange={(nextOpen) => !nextOpen && onDismiss()}>
			<DialogContent className="max-w-lg">
				<DialogHeader>
					<div className="flex items-center gap-2">
						<ShieldAlert className="h-5 w-5 text-orange-500" />
						<DialogTitle>
							{t("allowLocalTrigger", "Allow local trigger")}
						</DialogTitle>
					</div>
					<DialogDescription>
						{t(
							"thisEventRegistersATriggerThatRunsOnThisDeviceItStartsNowAndAgainWheneverFlowLikeStartsUntilTheEventIsDisabled",
							"This event registers a trigger that runs on this device. It starts now and again whenever Flow-Like starts, until the event is disabled.",
						)}
					</DialogDescription>
				</DialogHeader>

				<div className="rounded-md border bg-muted/30 p-3 text-sm">
					<div className="flex items-start gap-3">
						<Radio className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
						<div className="space-y-1">
							<p className="font-medium">{eventName}</p>
							<p className="text-muted-foreground">
								{t("triggerType", "Trigger type:")}{" "}
								<span className="font-mono">{eventType}</span>
							</p>
						</div>
					</div>
				</div>

				<DialogFooter className="flex-col gap-2 sm:flex-col">
					<div className="flex flex-wrap justify-end gap-2">
						<Button variant="outline" onClick={onDecline}>
							{t("dontRegister", "Don't register")}
						</Button>
						<Button variant="secondary" onClick={() => onConfirm("none")}>
							{t("allow", "Allow")}
						</Button>
						<Button variant="secondary" onClick={() => onConfirm("event")}>
							{t("rememberForThisEvent", "Remember for this event")}
						</Button>
						<Button onClick={() => onConfirm("app")}>
							{t("rememberForThisProject", "Remember for this project")}
						</Button>
					</div>
					<p className="text-right text-xs text-muted-foreground">
						{t(
							"rememberedApprovalsAreStoredLocallyOnThisDesktop",
							"Remembered approvals are stored locally on this desktop.",
						)}
					</p>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
