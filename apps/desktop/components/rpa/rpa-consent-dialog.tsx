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
import { MonitorCog, ShieldAlert } from "lucide-react";
import type {
	RpaCapability,
	RpaConsentContext,
	RpaConsentRememberScope,
} from "./rpa-consent";
import { capabilityLabels } from "./rpa-permission-dialog";

type RpaConsentDialogProps = {
	required?: RpaCapability[];
	pending?: boolean;
	error?: string | null;
	boardId?: string;
	context: RpaConsentContext;
	eventId?: string;
	onCancel: () => void;
	onConfirm: (rememberFor: RpaConsentRememberScope) => void;
	open: boolean;
};

export function RpaConsentDialog({
	boardId,
	required = [],
	pending = false,
	error,
	context,
	eventId,
	onCancel,
	onConfirm,
	open,
}: RpaConsentDialogProps) {
	const { t } = useTranslation("common");
	const isEventRegistration = context === "event_registration";
	const title = isEventRegistration
		? t("allowEventAutomation", "Allow event automation")
		: t("allowWorkflowAutomation", "Allow workflow automation");
	const description = isEventRegistration
		? t(
				"approveEventAutomationCapabilities",
				"This event can use the capabilities listed below when triggered. Approval allows background triggers to run this workflow revision.",
			)
		: t(
				"approveWorkflowAutomationCapabilities",
				"This workflow can use the capabilities listed below. Approve it only if you trust the board.",
			);

	return (
		<Dialog
			open={open}
			onOpenChange={(nextOpen) => !nextOpen && !pending && onCancel()}
		>
			<DialogContent className="max-w-lg">
				<DialogHeader>
					<div className="flex items-center gap-2">
						<ShieldAlert className="h-5 w-5 text-orange-500" />
						<DialogTitle>{title}</DialogTitle>
					</div>
					<DialogDescription>{description}</DialogDescription>
				</DialogHeader>

				<div className="rounded-md border bg-muted/30 p-3 text-sm">
					<div className="flex items-start gap-3">
						<MonitorCog className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
						<div className="space-y-1">
							<p className="font-medium">
								{t("requestedCapability", "Requested capability")}
							</p>
							<p className="text-muted-foreground">
								{required
									.map((capability) => capabilityLabels[capability])
									.join(", ")}
							</p>
							{eventId ? (
								<p className="text-xs text-muted-foreground">
									{t("event", "Event:")}{" "}
									<span className="font-mono">{eventId}</span>
								</p>
							) : null}
							{boardId ? (
								<p className="text-xs text-muted-foreground">
									{t("board", "Board:")}{" "}
									<span className="font-mono">{boardId}</span>
								</p>
							) : null}
						</div>
					</div>
				</div>

				{error && (
					<p role="alert" className="text-sm text-destructive">
						{error}
					</p>
				)}
				<DialogFooter className="flex-col gap-2 sm:flex-col">
					<div className="flex flex-wrap justify-end gap-2">
						<Button disabled={pending} variant="outline" onClick={onCancel}>
							{t("cancel", "Cancel")}
						</Button>
						{!isEventRegistration ? (
							<Button
								disabled={pending}
								variant="secondary"
								onClick={() => onConfirm("none")}
							>
								{t("runOnce", "Run once")}
							</Button>
						) : null}
						{eventId ? (
							<Button
								disabled={pending}
								variant="secondary"
								onClick={() => onConfirm("event")}
							>
								{t("rememberForThisEvent", "Remember for this event")}
							</Button>
						) : null}
						<Button disabled={pending} onClick={() => onConfirm("board")}>
							{t("rememberForThisBoard", "Remember for this board")}
						</Button>
					</div>
					<p className="text-right text-xs text-muted-foreground">
						{t(
							"rememberedApprovalsAreStoredLocallyOnThisDesktop",
							"Approvals are stored on this desktop for the current profile and workflow revision. Editing the workflow requires approval again.",
						)}
					</p>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
