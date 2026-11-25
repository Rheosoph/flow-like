"use client";

import { useTranslation } from "@flow-like/locales";
import { ChevronDownIcon, ChevronUpIcon, type LucideIcon } from "lucide-react";
import { type RefObject, useEffect, useRef, useState } from "react";
import {
	RequestMark,
	type RequestTone,
} from "../interfaces/chat-default/request-card";
import { Button } from "../ui/button";

/** Runs one focus move after the next render, when the element it needs exists. */
export function useFocusAfterRender() {
	const next = useRef<(() => void) | undefined>(undefined);
	useEffect(() => {
		const move = next.current;
		next.current = undefined;
		move?.();
	});
	return (move: () => void) => {
		next.current = move;
	};
}

export interface SettledFold {
	/** Show the one-line receipt instead of the card. */
	folded: boolean;
	/** The card is shown because its receipt was opened; it offers to fold again. */
	unfolded: boolean;
	open: () => void;
	close: () => void;
	/** After an answer from this card: focus lands on the receipt that replaces it. */
	focusReceipt: () => void;
	receiptRef: RefObject<HTMLButtonElement | null>;
	collapseRef: RefObject<HTMLButtonElement | null>;
}

/**
 * A settled approval in the transcript folds to one line; opening it shows the card again. Focus
 * follows the swap, so a keyboard never loses its place.
 */
export function useSettledFold(
	enabled: boolean,
	settled: boolean,
): SettledFold {
	const [opened, setOpened] = useState(false);
	const receiptRef = useRef<HTMLButtonElement>(null);
	const collapseRef = useRef<HTMLButtonElement>(null);
	const focusAfter = useFocusAfterRender();
	const foldable = enabled && settled;
	return {
		folded: foldable && !opened,
		unfolded: foldable && opened,
		open: () => {
			setOpened(true);
			focusAfter(() => collapseRef.current?.focus());
		},
		close: () => {
			setOpened(false);
			focusAfter(() => receiptRef.current?.focus());
		},
		focusReceipt: () => focusAfter(() => receiptRef.current?.focus()),
		receiptRef,
		collapseRef,
	};
}

/** A settled approval as one line: how it was answered and what it covered. */
export function ApprovalReceipt({
	fold,
	icon,
	tone,
	status,
	statusSlot,
	line,
	onOpen,
}: Readonly<{
	fold: SettledFold;
	icon: LucideIcon;
	tone: RequestTone;
	status: string;
	statusSlot: "approval-answer" | "approval-closure";
	line: string;
	onOpen?: () => void;
}>) {
	return (
		<button
			ref={fold.receiptRef}
			type="button"
			data-slot="approval-receipt"
			aria-expanded={false}
			title={line}
			onClick={() => {
				onOpen?.();
				fold.open();
			}}
			className="group/receipt -ml-1.5 flex w-full max-w-prose min-w-0 items-center gap-2 rounded-lg px-1.5 py-1 text-left text-xs text-muted-foreground outline-none transition-colors hover:bg-muted/60 focus-visible:ring-2 focus-visible:ring-primary/40"
		>
			<RequestMark icon={icon} tone={tone} size="xs" />
			<span
				data-slot={statusSlot}
				className="shrink-0 font-medium text-foreground/85"
			>
				{status}
			</span>
			<span aria-hidden className="shrink-0 text-muted-foreground/50">
				·
			</span>
			<span className="min-w-0 truncate">{line}</span>
			<ChevronDownIcon
				aria-hidden
				className="ml-auto size-3.5 shrink-0 text-muted-foreground/60 transition-colors group-hover/receipt:text-foreground"
			/>
		</button>
	);
}

/** The opened receipt's way back to one line, in the card header's corner. */
export function ReceiptCollapse({ fold }: Readonly<{ fold: SettledFold }>) {
	const { t } = useTranslation("chat");
	const label = t("showLess", "Show less");
	return (
		<Button
			ref={fold.collapseRef}
			type="button"
			variant="ghost"
			size="icon"
			aria-label={label}
			aria-expanded
			title={label}
			onClick={fold.close}
			className="-mt-1 -mr-1.5 size-7 rounded-md text-muted-foreground"
		>
			<ChevronUpIcon className="size-4" aria-hidden />
		</Button>
	);
}
