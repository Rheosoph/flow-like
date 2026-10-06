"use client";

import { Command, CornerDownLeft, Play } from "lucide-react";
import type { KeyboardEvent, Ref } from "react";
import { cn } from "../../../../lib/utils";
import { type DockProps, FOCUS_VALUE } from "../contracts";

/** A key held down must not click Run, Run again or Try again again and again (spec M2, key guards). */
export function guardRepeat(event: KeyboardEvent<HTMLElement>) {
	if (event.nativeEvent.isComposing || event.keyCode === 229) return;
	if (event.repeat && (event.key === "Enter" || event.key === " "))
		event.preventDefault();
}

/** The ⌘↵ (Ctrl ↵) hint inside the coral Run. */
export function KeyChip({
	mac,
	className,
}: Readonly<{ mac: boolean; className?: string }>) {
	return (
		<span
			aria-hidden="true"
			className={cn(
				"inline-flex h-5 items-center gap-0.75 rounded-md bg-scrim px-1.5 text-primary-foreground",
				className,
			)}
		>
			{mac ? (
				<Command className="size-2.75" strokeWidth={2.5} />
			) : (
				<span className="font-medium text-[11px] leading-none">Ctrl</span>
			)}
			<CornerDownLeft className="size-2.75" strokeWidth={2.5} />
		</span>
	);
}

const FOCUS_RING =
	"outline-none focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ring focus-visible:outline-solid";

const BASE = `inline-flex items-center justify-center gap-2 rounded-lg font-semibold whitespace-nowrap ${FOCUS_RING}`;
const CORAL = "border-0 bg-primary text-primary-foreground hover:brightness-95";
const NEUTRAL =
	"border border-border bg-card text-muted-foreground aria-disabled:cursor-not-allowed hover:bg-card";

const SIZE: Readonly<
	Record<DockProps["variant"], { readonly box: string; readonly icon: string }>
> = {
	rail: {
		box: "relative h-9 min-w-0 flex-1 px-3.5 text-[14px]",
		icon: "size-3.5",
	},
	hero: { box: "relative h-9 w-full px-3.5 text-[14px]", icon: "size-3.5" },
	strip: { box: "h-8 pr-2 pl-3.5 text-[14px]", icon: "size-3.25" },
	phone: {
		box: "h-11.5 min-w-0 flex-1 px-3.5 text-[15px]",
		icon: "size-3.75",
	},
};

export interface RunButtonProps {
	readonly variant: DockProps["variant"];
	readonly label: string;
	readonly title: string;
	/** A form without fields at its cap: neutral and aria-disabled, it keeps focus and still reports the press. */
	readonly capped: boolean;
	/** Touch metrics: 44 px (the phone bar is 46 px by itself). */
	readonly large: boolean;
	readonly mac: boolean;
	readonly onRun: () => void;
	readonly ref?: Ref<HTMLButtonElement>;
}

function RunFace({
	variant,
	label,
	icon,
	mac,
	chip,
}: Readonly<{
	variant: DockProps["variant"];
	label: string;
	icon: string;
	mac: boolean;
	chip: boolean;
}>) {
	const chipClass = variant === "strip" ? "ml-1.5" : "absolute top-2 right-2";
	return (
		<>
			<Play aria-hidden className={cn(icon, "shrink-0")} strokeWidth={2.25} />
			<span>{label}</span>
			{chip ? <KeyChip mac={mac} className={chipClass} /> : null}
		</>
	);
}

/** The Run control of every variant: the rail and phone dock, the hero card and the strip's left end. */
export function RunButton({
	variant,
	label,
	title,
	capped,
	large,
	mac,
	onRun,
	ref,
}: Readonly<RunButtonProps>) {
	const size = SIZE[variant];
	const touchTall = large && variant !== "phone";
	return (
		<button
			ref={ref}
			type="button"
			title={title}
			aria-disabled={capped || undefined}
			aria-keyshortcuts={mac ? "Meta+Enter" : "Control+Enter"}
			data-fw-focus={
				variant === "strip" ? FOCUS_VALUE.runAgain : FOCUS_VALUE.run
			}
			onClick={onRun}
			onKeyDown={guardRepeat}
			className={cn(
				BASE,
				size.box,
				touchTall && "h-11",
				capped ? `${NEUTRAL} px-3.5` : CORAL,
			)}
		>
			{capped ? (
				<span>{label}</span>
			) : (
				<RunFace
					variant={variant}
					label={label}
					icon={size.icon}
					mac={mac}
					chip={variant !== "phone" && !large}
				/>
			)}
		</button>
	);
}

function StopGlyph() {
	return (
		<svg
			viewBox="0 0 24 24"
			aria-hidden="true"
			className="size-3.5 shrink-0 fill-current stroke-current"
			strokeWidth={2}
			strokeLinejoin="round"
		>
			<rect width="12" height="12" x="6" y="6" rx="1.5" />
		</svg>
	);
}

export interface StopButtonProps {
	readonly label: string;
	readonly ariaLabel: string;
	readonly title: string;
	readonly large: boolean;
	/** The phone bar's 46 px square. */
	readonly iconOnly: boolean;
	readonly mac: boolean;
	readonly onStop: () => void;
}

/** Stop: secondary, next to Run, for the run on the stage while it can still be stopped or taken out of the queue. */
export function StopButton({
	label,
	ariaLabel,
	title,
	large,
	iconOnly,
	mac,
	onStop,
}: Readonly<StopButtonProps>) {
	return (
		<button
			type="button"
			aria-label={ariaLabel}
			title={title}
			aria-keyshortcuts={mac ? "Meta+." : "Control+."}
			data-fw-focus={FOCUS_VALUE.stop}
			onClick={onStop}
			className={cn(
				"inline-flex shrink-0 items-center justify-center gap-1.75 rounded-lg border border-border bg-card font-medium text-foreground hover:bg-row-hover",
				FOCUS_RING,
				iconOnly ? "size-11.5" : "h-9 px-3.5 text-[13.5px]",
				large && !iconOnly && "h-11",
			)}
		>
			<StopGlyph />
			{iconOnly ? null : <span>{label}</span>}
		</button>
	);
}
