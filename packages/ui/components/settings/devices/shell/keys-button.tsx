"use client";

import { useTranslation } from "@flow-like/locales";
import { Lock, LockOpen } from "lucide-react";
import { type ReactNode, useState } from "react";
import type { DevicesScope } from "../../../../lib/device-management/model/types";
import { Popover, PopoverContent, PopoverTrigger } from "../../../ui/popover";
import { DvButton } from "../primitives/dv-button";
import { cx } from "../primitives/tone";
import { useKeyChip } from "../workspace";
import {
	CHROME_BUTTON,
	CHROME_LABEL,
	CHROME_POPOVER,
} from "./attention-button";
import { KeysPopover } from "./keys-popover";
import type { ChromeNavigate } from "./rail-row";

export interface KeysButtonProps {
	scope: DevicesScope;
	onNavigate: ChromeNavigate;
	className?: string;
}

export interface KeysButtonViewProps {
	/** Devices whose keys are open on this computer. */
	unlocked: number;
	open?: boolean;
	onOpenChange?: (open: boolean) => void;
	/** The popover's content. */
	children?: ReactNode;
	className?: string;
}

/** The Keys button over a plain count. */
export function KeysButtonView({
	unlocked,
	open,
	onOpenChange,
	children,
	className,
}: Readonly<KeysButtonViewProps>) {
	const { t } = useTranslation("devices");
	const label =
		unlocked > 0
			? t("chrome.keys.unlocked", "{{count, number}} unlocked", {
					count: unlocked,
				})
			: t("chrome.keys.locked", "Locked");
	return (
		<Popover open={open} onOpenChange={onOpenChange}>
			<PopoverTrigger asChild>
				<DvButton
					variant="ghost"
					icon={unlocked > 0 ? LockOpen : Lock}
					aria-haspopup="dialog"
					aria-label={
						unlocked > 0
							? t(
									"chrome.keys.nameUnlocked",
									"Key sessions: {{count, number}} unlocked",
									{ count: unlocked },
								)
							: t("chrome.keys.nameLocked", "Key sessions: all locked")
					}
					data-chrome="keys"
					className={cx(CHROME_BUTTON, className)}
				>
					<span className={CHROME_LABEL}>{label}</span>
					<span
						aria-hidden
						className="hidden font-mono text-xs font-medium tabular-nums @max-[720px]/devices:inline"
					>
						{unlocked > 0 ? unlocked : null}
					</span>
				</DvButton>
			</PopoverTrigger>
			<PopoverContent
				align="end"
				aria-label={t("chrome.keys.popover", "Key sessions")}
				className={cx(CHROME_POPOVER, "w-[min(420px,calc(100vw-16px))]")}
			>
				{children}
			</PopoverContent>
		</Popover>
	);
}

/** SPEC §3.2 item 7: "N unlocked" / "Locked"; opens the key sessions popover. */
export function KeysButton({
	scope,
	onNavigate,
	className,
}: Readonly<KeysButtonProps>) {
	const [open, setOpen] = useState(false);
	const { unlockedCount } = useKeyChip();
	return (
		<KeysButtonView
			unlocked={unlockedCount}
			open={open}
			onOpenChange={setOpen}
			className={className}
		>
			<KeysPopover
				scope={scope}
				onNavigate={onNavigate}
				onClose={() => setOpen(false)}
			/>
		</KeysButtonView>
	);
}
