"use client";

import { useTranslation } from "@flow-like/locales";
import { Layers } from "lucide-react";
import type { AreaTime, DevicesT } from "./area-context";
import { useAreaTime, useHubFreshness } from "./area-context";
import { SOURCE_ICON, type StampAge, type StampSource } from "./icons";
import { cx } from "./tone";

/** A stamp's identity for R5 comparisons (block head vs row). */
export interface StampSpec {
	source: StampSource;
	age: StampAge;
	/** Hub verb: "checked" (default) or "updated". */
	verb?: "checked" | "updated";
	text?: string;
}

export interface StampError {
	/** Unix seconds of the last good read. */
	dataFrom?: number;
	/** Unix seconds of the next automatic retry. */
	retryAt?: number;
	/** Translated one-line cause for the hover. */
	reason?: string;
}

export interface FreshnessStampProps extends StampSpec {
	/** Unix seconds the data was observed (device) or fetched (hub). */
	observedAt?: number;
	/** Producer cadence in seconds; defaults per source (hub 30, snap 60, live 15). */
	cadenceSec?: number;
	/** Detected clock skew in seconds (device clock minus hub clock). */
	skewSec?: number;
	/** Refresh failure for this block; hub stamps also follow `useHubFreshness()`. */
	error?: StampError;
	/** Table cells and dense rows: no box, source word visually hidden. */
	compact?: boolean;
	/** Block heads, attention items, popovers. */
	boxed?: boolean;
	/** Keep the age under a failing hub (per-row "reported at" stamps, R5). */
	noFail?: boolean;
	className?: string;
}

const DEFAULT_CADENCE: Partial<Record<StampSource, number>> = {
	hub: 30,
	snap: 60,
	live: 15,
};

const GLYPH: Record<StampAge, string> = {
	live: "size-[7px] rounded-full bg-good-solid",
	current: "size-[7px] rounded-full bg-good-solid",
	delayed: "size-[7px] rounded-[1px] bg-warning-solid",
	lastknown: "size-[7px] rounded-full border-[1.5px] border-unknown",
	snapshot: "size-[7px] rounded-full border-[1.5px] border-unknown",
	locked: "size-[7px] rounded-[1px] border-[1.5px] border-locked",
	notloaded: "h-0.5 w-[7px] bg-unknown",
	noaccess: "h-0.5 w-[7px] bg-unknown",
	unsupported: "h-0.5 w-[7px] bg-unknown",
	error: "size-1.5 rotate-45 rounded-[1px] bg-critical-solid",
};

export function sourceLabel(t: DevicesT, source: StampSource): string {
	const labels = {
		hub: t("devices:common.stamp.source.hub", "Hub"),
		snap: t("devices:common.stamp.source.snap", "Encrypted snapshot"),
		saved: t("devices:common.stamp.source.saved", "Snapshot"),
		live: t("devices:common.stamp.source.live", "Live"),
		local: t("devices:common.stamp.source.local", "This computer"),
		device: t("devices:common.stamp.source.device", "On-device only"),
	} satisfies Record<StampSource, string>;
	return labels[source];
}

export function ageLabel(t: DevicesT, age: StampAge): string {
	const labels = {
		live: t("devices:enum.freshness.live", "Live"),
		current: t("devices:enum.freshness.current", "Current"),
		delayed: t("devices:enum.freshness.delayed", "Delayed"),
		lastknown: t("devices:enum.freshness.lastknown", "Last known"),
		snapshot: t("devices:enum.freshness.snapshot", "Snapshot"),
		locked: t("devices:enum.freshness.locked", "Locked"),
		notloaded: t("devices:enum.freshness.notloaded", "Not loaded"),
		noaccess: t("devices:enum.freshness.noaccess", "No access"),
		unsupported: t("devices:enum.freshness.unsupported", "Not supported"),
		error: t("devices:enum.freshness.error", "Couldn't refresh"),
	} satisfies Record<StampAge, string>;
	return labels[age];
}

