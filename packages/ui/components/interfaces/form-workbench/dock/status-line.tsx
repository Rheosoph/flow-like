"use client";

import { ChevronDown, ChevronUp, LoaderCircle, OctagonX } from "lucide-react";
import { type MouseEvent, useId } from "react";
import { cn } from "../../../../lib/utils";
import type { LineAction, LineIcon, LineTone, LineView } from "./line-view";

const TONE_CLASS: Readonly<Record<LineTone, string>> = {
	info: "text-info",
	critical: "text-critical",
	ink: "text-ink-2",
	muted: "text-muted-foreground",
};

/** Buttons in the line leave the cursor where it is: a mouse press must not move focus out of the field being typed in. */
const keepFocus = (event: MouseEvent) => event.preventDefault();

const FOCUS_RING =
	"outline-none focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ring focus-visible:outline-solid";

interface Metrics {
	readonly line: string;
	readonly icon: string;
	readonly arrow: string;
	readonly arrowIcon: string;
	readonly action: string;
}

const RAIL: Metrics = {
	line: "min-h-5.5 gap-1.5 text-xs leading-4",
	icon: "size-3.25",
	arrow: "size-5.5",
	arrowIcon: "size-3.5",
	action: "-my-px h-6 px-1 text-xs leading-4",
};

const BIG: Metrics = {
	line: "min-h-11 gap-2 text-[13px] leading-4.5",
	icon: "size-3.5",
	arrow: "size-11",
	arrowIcon: "size-4",
	action: "h-11 px-3 text-[13px] leading-4.5",
};

function LineGlyph({
	icon,
	className,
}: Readonly<{ icon: LineIcon; className: string }>) {
	if (icon === "spinner")
		return (
			<LoaderCircle
				aria-hidden
				className={cn(
					className,
					"shrink-0 animate-spin motion-reduce:animate-none",
				)}
				strokeWidth={2.25}
			/>
		);
	if (icon === "alert")
		return <OctagonX aria-hidden className={cn(className, "shrink-0")} />;
	if (icon === "bar")
		return (
			<span
				aria-hidden="true"
				className="h-3 w-0.5 shrink-0 rounded-xs bg-foreground"
			/>
		);
	return null;
}

function StepButton({
	label,
	direction,
	metrics,
	onStep,
}: Readonly<{
	label: string;
	direction: 1 | -1;
	metrics: Metrics;
	onStep: (direction: 1 | -1) => void;
}>) {
	const Chevron = direction > 0 ? ChevronDown : ChevronUp;
	return (
		<button
			type="button"
			aria-label={label}
			onMouseDown={keepFocus}
			onClick={() => onStep(direction)}
			className={cn(
				"inline-flex shrink-0 items-center justify-center rounded-md text-ink-2 hover:bg-row-hover",
				FOCUS_RING,
				metrics.arrow,
			)}
		>
			<Chevron aria-hidden className={metrics.arrowIcon} />
		</button>
	);
}

function ActionButton({
	action,
	metrics,
	describedBy,
	onActed,
}: Readonly<{
	action: LineAction;
	metrics: Metrics;
	describedBy: string;
	onActed: (viaKeyboard: boolean) => void;
}>) {
	return (
		<button
			type="button"
			aria-label={action.ariaLabel}
			aria-describedby={describedBy}
			onMouseDown={keepFocus}
			onClick={(event) => {
				action.run();
				onActed(event.detail === 0);
			}}
			className={cn(
				"shrink-0 rounded-md font-medium text-ink-2 underline-offset-2 hover:underline",
				FOCUS_RING,
				metrics.action,
			)}
		>
			{action.label}
		</button>
	);
}

export interface StatusLineProps {
	readonly view: LineView;
	/** Phone and touch metrics: 44 px buttons, 13 px text. */
	readonly big: boolean;
	/** The phone's own bar above the dock: top border, card fill, room for the arrows' edge. */
	readonly bar: boolean;
	readonly onStep: (direction: 1 | -1) => void;
	/** After a button of the line ran; true when it was pressed with the keyboard (focus must not fall to the page). */
	readonly onActed: (viaKeyboard: boolean) => void;
}

/** The dock's single status line: one thing at a time, its words, tone, icon and the buttons at its right end. */
export function StatusLine({
	view,
	big,
	bar,
	onStep,
	onActed,
}: Readonly<StatusLineProps>) {
	const metrics = big ? BIG : RAIL;
	const textId = useId();
	return (
		<div
			data-fw-dock-line={view.kind}
			className={cn(
				"order-first flex items-center",
				metrics.line,
				TONE_CLASS[view.tone],
				bar && "border-t border-border bg-card pr-2 pl-4",
			)}
		>
			<LineGlyph icon={view.icon} className={metrics.icon} />
			<span
				id={textId}
				className={cn(
					"min-w-0 flex-1",
					view.strong ? "font-medium" : "font-normal",
				)}
			>
				{view.text}
			</span>
			{view.actions.map((action) => (
				<ActionButton
					key={action.id}
					action={action}
					metrics={metrics}
					describedBy={textId}
					onActed={onActed}
				/>
			))}
			{view.steps ? (
				<>
					<StepButton
						label={view.steps.prev}
						direction={-1}
						metrics={metrics}
						onStep={onStep}
					/>
					<StepButton
						label={view.steps.next}
						direction={1}
						metrics={metrics}
						onStep={onStep}
					/>
				</>
			) : null}
		</div>
	);
}

/**
 * A hidden mirror of the line while it is a message, so a key press gets its answer read out (the line itself is
 * silent). Zero height at the dock's own width: the text wraps inside it and never widens the page.
 */
export function MessageLiveRegion({ text }: Readonly<{ text: string }>) {
	return (
		<output
			aria-live="polite"
			aria-atomic="true"
			className="pointer-events-none absolute inset-x-0 top-0 h-0 overflow-hidden"
		>
			{text}
		</output>
	);
}
