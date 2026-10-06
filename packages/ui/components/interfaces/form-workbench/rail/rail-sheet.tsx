"use client";

import { useTranslation } from "@flow-like/locales";
import * as Dialog from "@radix-ui/react-dialog";
import { X } from "lucide-react";
import { type ReactNode, useCallback, useRef, useState } from "react";
import { cx } from "../../../settings/devices/primitives/tone";
import {
	type FocusSpot,
	focusOnPage,
	focusSpotOf,
	returnFocusTo,
	useIsoLayoutEffect,
} from "../shell/focus";

export interface RailSheetProps {
	readonly open: boolean;
	readonly onClose: () => void;
	readonly title: string;
	/** `bottom`: at most 80 % of the box, 10 px top corners. `full`: the whole box. */
	readonly variant: "bottom" | "full";
	readonly children: ReactNode;
}

const INTERFACE_ROOT = "[data-fw-root]";

/** The interface root a sheet covers; the document body when the rail is shown without one. */
function useInterfaceRoot() {
	const [root, setRoot] = useState<HTMLElement | null>(null);
	const probe = useCallback((node: HTMLElement | null) => {
		const found = node?.closest<HTMLElement>(INTERFACE_ROOT);
		setRoot(found ?? node?.ownerDocument.body ?? null);
	}, []);
	return { root, probe };
}

/**
 * The sheet has no trigger, so Radix gives focus back to nothing: the element that had focus when it
 * opened gets it back, caret kept, when closing left focus on the page. Read in a layout effect, before
 * a control in the sheet takes focus from its own effect.
 */
function useOpenerReturn(open: boolean) {
	const opener = useRef<FocusSpot | null>(null);
	useIsoLayoutEffect(() => {
		if (open) opener.current = focusSpotOf(document);
	}, [open]);
	return useCallback((event: Event) => {
		event.preventDefault();
		const from = opener.current;
		opener.current = null;
		if (focusOnPage(document)) returnFocusTo(from);
	}, []);
}

const SHEET: Readonly<Record<RailSheetProps["variant"], string>> = {
	bottom:
		"inset-x-0 bottom-0 max-h-[80%] rounded-t-[10px] border border-b-0 border-border-strong",
	full: "inset-0",
};

/**
 * A sheet inside the interface box (phone): the scrim covers the interface only, focus stays in the
 * sheet, Esc and the scrim close it. The root is `relative`, so the sheet is positioned against it.
 */
export function RailSheet({
	open,
	onClose,
	title,
	variant,
	children,
}: Readonly<RailSheetProps>) {
	const { t } = useTranslation("interfaces");
	const { root, probe } = useInterfaceRoot();
	const content = useRef<HTMLDivElement | null>(null);
	const giveFocusBack = useOpenerReturn(open);
	/** The sheet itself takes focus unless a field in it already has it (no ring on a close button). */
	const takeFocus = (event: Event) => {
		event.preventDefault();
		const node = content.current;
		if (node && !node.contains(node.ownerDocument.activeElement)) node.focus();
	};
	return (
		<>
			<span ref={probe} hidden />
			{open && root ? (
				<Dialog.Root
					open
					onOpenChange={(next) => {
						if (!next) onClose();
					}}
				>
					<Dialog.Portal container={root}>
						<Dialog.Overlay className="absolute inset-0 z-40 bg-scrim" />
						<Dialog.Content
							ref={content}
							tabIndex={-1}
							data-fw-modal=""
							aria-describedby={undefined}
							onOpenAutoFocus={takeFocus}
							onCloseAutoFocus={giveFocusBack}
							className={cx(
								"absolute z-50 flex min-h-0 flex-col bg-popover text-popover-foreground shadow-floating outline-none",
								SHEET[variant],
							)}
						>
							<header className="flex h-12 flex-none items-center gap-2 border-b border-hairline pr-1 pl-4">
								<Dialog.Title className="min-w-0 flex-1 truncate text-[15px]/5 font-semibold">
									{title}
								</Dialog.Title>
								<Dialog.Close
									aria-label={t("workbench.preset.sheet.close", "Close")}
									className="inline-flex size-11 flex-none items-center justify-center rounded-md text-ink-2 hover:bg-row-hover focus-visible:outline-2 focus-visible:outline-ring"
								>
									<X aria-hidden className="size-4" />
								</Dialog.Close>
							</header>
							{children}
						</Dialog.Content>
					</Dialog.Portal>
				</Dialog.Root>
			) : null}
		</>
	);
}
