"use client";

import { useTranslation } from "@flow-like/locales";
import { Lock, type LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { Skeleton } from "../../../ui/skeleton";
import { useAreaTime } from "./area-context";
import { Banner } from "./banner";
import { GATE_ICON, type GateKind, STATE_ICON, type StateKind } from "./icons";
import { cx } from "./tone";

const FRAME: Record<Exclude<StateKind, "loading">, string> = {
	empty: "border-solid border-border bg-card",
	notloaded: "border-dashed border-unknown-line bg-surface-sunken",
	never: "border-dashed border-unknown-line bg-surface-sunken",
	locked: "border-dashed border-locked-line bg-surface-sunken",
	noaccess: "border-dashed border-unknown-line bg-surface-sunken",
	unsupported: "border-dashed border-unknown-line bg-surface-sunken",
	error: "border-solid border-critical-line bg-card",
	gate: "border-dashed border-unknown-line bg-card px-6 py-7",
};

const ICON_TONE: Partial<Record<StateKind, string>> = {
	locked: "text-locked",
	error: "text-critical",
};

const SKELETON_WIDTHS = ["w-4/5", "w-3/5", "w-2/5"];

function skeletonRows(count: number) {
	return Array.from({ length: Math.max(1, count) }, (_, index) => ({
		id: `skeleton-${index}`,
		width: SKELETON_WIDTHS[index % SKELETON_WIDTHS.length],
	}));
}

export interface StateViewProps {
	kind: StateKind;
	title?: ReactNode;
	text?: ReactNode;
	actions?: ReactNode;
	/** Icon for `kind="gate"` (the area gate's reason). */
	gate?: GateKind;
	icon?: LucideIcon;
	/** Skeleton rows for `loading`. */
	rows?: number;
	className?: string;
}

/** SPEC §4.31: never "empty" for not loaded, locked, no access, not supported or error (R6). */
export function StateView({
	kind,
	title,
	text,
	actions,
	gate,
	icon,
	rows = 3,
	className,
}: Readonly<StateViewProps>) {
	const { t } = useTranslation("devices");

	if (kind === "loading") {
		return (
			// biome-ignore lint/a11y/useSemanticElements: <output> allows phrasing content only; the skeleton bars are blocks
			<div
				role="status"
				aria-busy="true"
				data-kind="loading"
				className={cx("flex flex-col gap-2 py-3", className)}
			>
				<span className="sr-only">
					{title ?? t("common.state.loading", "Loading…")}
				</span>
				{skeletonRows(rows).map((row) => (
					<Skeleton
						key={row.id}
						aria-hidden
						className={cx("h-3 rounded-[3px] bg-muted", row.width)}
					/>
				))}
			</div>
		);
	}

	const titles: Record<Exclude<StateKind, "loading">, string> = {
		empty: t("common.state.empty", "Nothing here yet"),
		notloaded: t("common.state.notloaded", "Not loaded"),
		never: t("common.state.never", "Not reported yet"),
		locked: t("common.state.locked", "Locked"),
		noaccess: t("common.state.noaccess", "No access"),
		unsupported: t("common.state.unsupported", "Not supported"),
		error: t("common.state.error", "Couldn't load"),
		gate: t("common.state.gate", "Not available"),
	};
	const Icon =
		icon ?? (kind === "gate" ? GATE_ICON[gate ?? "hub"] : STATE_ICON[kind]);

	return (
		<div
			data-kind={kind}
			data-gate={kind === "gate" ? gate : undefined}
			className={cx(
				"flex min-w-0 items-start gap-3 rounded-lg border p-4",
				FRAME[kind],
				className,
			)}
		>
			<Icon
				aria-hidden
				className={cx(
					"mt-0.5 size-4 shrink-0 text-muted-foreground",
					ICON_TONE[kind],
				)}
			/>
			<div className="flex min-w-0 flex-col gap-1">
				<p
					className={cx(
						"font-semibold",
						kind === "gate" ? "text-headline" : "text-sm",
					)}
				>
					{title ?? titles[kind]}
				</p>
				{text ? (
					<div className="max-w-[72ch] text-ui text-muted-foreground">
						{text}
					</div>
				) : null}
				{actions ? (
					<div className="mt-2 flex flex-wrap items-center gap-2">
						{actions}
					</div>
				) : null}
			</div>
		</div>
	);
}

/** Values of a block whose data was read before locking (SPEC §4.31 "locked with last data"). */
export const LOCKED_DATA_CLASS =
	"text-muted-foreground [&_td]:text-muted-foreground";

export function LockedDataBanner({
	readAt,
	actions,
	className,
}: Readonly<{
	/** Unix seconds of the last read before the keys were locked. */
	readAt: number;
	actions?: ReactNode;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	return (
		<Banner tone="info" icon={Lock} actions={actions} className={className}>
			{t(
				"common.state.lockedData",
				"Locked. Showing what was read at {{time}}.",
				{
					time: time.clock(readAt),
				},
			)}
		</Banner>
	);
}
