"use client";

import { useTranslation } from "@flow-like/locales";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useRef,
	useState,
} from "react";
import { toast } from "sonner";
import type { IEvent } from "../../../lib/schema/flow/event";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";

/** What the form inside the dialog reports back to the shell. */
export interface NewEventShell {
	onSavedChange(event: IEvent | null): void;
	onBusyChange(busy: boolean): void;
	/** The event was created and, if it was meant to, deployed. */
	onDeploymentComplete(): void;
	onCancel(): void;
}

/**
 * The New event dialog. Once the form has saved a device event the dialog turns
 * into the deployment, can't be dismissed while a step is busy, and says where
 * to finish if it is closed first.
 */
export function NewEventDialog({
	open,
	onOpenChange,
	onDeployed,
	children,
}: Readonly<{
	open: boolean;
	onOpenChange: (open: boolean) => void;
	onDeployed?: () => void;
	children: (shell: NewEventShell) => ReactNode;
}>) {
	const { t } = useTranslation("settings");
	const [busy, setBusy] = useState(false);
	const [saved, setSaved] = useState<IEvent | null>(null);
	const savedRef = useRef<IEvent | null>(null);
	const deployedRef = useRef(false);

	useEffect(() => {
		if (open) return;
		setBusy(false);
		setSaved(null);
		savedRef.current = null;
		deployedRef.current = false;
	}, [open]);

	const close = useCallback(() => {
		const event = savedRef.current;
		if (event && !deployedRef.current)
			toast.info(
				t(
					"eventSavedDeployFromRunsOn",
					"{{name}} was saved. Deploy it from its Runs on column.",
					{ name: event.name },
				),
			);
		onOpenChange(false);
	}, [onOpenChange, t]);

	const shell: NewEventShell = {
		onSavedChange: (event) => {
			savedRef.current = event;
			setSaved(event);
			if (!event) setBusy(false);
		},
		onBusyChange: setBusy,
		onDeploymentComplete: () => {
			deployedRef.current = true;
			onDeployed?.();
			onOpenChange(false);
		},
		onCancel: close,
	};

	return (
		<Dialog
			open={open}
			onOpenChange={(next) => {
				if (busy) return;
				if (next) onOpenChange(true);
				else close();
			}}
		>
			<DialogContent
				className="w-[calc(100vw-2rem)] max-w-[1100px] gap-0 overflow-hidden p-0 sm:max-w-[1100px]"
				showCloseButton={!busy}
			>
				<DialogHeader className="border-b px-5 py-5 pr-12 sm:px-7">
					<DialogTitle>
						{saved
							? t("deployEventNamed", "Deploy {{name}}", { name: saved.name })
							: t("newEvent", "New event")}
					</DialogTitle>
					<DialogDescription>
						{saved
							? t(
									"eventSavedFinishDeployment",
									"Event saved. Finish the deployment to your devices.",
								)
							: t(
									"chooseEventStartAndDestination",
									"Choose what starts your flow and where it runs.",
								)}
					</DialogDescription>
				</DialogHeader>
				{children(shell)}
			</DialogContent>
		</Dialog>
	);
}