/** SPEC §6.1 plane explanations, with the producer cadence. */
export function planeText(
	t: DevicesT,
	source: StampSource,
	cadenceSec: number,
): string {
	const texts = {
		hub: t(
			"devices:common.stamp.plane.hub",
			"Registry data any signed-in account can see without a password. The app checks every {{count}} s.",
			{ count: cadenceSec },
		),
		snap: t(
			"devices:common.stamp.plane.snap",
			"Devices send their status to the hub encrypted. Only keys on this computer can read it, even while a device is offline. Devices send it when something changes, at least every {{count}} s.",
			{ count: cadenceSec },
		),
		saved: t(
			"devices:common.stamp.plane.saved",
			"Saved at the last live read. It never claims to be current.",
		),
		live: t(
			"devices:common.stamp.plane.live",
			"Read over the live connection; refreshed every {{count}} s while connected.",
			{ count: cadenceSec },
		),
		local: t(
			"devices:common.stamp.plane.local",
			"Stored in this app on this computer. Invisible from other browsers, profiles, hubs or accounts.",
		),
		device: t(
			"devices:common.stamp.plane.device",
			"Only visible with a shell on the device. The app can point you there.",
		),
	} satisfies Record<StampSource, string>;
	return texts[source];
}

function failureText(t: DevicesT, time: AreaTime, failure: StampError): string {
	const from =
		failure.dataFrom === undefined ? undefined : time.clock(failure.dataFrom);
	const retry =
		failure.retryAt === undefined ? undefined : time.countdown(failure.retryAt);
	if (from && retry) {
		return t(
			"devices:common.stamp.errorFromRetry",
			"couldn't refresh · data from {{from}} · retry in {{retry}}",
			{ from, retry },
		);
	}
	if (from) {
		return t(
			"devices:common.stamp.errorFrom",
			"couldn't refresh · data from {{from}}",
			{ from },
		);
	}
	return t("devices:common.stamp.error", "couldn't refresh");
}

interface AgeTextInput {
	age: StampAge;
	source: StampSource;
	verb: "checked" | "updated";
	ago: string;
	cadence: number;
	/** Clock time of the last read, for locked stamps. */
	lockedAt?: string;
}

function currentText(t: DevicesT, { source, verb, ago }: AgeTextInput): string {
	if (source === "local") {
		return ago
			? t("devices:common.stamp.readAgo", "read {{ago}}", { ago })
			: t("devices:common.stamp.storedHere", "stored here");
	}
	if (!ago) return ageLabel(t, "current");
	if (source !== "hub") {
		return t("devices:common.stamp.currentAgo", "Current · {{ago}}", { ago });
	}
	return verb === "updated"
		? t("devices:common.stamp.updatedAgo", "updated {{ago}}", { ago })
		: t("devices:common.stamp.checkedAgo", "checked {{ago}}", { ago });
}

/** Ages whose text is "<word> · <how long ago>": without a time only the word is left. */
const TIMED_AGES: readonly StampAge[] = ["delayed", "lastknown", "snapshot"];

function ageText(t: DevicesT, input: AgeTextInput): string {
	const { ago, cadence, lockedAt } = input;
	if (!ago && TIMED_AGES.includes(input.age)) return ageLabel(t, input.age);
	const texts = {
		live: () =>
			ago
				? t(
						"devices:common.stamp.liveAgo",
						"read {{ago}} · every {{count}} s",
						{ ago, count: cadence },
					)
				: t("devices:common.stamp.following", "following"),
		current: () => currentText(t, input),
		delayed: () =>
			t("devices:common.stamp.delayedAgo", "Delayed · {{ago}}", { ago }),
		lastknown: () =>
			t("devices:common.stamp.lastknownAgo", "Last known · {{ago}}", { ago }),
		snapshot: () =>
			t("devices:common.stamp.snapshotAgo", "{{ago}} · not live", { ago }),
		locked: () =>
			lockedAt === undefined
				? t("devices:common.stamp.locked", "locked")
				: t("devices:common.stamp.lockedRead", "locked · last read {{time}}", {
						time: lockedAt,
					}),
		notloaded: () => t("devices:common.stamp.notloaded", "not loaded"),
		noaccess: () => t("devices:common.stamp.noaccess", "no access"),
		unsupported: () => t("devices:common.stamp.unsupported", "not supported"),
		error: () => t("devices:common.stamp.error", "couldn't refresh"),
	} satisfies Record<StampAge, () => string>;
	return texts[input.age]();
}

function observedText(
	t: DevicesT,
	time: AreaTime,
	source: StampSource,
	observedAt: number | undefined,
): string {
	if (observedAt === undefined) return "";
	const at = time.abs(observedAt);
	return source === "hub"
		? t("devices:common.stamp.checkedAt", "Checked {{at}}", { at })
		: t("devices:common.stamp.readAt", "Read {{at}}", { at });
}

function skewText(t: DevicesT, skewSec: number | undefined): string {
	if (!skewSec || Math.abs(skewSec) < 1) return "";
	return t(
		"devices:common.stamp.skew",
		"The device clock differs from the hub by {{count}} s.",
		{ count: Math.round(Math.abs(skewSec)) },
	);
}

