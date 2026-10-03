"use client";

import { useTranslation } from "@flow-like/locales";
import { type DevicesT, useAreaTime } from "./area-context";
import { cx } from "./tone";

/** One quarter-hour: checked in, missed, or before the device was registered. */
export type LaneTick = "ok" | "miss" | "pre";

export const LANE_SLOTS = 96;
export const LANE_SLOT_SEC = 900;

/** "Checked in during 84 of the last 96 quarter-hours. Missed every one since 11:00." */
export function laneLabel(
	t: DevicesT,
	ticks: readonly LaneTick[],
	sinceText: (slotIndex: number) => string,
): string {
	const counted = ticks.filter((tick) => tick !== "pre").length;
	if (!counted)
		return t(
			"devices:view.lane.none",
			"Not checking in: revoked or not registered in the last 24 hours.",
		);
	const ok = ticks.filter((tick) => tick === "ok").length;
	const head = t(
		"devices:view.lane.label",
		"Checked in during {{ok, number}} of the last {{count, number}} quarter-hours.",
		{ ok, count: counted },
	);
	const lastOk = ticks.lastIndexOf("ok");
	if (ticks.at(-1) === "miss" && lastOk < ticks.length - 1) {
		return lastOk >= 0
			? t(
					"devices:view.lane.missedSince",
					"{{head}} Missed every one since {{time}}.",
					{
						head,
						time: sinceText(lastOk + 1),
					},
				)
			: t("devices:view.lane.notYet", "{{head}} It hasn't checked in yet.", {
					head,
				});
	}
	return counted > ok
		? t("devices:view.lane.missed", "{{head}} Missed {{count, number}}.", {
				head,
				count: counted - ok,
			})
		: head;
}

/**
 * SPEC §4.26: 96 quarter-hour ticks at a fixed 112 px so lanes align down a
 * column ("Last 24 h" lives in the column header).
 */
export function CheckinLane({
	ticks,
	endAt,
	className,
}: Readonly<{
	/** Oldest first; the last tick ends at `endAt`. */
	ticks: readonly LaneTick[];
	/** Unix seconds the last slot ends; the area clock otherwise. */
	endAt?: number;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	if (!ticks.length) return null;
	const end = endAt ?? time.nowS;
	const label = laneLabel(t, ticks, (slot) =>
		time.at(end - (ticks.length - slot) * LANE_SLOT_SEC),
	);
	return (
		<span
			role="img"
			aria-label={label}
			title={label}
			data-lane=""
			className={cx("inline-flex shrink-0 align-middle", className)}
		>
			<svg
				viewBox="0 0 191 12"
				width="112"
				height="12"
				preserveAspectRatio="none"
				aria-hidden="true"
				className="block"
			>
				{ticks.map((tick, index) =>
					tick === "ok" ? (
						<rect
							// biome-ignore lint/suspicious/noArrayIndexKey: ticks are positional
							key={index}
							data-tick="ok"
							className="fill-lane-ok"
							x={2 * index}
							y="4"
							width="1"
							height="8"
						/>
					) : tick === "miss" ? (
						<rect
							// biome-ignore lint/suspicious/noArrayIndexKey: ticks are positional
							key={index}
							data-tick="miss"
							className="fill-critical-solid"
							x={2 * index}
							y="0"
							width="1"
							height="12"
						/>
					) : (
						<rect
							// biome-ignore lint/suspicious/noArrayIndexKey: ticks are positional
							key={index}
							data-tick="pre"
							className="fill-hairline"
							x={2 * index}
							y="11"
							width="1"
							height="1"
						/>
					),
				)}
			</svg>
		</span>
	);
}
