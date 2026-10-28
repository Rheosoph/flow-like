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
import { KeyRound, Radio, ShieldAlert } from "lucide-react";
import type { LocalSinkConsentRememberScope } from "./local-sink-consent";

type TriggerSummaryProps = {
	eventName?: string;
	eventType?: string;
};

type LocalSinkConsentDialogProps = TriggerSummaryProps & {
	/** The trigger signs in with a token, which the next step asks for. */
	needsToken?: boolean;
	/** The app whose trigger for this event runs here now. */
	movesFromApp?: string;
	/** The explicit "Don't register" choice: the event is saved without a trigger. */
	onDecline: () => void;
	/** Closed without a choice (Escape, outside click): the save is abandoned. */
	onDismiss: () => void;
	onConfirm: (rememberFor: LocalSinkConsentRememberScope) => void;
	open: boolean;
};

type LocalTriggerTokenDialogProps = TriggerSummaryProps & {
	/** "Save without trigger": the event is saved without a trigger here. */
	onDecline: () => void;
	/** Closed without a choice (Escape, outside click): the save is abandoned. */
	onDismiss: () => void;
	onChooseToken: () => void;
	open: boolean;
};

function TriggerSummary({ eventName, eventType }: TriggerSummaryProps) {
	const { t } = useTranslation("common");
	return (
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
	);
}

export function LocalSinkConsentDialog({
	eventName,
	eventType,
	needsToken,
	movesFromApp,
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

				<TriggerSummary eventName={eventName} eventType={eventType} />

				{movesFromApp && (
					<p className="text-sm text-muted-foreground">
						{t(
							"localTriggerMovesFromApp",
							"This computer runs this trigger for {{app}} now. Allowing it moves the trigger to this app.",
							{ app: movesFromApp },
						)}
					</p>
				)}
				{needsToken && (
					<p className="text-sm text-muted-foreground">
						{t(
							"localTriggerChoosesTokenNext",
							"It signs in to the hub with a personal access token, which you choose next.",
						)}
					</p>
				)}

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

/** Asks for the token a trigger signs in with when no approval comes first. */
export function LocalTriggerTokenDialog({
	eventName,
	eventType,
	onDecline,
	onDismiss,
	onChooseToken,
	open,
}: LocalTriggerTokenDialogProps) {
	const { t } = useTranslation("common");

	return (
		<Dialog open={open} onOpenChange={(nextOpen) => !nextOpen && onDismiss()}>
			<DialogContent className="max-w-lg">
				<DialogHeader>
					<div className="flex items-center gap-2">
						<KeyRound className="h-5 w-5 text-muted-foreground" />
						<DialogTitle>
							{t("localTriggerTokenTitle", "Choose a token for {{name}}", {
								name: eventName,
							})}
						</DialogTitle>
					</div>
					<DialogDescription>
						{t(
							"localTriggerTokenDescription",
							"This trigger runs on this computer and signs in to the hub with a personal access token. Without one, the event is saved but its trigger doesn't run here.",
						)}
					</DialogDescription>
				</DialogHeader>

				<TriggerSummary eventName={eventName} eventType={eventType} />

				<DialogFooter className="flex-wrap gap-2">
					<Button variant="outline" onClick={onDecline}>
						{t("saveWithoutTrigger", "Save without trigger")}
					</Button>
					<Button onClick={onChooseToken}>
						{t("settings:addToken", "Add token")}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