/** SPEC §4.1: source + age glyph + age text; absolute time, cadence and skew in the hover (R5, R16). */
export function FreshnessStamp({
	source,
	age: ageIn,
	verb = "checked",
	text: textIn,
	observedAt,
	cadenceSec,
	skewSec,
	error,
	compact = false,
	boxed = false,
	noFail = false,
	className,
}: Readonly<FreshnessStampProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const hub = useHubFreshness();
	const hubFail = source === "hub" && hub.failing && !noFail;
	const failure: StampError | undefined = hubFail
		? { dataFrom: hub.dataFrom, retryAt: hub.retryAt, reason: hub.reason }
		: (error ?? (ageIn === "error" ? {} : undefined));
	const age: StampAge = failure ? "error" : ageIn;
	/* A device clock ahead of the area clock must not read "in 40 s": an observation is never in the future. */
	const ago =
		observedAt === undefined ? "" : time.ago(Math.min(observedAt, time.nowS));
	const cadence = cadenceSec ?? DEFAULT_CADENCE[source] ?? 0;

	const text = failure
		? failureText(t, time, failure)
		: (textIn ??
			ageText(t, {
				age,
				source,
				verb,
				ago,
				cadence,
				lockedAt: observedAt === undefined ? undefined : time.clock(observedAt),
			}));
	const title = [
		ageLabel(t, age),
		observedText(t, time, source, observedAt),
		planeText(t, source, cadence),
		skewText(t, skewSec),
		failure?.reason ?? "",
	]
		.filter(Boolean)
		.join(" · ");

	const SourceIcon = SOURCE_ICON[source];
	const isBoxed = !compact && (boxed || age === "error");

	return (
		<span
			data-stamp=""
			data-src={source}
			data-age={age}
			title={title}
			className={cx(
				"inline-flex max-w-full min-w-0 items-center align-middle text-xs whitespace-nowrap text-muted-foreground",
				compact ? "gap-1" : "gap-1.5",
				isBoxed && "h-5.5 rounded-sm border border-hairline bg-card px-1.5",
				age === "error" && !compact && "border-critical-line bg-critical-bg",
				className,
			)}
		>
			<span className="inline-flex shrink-0 items-center gap-1 font-medium">
				<SourceIcon aria-hidden className="size-3" />
				<span className={compact ? "sr-only" : undefined}>
					{sourceLabel(t, source)}
				</span>
			</span>
			<span
				className={cx(
					"inline-flex min-w-0 items-center gap-1",
					age === "error" && "text-critical",
				)}
			>
				<span
					aria-hidden
					data-glyph={age}
					className={cx("inline-block shrink-0", GLYPH[age])}
				/>
				<span className="min-w-0 truncate">{text}</span>
			</span>
		</span>
	);
}

/** For a block head whose rows come from different sources (each row states its own). */
export function MixedSourcesStamp({
	compact = false,
	className,
}: Readonly<{ compact?: boolean; className?: string }>) {
	const { t } = useTranslation("devices");
	const label = t("common.stamp.mixed", "Sources vary per item");
	return (
		<span
			data-stamp=""
			data-src="mixed"
			title={t(
				"common.stamp.mixedTitle",
				"Each item states its own source and age.",
			)}
			className={cx(
				"inline-flex items-center gap-1 text-xs font-medium text-muted-foreground",
				className,
			)}
		>
			<Layers aria-hidden className="size-3" />
			<span className={compact ? "sr-only" : undefined}>{label}</span>
		</span>
	);
}

const specKey = (spec: StampSpec) =>
	`${spec.source}|${spec.age}|${spec.verb ?? ""}|${spec.text ?? ""}`;

/** R5: a row repeats a stamp only when its source or age state differs from the block's. */
export function sameSource(
	a: StampSpec | null | undefined,
	b: StampSpec | null | undefined,
): boolean {
	return !!a && !!b && specKey(a) === specKey(b);
}

/** The most common stamp among rows (hub wins ties), or null when fewer than two share one. */
export function baseSource<T extends StampSpec>(
	items: readonly (T | null | undefined)[],
): T | null {
	const counts = new Map<string, number>();
	let best: T | null = null;
	let bestCount = 0;
	for (const item of items) {
		if (!item) continue;
		const key = specKey(item);
		const count = (counts.get(key) ?? 0) + 1;
		counts.set(key, count);
		if (
			count > bestCount ||
			(count === bestCount && item.source === "hub" && best?.source !== "hub")
		) {
			best = item;
			bestCount = count;
		}
	}
	return bestCount >= 2 ? best : null;
}
