"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CircleCheck,
	CircleDashed,
	type LucideIcon,
	OctagonX,
	SquareX,
	TriangleAlert,
	WifiOff,
} from "lucide-react";
import type { ReactNode } from "react";
import { enumLabel } from "../copy/enum-labels";
import type { DevicesT } from "./area-context";
import { healthLabel } from "./status-chip";
import { TONE_TEXT, type Tone, cx } from "./tone";

export const ANNUNCIATOR_WINDOWS = [
	"critical",
	"attention",
	"unknown",
	"healthy",
	"offline",
	"revoked",
] as const;
export type AnnunciatorWindow = (typeof ANNUNCIATOR_WINDOWS)[number];

export interface AnnunciatorCell {
	count: number;
	/** Device names in this window (the hover lists all of them). */
	names?: readonly string[];
	/** Appended after the names ("locked", "also critical"). */
	note?: string;
	/** Replaces the names line ("1 still billed to you"). */
	sub?: string;
}

/** `neutral`: a lit window takes the foreground ink instead of a status colour. */
export type AnnunciatorTone = Tone | "neutral";

/** One window of a summary that isn't the fleet's (keys, certificates). */
export interface AnnunciatorWindowSpec<T extends string = string>
	extends AnnunciatorCell {
	id: T;
	tone: AnnunciatorTone;
	icon: LucideIcon;
	label: string;
	/** Hover text; the names, or the label, otherwise. */
	title?: string;
}

const LOOK: Record<
	AnnunciatorWindow,
	{ tone: AnnunciatorTone; icon: LucideIcon }
> = {
	critical: { tone: "critical", icon: OctagonX },
	attention: { tone: "warning", icon: TriangleAlert },
	unknown: { tone: "unknown", icon: CircleDashed },
	healthy: { tone: "good", icon: CircleCheck },
	offline: { tone: "critical", icon: WifiOff },
	revoked: { tone: "unknown", icon: SquareX },
};

function windowLabel(t: DevicesT, window: AnnunciatorWindow): string {
	return window === "offline"
		? enumLabel(t, "presence", "offline")
		: healthLabel(t, window);
}

const NAMES_SHOWN = 3;

function namesList(t: DevicesT, names: readonly string[]): string {
	const shown = names.slice(0, NAMES_SHOWN).join(", ");
	return names.length > NAMES_SHOWN
		? t("devices:view.annunciator.more", "{{names}} +{{count, number}} more", {
				names: shown,
				count: names.length - NAMES_SHOWN,
			})
		: shown;
}

function withNote(t: DevicesT, list: string, note?: string): string {
	if (!note) return list;
	return list
		? t("devices:view.annunciator.withNote", "{{list}} · {{note}}", {
				list,
				note,
			})
		: note;
}

/** "a, b, c +2 more · note"; "none right now" only for a zero count; the note alone when no names are given. */
export function annunciatorSub(t: DevicesT, cell: AnnunciatorCell): string {
	if (cell.sub) return cell.sub;
	if (cell.count === 0)
		return t("devices:view.annunciator.none", "none right now");
	return withNote(t, namesList(t, cell.names ?? []), cell.note);
}

/**
 * SPEC §4.12: six joined windows (6 → 3 → 2 by container width). A lit window
 * colours its number and label; a lit critical one also fills. Pressing filters.
 */
export function AnnunciatorWindows<T extends string>({
	label,
	windows,
	pressed = null,
	onSelect,
	caption,
	className,
}: Readonly<{
	/** Accessible name of the group ("Certificates by state. Select a window to filter the list."). */
	label: string;
	windows: readonly AnnunciatorWindowSpec<T>[];
	pressed?: T | null;
	/** Called with the window, or null when the pressed one is pressed again. */
	onSelect?: (window: T | null) => void;
	caption?: ReactNode;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	return (
		<div
			className={cx("@container/annun flex min-w-0 flex-col gap-2", className)}
		>
			<fieldset
				aria-label={label}
				className="m-0 grid min-w-0 grid-cols-6 gap-px overflow-hidden rounded-lg border border-border bg-hairline p-0 @max-[900px]/annun:grid-cols-3 @max-[480px]/annun:grid-cols-2"
			>
				{windows.map((window) => {
					const { id, tone, icon: Icon } = window;
					const lit = window.count > 0;
					const sub = annunciatorSub(t, window);
					const isPressed = pressed === id;
					const ink = !lit
						? "text-muted-foreground"
						: tone === "neutral"
							? "text-foreground"
							: TONE_TEXT[tone];
					return (
						<button
							key={id}
							type="button"
							data-window={id}
							data-tone={tone}
							data-lit={lit}
							aria-pressed={isPressed}
							title={
								window.title ??
								(window.names?.length ? window.names.join(", ") : window.label)
							}
							onClick={() => onSelect?.(isPressed ? null : id)}
							className={cx(
								"grid min-w-0 cursor-pointer grid-cols-[auto_minmax(0,1fr)] grid-rows-[auto_auto] items-center gap-x-2.5 border-0 px-3.5 py-2.5 text-left text-muted-foreground focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring",
								lit && tone === "critical"
									? "bg-critical-bg"
									: "bg-card hover:bg-row-hover",
								isPressed && "outline-2 -outline-offset-2 outline-foreground",
							)}
						>
							<span
								className={cx(
									"row-span-2 font-mono text-[22px] leading-6.5 font-medium tabular-nums",
									ink,
								)}
							>
								{window.count}
							</span>
							<span
								className={cx(
									"inline-flex min-w-0 items-center gap-1.25 truncate text-ui",
									lit ? cx("font-medium", ink) : "font-medium",
								)}
							>
								<Icon aria-hidden className="size-3.5 shrink-0" />
								<span className="truncate">{window.label}</span>
							</span>
							<span className="truncate text-xs text-muted-foreground">
								{sub}
							</span>
						</button>
					);
				})}
			</fieldset>
			{caption ? (
				<p className="max-w-[110ch] text-xs text-muted-foreground">{caption}</p>
			) : null}
		</div>
	);
}

/** The fleet's six health windows (N1). */
export function Annunciator({
	cells,
	pressed = null,
	onSelect,
	caption,
	className,
}: Readonly<{
	cells: Record<AnnunciatorWindow, AnnunciatorCell>;
	pressed?: AnnunciatorWindow | null;
	/** Called with the window, or null when the pressed one is pressed again. */
	onSelect?: (window: AnnunciatorWindow | null) => void;
	caption?: ReactNode;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	return (
		<AnnunciatorWindows
			label={t(
				"view.annunciator.label",
				"Fleet health. Select a window to filter devices.",
			)}
			windows={ANNUNCIATOR_WINDOWS.map((id) => ({
				id,
				...LOOK[id],
				label: windowLabel(t, id),
				...cells[id],
			}))}
			pressed={pressed}
			onSelect={onSelect}
			caption={caption}
			className={className}
		/>
	);
}
