"use client";

import { useTranslation } from "@flow-like/locales";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import * as PopoverPrimitive from "@radix-ui/react-popover";
import { useRef } from "react";
import { usePortalContainer } from "../../../ui/portal-container";
import {
	PerRunPanel,
	type PerRunPanelProps,
	type TextSlot,
	rowInput,
} from "./per-run-panel";

type PanelData = Omit<PerRunPanelProps, "Title" | "Lead">;

const PlainText: TextSlot = ({ className, children }) => (
	<div className={className}>{children}</div>
);

export interface PerRunPopoverContentProps {
	readonly panel: PanelData;
	/** The row that takes the cursor on open (the field whose marker opened the panel), else the first. */
	readonly focusName: string | null;
	readonly maxHeight: number;
	/** Gives the cursor back to the field whose marker opened the panel; false leaves it to "Change". */
	readonly returnFocus: (name: string) => boolean;
}

/**
 * The "Per run" popover (spec M1 c): above the after-run line, its right edge on the dock's inner right edge,
 * 320 px wide. Esc and "Done" close it and the cursor goes back to "Change", or to the field whose marker opened it.
 */
export function PerRunPopoverContent({
	panel,
	focusName,
	maxHeight,
	returnFocus,
}: Readonly<PerRunPopoverContentProps>) {
	const { t } = useTranslation("interfaces");
	const container = usePortalContainer();
	const contentRef = useRef<HTMLDivElement>(null);
	const openedFor = useRef(focusName);
	return (
		<PopoverPrimitive.Portal container={container}>
			<PopoverPrimitive.Content
				ref={contentRef}
				side="top"
				align="end"
				sideOffset={8}
				collisionPadding={8}
				aria-label={t("interfaces:workbench.dock.panelTitle", "Per run")}
				onOpenAutoFocus={(event) => {
					event.preventDefault();
					rowInput(contentRef.current, focusName)?.focus();
				}}
				onCloseAutoFocus={(event) => {
					const name = openedFor.current;
					openedFor.current = null;
					if (name !== null && returnFocus(name)) event.preventDefault();
				}}
				style={{ maxHeight }}
				className="z-50 flex w-80 flex-col rounded-xl border border-border-strong bg-popover p-3 text-foreground shadow-floating outline-none"
			>
				<PerRunPanel {...panel} Title={PlainText} Lead={PlainText} />
			</PopoverPrimitive.Content>
		</PopoverPrimitive.Portal>
	);
}

const SheetTitle: TextSlot = ({ className, children }) => (
	<DialogPrimitive.Title asChild>
		<div className={className}>{children}</div>
	</DialogPrimitive.Title>
);

const SheetLead: TextSlot = ({ className, children }) => (
	<DialogPrimitive.Description asChild>
		<div className={className}>{children}</div>
	</DialogPrimitive.Description>
);

export interface PerRunSheetProps {
	readonly open: boolean;
	readonly panel: PanelData;
	readonly focusName: string | null;
	/** The interface root, so the sheet and its scrim cover the interface only. */
	readonly container: HTMLElement | null;
	readonly onOpenChange: (open: boolean) => void;
}

/**
 * The narrow layout's "Per run" bottom sheet (spec M1 phone): inside the interface, 10 px top radius, at most 80 % of
 * the box, 44 px rows and a 44 px "Done".
 */
export function PerRunSheet({
	open,
	panel,
	focusName,
	container,
	onOpenChange,
}: Readonly<PerRunSheetProps>) {
	const contentRef = useRef<HTMLDivElement>(null);
	if (!container) return null;
	return (
		<DialogPrimitive.Root open={open} onOpenChange={onOpenChange}>
			<DialogPrimitive.Portal container={container}>
				<DialogPrimitive.Overlay className="absolute inset-0 z-40 bg-scrim" />
				<DialogPrimitive.Content
					ref={contentRef}
					onOpenAutoFocus={(event) => {
						event.preventDefault();
						rowInput(contentRef.current, focusName)?.focus();
					}}
					className="absolute inset-x-0 bottom-0 z-40 flex max-h-[80%] flex-col rounded-t-xl border-border-strong border-t bg-popover p-4 text-foreground shadow-floating outline-none"
				>
					<PerRunPanel {...panel} Title={SheetTitle} Lead={SheetLead} />
				</DialogPrimitive.Content>
			</DialogPrimitive.Portal>
		</DialogPrimitive.Root>
	);
}
