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
import { MonitorCog, ShieldAlert } from "lucide-react";
import type { RpaConsentContext, RpaConsentRememberScope } from "./rpa-consent";

type RpaConsentDialogProps = {
	boardId?: string;
	context: RpaConsentContext;
	eventId?: string;
	onCancel: () => void;
	onConfirm: (rememberFor: RpaConsentRememberScope) => void;
	open: boolean;
};

export function RpaConsentDialog({
	boardId,
	context,
	eventId,
	onCancel,
	onConfirm,
	open,
}: RpaConsentDialogProps) {
	const isEventRegistration = context === "event_registration";
	const title = isEventRegistration
		? "Allow event automation"
		: "Allow computer automation";
	const description = isEventRegistration
		? "This event can run local computer automation when it is triggered. Approve it now so API, chat, and scheduled triggers can run without a foreground prompt."
		: "This workflow can control the local computer and read the screen. Approve this run only if you trust the board.";

	return (
		<Dialog open={open} onOpenChange={(nextOpen) => !nextOpen && onCancel()}>
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
							<p className="font-medium">Requested capability</p>
							<p className="text-muted-foreground">
								Mouse and keyboard automation, screenshot capture, and UI
								inspection for local RPA nodes.
							</p>
							{eventId ? (
								<p className="text-xs text-muted-foreground">
									Event: <span className="font-mono">{eventId}</span>
								</p>
							) : null}
							{boardId ? (
								<p className="text-xs text-muted-foreground">
									Board: <span className="font-mono">{boardId}</span>
								</p>
							) : null}
						</div>
					</div>
				</div>

				<DialogFooter className="flex-col gap-2 sm:flex-col">
					<div className="flex flex-wrap justify-end gap-2">
						<Button variant="outline" onClick={onCancel}>
							Cancel
						</Button>
						{!isEventRegistration ? (
							<Button variant="secondary" onClick={() => onConfirm("none")}>
								Run once
							</Button>
						) : null}
						{eventId ? (
							<Button variant="secondary" onClick={() => onConfirm("event")}>
								Remember for this event
							</Button>
						) : null}
						<Button onClick={() => onConfirm("board")}>
							Remember for this board
						</Button>
					</div>
					<p className="text-right text-xs text-muted-foreground">
						Remembered approvals are stored locally on this desktop.
					</p>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
