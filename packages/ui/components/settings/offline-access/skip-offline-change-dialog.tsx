"use client";

import { useTranslation } from "@flow-like/locales";
import { TriangleAlertIcon } from "lucide-react";
import { useEffect, useId, useState } from "react";
import type { IOfflineOperation } from "../../../state/backend-state/offline-writes-state";
import { Button } from "../../ui/button";
import { Checkbox } from "../../ui/checkbox";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";
import { Label } from "../../ui/label";
import { Textarea } from "../../ui/textarea";
import { SKIP_REASON_MAX_LENGTH, canSubmitSkip } from "./offline-access-logic";

export function SkipOfflineChangeDialog({
	operation,
	onOpenChange,
	onSkip,
}: Readonly<{
	operation: IOfflineOperation | null;
	onOpenChange: (open: boolean) => void;
	onSkip: (reason: string, acknowledgeUncertain: boolean) => Promise<void>;
}>) {
	const { t } = useTranslation("settings");
	const reasonId = useId();
	const acknowledgeId = useId();
	const [reason, setReason] = useState("");
	const [acknowledged, setAcknowledged] = useState(false);
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string>();

	const operationId = operation?.operationId;
	useEffect(() => {
		if (!operationId) return;
		setReason("");
		setAcknowledged(false);
		setError(undefined);
	}, [operationId]);

	const attempts = operation?.attempts ?? 0;
	const uncertain = attempts > 0;
	const ready = canSubmitSkip(reason, attempts, acknowledged);

	const submit = async () => {
		if (!ready || busy) return;
		setBusy(true);
		setError(undefined);
		try {
			await onSkip(reason.trim(), uncertain && acknowledged);
			onOpenChange(false);
		} catch (cause) {
			setError(cause instanceof Error ? cause.message : String(cause));
		} finally {
			setBusy(false);
		}
	};

	return (
		<Dialog open={operation !== null} onOpenChange={onOpenChange}>
			<DialogContent>
				<DialogHeader>
					<DialogTitle>
						{t("settings:offlineAccess.skipTitle", "Skip this change?")}
					</DialogTitle>
					<DialogDescription>
						{t(
							"settings:offlineAccess.skipBody",
							"The change is removed from this device and never sent. Later changes to the same table or file continue to sync.",
						)}
					</DialogDescription>
				</DialogHeader>
				<form
					className="space-y-4"
					onSubmit={(event) => {
						event.preventDefault();
						void submit();
					}}
				>
					<div className="space-y-2">
						<Label htmlFor={reasonId}>
							{t("settings:offlineAccess.skipReason", "Reason")}
						</Label>
						<Textarea
							id={reasonId}
							value={reason}
							maxLength={SKIP_REASON_MAX_LENGTH}
							required
							disabled={busy}
							placeholder={t(
								"settings:offlineAccess.skipReasonPlaceholder",
								"Why are you skipping this change?",
							)}
							onChange={(event) => setReason(event.target.value)}
						/>
					</div>
					{uncertain && (
						<div className="space-y-2 rounded-md border border-destructive/40 bg-destructive/5 p-3">
							<p className="flex items-start gap-2 text-sm text-destructive">
								<TriangleAlertIcon className="mt-0.5 size-4 shrink-0" />
								{t(
									"settings:offlineAccess.skipUncertain",
									"This change may already have reached the cloud. Skipping cannot undo it there.",
								)}
							</p>
							<div className="flex items-start gap-2">
								<Checkbox
									id={acknowledgeId}
									checked={acknowledged}
									disabled={busy}
									onCheckedChange={(value) => setAcknowledged(value === true)}
								/>
								<Label
									htmlFor={acknowledgeId}
									className="text-sm font-normal leading-snug"
								>
									{t(
										"settings:offlineAccess.skipAcknowledge",
										"I understand that skipping cannot undo a change the cloud already applied",
									)}
								</Label>
							</div>
						</div>
					)}
					{error && (
						<p role="alert" className="text-sm text-destructive">
							{error}
						</p>
					)}
					<DialogFooter>
						<Button
							type="button"
							variant="outline"
							disabled={busy}
							onClick={() => onOpenChange(false)}
						>
							{t("common:cancel", "Cancel")}
						</Button>
						<Button
							type="submit"
							variant="destructive"
							disabled={!ready || busy}
						>
							{t("settings:offlineAccess.skip", "Skip change")}
						</Button>
					</DialogFooter>
				</form>
			</DialogContent>
		</Dialog>
	);
}
