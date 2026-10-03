"use client";

import { useTranslation } from "@flow-like/locales";
import type { IEvent } from "../../../lib/schema/flow/event";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "../../ui/alert-dialog";
import { Button } from "../../ui/button";
import { useOptionalEventsDevices } from "../devices/events/events-devices";

/** Devices the app's Runs on data says serve the event right now. */
function useServingDevices(eventId: string | undefined): string[] {
	const devices = useOptionalEventsDevices();
	const row = eventId ? devices?.live?.rows.get(eventId) : undefined;
	return row?.served.map((served) => served.device) ?? [];
}

/** Deleting never stops a device's service, so the dialog says which ones keep it. */
export function EventDeleteDialog({
	event,
	onConfirm,
	onCancel,
}: Readonly<{
	event: IEvent | null;
	onConfirm: (eventId: string) => void;
	onCancel: () => void;
}>) {
	const { t } = useTranslation("settings");
	const devices = useOptionalEventsDevices();
	const serving = useServingDevices(event?.id);
	const [first] = serving;
	const others = serving.length - 1;
	const stillRuns = !first
		? null
		: others === 0
			? t("stillRunsOnDevice", "Still runs on {{device}}.", { device: first })
			: others === 1
				? t(
						"stillRunsOnDeviceAndOne",
						"Still runs on {{device}} and 1 other.",
						{ device: first },
					)
				: t(
						"stillRunsOnDeviceAndOthers",
						"Still runs on {{device}} and {{count}} others.",
						{ device: first, count: others },
					);

	return (
		<AlertDialog
			open={event !== null}
			onOpenChange={(open) => {
				if (!open) onCancel();
			}}
		>
			<AlertDialogContent>
				<AlertDialogHeader>
					<AlertDialogTitle>
						{t("deleteEventNamed", "Delete {{name}}?", {
							name: event?.name ?? "",
						})}
					</AlertDialogTitle>
					<AlertDialogDescription>
						{stillRuns
							? `${stillRuns} ${t(
									"deletingDoesNotStopServices",
									"Deleting the event does not stop those services.",
								)}`
							: t(
									"deletingRemovesEventAndRoute",
									"This removes the event and its route from the app.",
								)}
					</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel>{t("cancel", "Cancel")}</AlertDialogCancel>
					{stillRuns && event && devices && (
						<Button
							variant="outline"
							onClick={() => {
								devices.go({
									screen: "app-devices",
									by: "event",
									eventId: event.id,
								});
								onCancel();
							}}
						>
							{t("openDevices", "Open Devices")}
						</Button>
					)}
					<AlertDialogAction
						className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
						onClick={() => event && onConfirm(event.id)}
					>
						{t("deleteEvent", "Delete event")}
					</AlertDialogAction>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
