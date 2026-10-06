"use client";

import { useTranslation } from "@flow-like/locales";
import * as PopoverPrimitive from "@radix-ui/react-popover";
import { Keyboard } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "../../../../lib/utils";
import { FOCUS_VALUE } from "../contracts";
import {
	afterRunChange,
	afterRunChangeLabel,
	afterRunSummary,
	shortcutsLabel,
} from "./copy";

const FOCUS_RING =
	"outline-none focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ring focus-visible:outline-solid";

function ShortcutsButton({
	mac,
	onClick,
}: Readonly<{ mac: boolean; onClick: () => void }>) {
	const { t } = useTranslation("interfaces");
	const label = shortcutsLabel(t, mac);
	return (
		<button
			type="button"
			aria-label={label}
			title={label}
			aria-keyshortcuts={mac ? "Meta+/" : "Control+/"}
			onClick={onClick}
			className={cn(
				"-my-1 inline-flex size-6 items-center justify-center rounded-md text-muted-foreground hover:bg-row-hover hover:text-foreground",
				FOCUS_RING,
			)}
		>
			<Keyboard aria-hidden className="size-3.5" />
		</button>
	);
}

export interface AfterRunLineProps {
	/** How many inputs are per run. */
	readonly count: number;
	/** Forms with 4 or more fields and a fine pointer: the keyboard icon at the right end. */
	readonly keyboardIcon: boolean;
	readonly mac: boolean;
	readonly open: boolean;
	readonly onOpenChange: (open: boolean) => void;
	readonly onShortcuts: () => void;
	/** The popover's content; it anchors to this line, so its right edge sits on the dock's inner right edge. */
	readonly children: ReactNode;
}

/**
 * The line under the Run row (spec M1 b): "3 inputs are per run · Change", the keyboard icon at the right end and the
 * "Per run" popover. "Change" is the popover's trigger and a Tab stop after Run and Stop.
 */
export function AfterRunLine({
	count,
	keyboardIcon,
	mac,
	open,
	onOpenChange,
	onShortcuts,
	children,
}: Readonly<AfterRunLineProps>) {
	const { t } = useTranslation("interfaces");
	return (
		<PopoverPrimitive.Root open={open} onOpenChange={onOpenChange}>
			<PopoverPrimitive.Anchor asChild>
				<div
					data-fw-after-run=""
					className="flex h-4 items-center gap-1 text-muted-foreground text-xs leading-4"
				>
					<span>{afterRunSummary(t, count)}</span>
					<span aria-hidden="true">·</span>
					<PopoverPrimitive.Trigger asChild>
						<button
							type="button"
							aria-label={afterRunChangeLabel(t)}
							data-fw-focus={FOCUS_VALUE.change}
							className={cn(
								"relative rounded-md font-medium text-ink-2 underline-offset-2 hover:underline after:absolute after:-inset-x-1 after:top-1/2 after:h-6 after:-translate-y-1/2 after:content-['']",
								FOCUS_RING,
							)}
						>
							{afterRunChange(t)}
						</button>
					</PopoverPrimitive.Trigger>
					<span className="flex-1" />
					{keyboardIcon ? (
						<ShortcutsButton mac={mac} onClick={onShortcuts} />
					) : null}
				</div>
			</PopoverPrimitive.Anchor>
			{children}
		</PopoverPrimitive.Root>
	);
}
