"use client";

import { useTranslation } from "@flow-like/locales";
import type { ReactNode } from "react";
import {
	type DesiredRun,
	type ObservedRun,
	requestedActualLabel,
} from "./requested-actual";
import type { ConvergenceChipKind } from "./status-chip";
import { cx } from "./tone";

function RequestedPin({ desired }: Readonly<{ desired: DesiredRun }>) {
	const cls = "fill-none stroke-muted-foreground";
	return desired === "stopped" ? (
		<rect
			className={cls}
			x="1.5"
			y="2.5"
			width="7"
			height="7"
			rx="1"
			strokeWidth="1.5"
		/>
	) : (
		<circle className={cls} cx="5" cy="6" r="3.5" strokeWidth="1.5" />
	);
}

function Wire({ conv }: Readonly<{ conv: ConvergenceChipKind }>) {
	if (conv === "crash_looping" || conv === "failed_stopped") {
		return (
			<g className="stroke-critical" strokeWidth="1.5" data-wire="crash">
				<line x1="9" y1="6" x2="13.2" y2="6" />
				<line x1="16.8" y1="6" x2="21" y2="6" />
				<line x1="13.5" y1="9.5" x2="16.5" y2="2.5" />
			</g>
		);
	}
	if (conv === "converged" || conv === "stopped_by_user") {
		return (
			<line
				data-wire="ok"
				className="stroke-border-strong"
				x1="9"
				y1="6"
				x2="21"
				y2="6"
				strokeWidth="1.5"
			/>
		);
	}
	return (
		<line
			data-wire="moving"
			className="stroke-info"
			x1="9"
			y1="6"
			x2="21"
			y2="6"
			strokeWidth="1.5"
			strokeDasharray="2 2"
		/>
	);
}

const HALF_PIN = (d: string) => (
	<>
		<circle
			className="fill-none stroke-info-solid"
			cx="25"
			cy="6"
			r="3.5"
			strokeWidth="1.5"
		/>
		<path className="fill-info-solid" d={d} />
	</>
);

const UNKNOWN_PIN = (
	<circle
		className="fill-none stroke-unknown"
		cx="25"
		cy="6"
		r="3.8"
		strokeWidth="1.5"
		strokeDasharray="2 1.6"
	/>
);

const ACTUAL_PIN: Record<ObservedRun, ReactNode> = {
	running: <circle className="fill-good-solid" cx="25" cy="6" r="4" />,
	starting: HALF_PIN("M25 2.5a3.5 3.5 0 0 1 0 7z"),
	stopping: HALF_PIN("M25 2.5a3.5 3.5 0 0 0 0 7z"),
	backoff: <path className="fill-critical-solid" d="M25 1.6 29.4 10H20.6z" />,
	failed: (
		<>
			<circle className="fill-critical-solid" cx="25" cy="6" r="4.2" />
			<path
				className="stroke-card"
				strokeWidth="1.4"
				d="M23.2 4.2l3.6 3.6M26.8 4.2l-3.6 3.6"
			/>
		</>
	),
	stopped: (
		<rect
			className="fill-paused-solid"
			x="21"
			y="2"
			width="8"
			height="8"
			rx="1"
		/>
	),
	unknown: UNKNOWN_PIN,
	removed: UNKNOWN_PIN,
};

/**
 * SPEC §4.8: hollow requested pin, a wire for convergence, a filled actual pin.
 * Always followed by text in the same row; `title` once per page carries the legend.
 */
export function PairedPins({
	desired,
	observed,
	conv,
	title = false,
	className,
}: Readonly<{
	desired: DesiredRun;
	observed: ObservedRun;
	conv: ConvergenceChipKind;
	/** The first use on a page explains the glyph in its hover. */
	title?: boolean;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	const label = requestedActualLabel(t, desired, observed);
	return (
		<span
			role="img"
			aria-label={label}
			title={
				title
					? t(
							"view.pins.title",
							"{{label}}. Hollow pin: what you asked for. Filled pin: what the device reports.",
							{ label },
						)
					: undefined
			}
			data-req={desired}
			data-act={observed}
			data-conv={conv}
			className={cx("inline-flex shrink-0 align-middle", className)}
		>
			<svg
				viewBox="0 0 30 12"
				width="30"
				height="12"
				aria-hidden="true"
				className="block overflow-visible"
			>
				<RequestedPin desired={desired} />
				<Wire conv={conv} />
				{ACTUAL_PIN[observed]}
			</svg>
		</span>
	);
}
