"use client";

import { useTranslation } from "@flow-like/locales";
import { Check, Copy } from "lucide-react";
import { type ReactNode, useState } from "react";
import type { Freshness } from "../../../../lib/device-management/model/types";
import { useAreaPrefs } from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { cx } from "../primitives/tone";
import { useCopy } from "../primitives/use-copy";
import { stampOf } from "../shell/attention-popover";
import { useHubSupport } from "../workspace";

/** The active language for number and size formatting, without following the area clock. */
export function useLocale(): string {
	const { i18n } = useTranslation("devices");
	return i18n?.language ?? "en";
}

/** A raw key or code, shown only with developer mode on (R3). */
export function Tech({ children }: Readonly<{ children: ReactNode }>) {
	const { showTechnicalKeys } = useAreaPrefs();
	if (!showTechnicalKeys) return null;
	return (
		<span
			data-tech=""
			className="ml-1.5 font-mono text-[11px] font-normal text-muted-foreground"
		>
			{children}
		</span>
	);
}

/**
 * The app's base layer gives every `table` a margin and every cell a full
 * border; inside a block only the row hairlines of the table primitive remain.
 * A chip in a cell has 4 px corners.
 */
export const TABLE_RESET =
	"my-0 [&_td]:border-x-0 [&_td]:border-b-0 [&_th]:border-x-0 [&_th]:border-t-0 [&_td_[data-slot=badge]]:rounded-md";

/**
 * While devices are off the device list can't be read, which the area counts
 * as a failing hub. This page's blocks read other routes, so each states the
 * age of its own read instead.
 */
export function useOwnStampAge(): boolean {
	return useHubSupport().support.state === "off";
}

/** A block head's Hub stamp: "checking…" until the first answer, then source and age (R5). */
export function HubReadStamp({
	freshness,
	loading,
}: Readonly<{ freshness: Freshness; loading: boolean }>) {
	const { t } = useTranslation("devices");
	const own = useOwnStampAge();
	if (loading)
		return (
			<FreshnessStamp
				source="hub"
				age="notloaded"
				text={t("hub.stamp.checking", "checking…")}
				noFail={own}
			/>
		);
	return <FreshnessStamp {...stampOf(freshness)} noFail={own} />;
}

/** One address on one line with Copy; the full value is in the hover. */
export function UrlLine({
	url,
	copyLabel,
}: Readonly<{ url: string; copyLabel: string }>) {
	const { t } = useTranslation("devices");
	const { copied, copy } = useCopy();
	return (
		<span className="flex max-w-full min-w-0 items-center gap-0.5">
			<span title={url} className="min-w-0 truncate font-mono text-[12.5px]">
				{url}
			</span>
			<DvButton
				variant="ghost"
				size="xs"
				iconOnly
				icon={copied ? Check : Copy}
				aria-label={copied ? t("hub.copied", "Copied") : copyLabel}
				onClick={() => {
					void copy(url);
				}}
				className="size-5.5 shrink-0 text-muted-foreground"
			/>
		</span>
	);
}

/**
 * A key fingerprint in groups of four with its case kept. It is base64url, so
 * re-casing it (as `IdRef group4` does) would show another key. Copy takes the
 * raw value.
 */
export function Fingerprint({
	value,
	copyLabel,
}: Readonly<{ value: string; copyLabel: string }>) {
	const { t } = useTranslation("devices");
	const [revealed, setRevealed] = useState(false);
	const { copied, copy } = useCopy();
	const groups = value.match(/.{1,4}/g) ?? [value];
	const short = `${groups.slice(0, 4).join(" ")} …`;
	return (
		<span
			data-idref=""
			data-fingerprint=""
			className="inline-flex max-w-full min-w-0 items-center gap-0.5 align-middle"
		>
			<button
				type="button"
				title={value}
				aria-expanded={revealed}
				onClick={() => setRevealed((current) => !current)}
				className={cx(
					"min-w-0 cursor-pointer rounded-sm border border-hairline bg-surface-sunken px-1.5 py-px text-left font-mono text-xs text-ink-2 hover:border-border-strong focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring",
					revealed
						? "wrap-normal break-normal whitespace-normal"
						: "truncate whitespace-nowrap",
				)}
			>
				{revealed ? groups.join(" ") : short}
			</button>
			<DvButton
				variant="ghost"
				size="xs"
				iconOnly
				icon={copied ? Check : Copy}
				aria-label={copied ? t("hub.copied", "Copied") : copyLabel}
				onClick={() => {
					void copy(value);
				}}
				className="size-5.5 text-muted-foreground"
			/>
		</span>
	);
}

export function Hint({
	className,
	children,
}: Readonly<{ className?: string; children: ReactNode }>) {
	return (
		<p className={cx("text-xs text-muted-foreground", className)}>{children}</p>
	);
}

/** A sub-heading inside a block, with a muted note beside it. */
export function SectionHead({
	title,
	note,
	className,
}: Readonly<{ title: ReactNode; note?: ReactNode; className?: string }>) {
	return (
		<div
			className={cx(
				"flex flex-wrap items-center gap-x-2.5 gap-y-1.5",
				className,
			)}
		>
			<h3 className="text-ui font-semibold">{title}</h3>
			{note ? (
				<span className="text-xs text-muted-foreground">{note}</span>
			) : null}
		</div>
	);
}

/** A big mono figure with its unit ("24 hours"). */
export function Figure({
	value,
	unit,
	className,
}: Readonly<{ value: ReactNode; unit?: ReactNode; className?: string }>) {
	return (
		<div
			className={cx(
				"font-mono text-xl/[26px] font-semibold whitespace-nowrap tabular-nums",
				className,
			)}
		>
			{value}
			{unit ? (
				<small className="ml-1 font-sans text-ui font-medium text-muted-foreground">
					{unit}
				</small>
			) : null}
		</div>
	);
}
