"use client";

import { useTranslation } from "@flow-like/locales";
import { Check, Copy, type LucideIcon } from "lucide-react";
import { type ReactNode, useState } from "react";
import { groupFingerprint } from "../../../../lib/device-management/fingerprint";
import type { AreaTime, DevicesT } from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { TONE_TEXT, cx } from "../primitives/tone";
import { useCopy } from "../primitives/use-copy";
import { stampOf } from "../shell/attention-popover";
import { useEnrollments } from "../workspace";

/** Where the pending setups come from: the hub's list, or on older hubs what this computer tracks (BG2 interim). */
export function PendingSetupsStamp() {
	const { t } = useTranslation("devices");
	const enrollments = useEnrollments();
	return enrollments.missingOnHub ? (
		<FreshnessStamp
			source="local"
			age="current"
			text={t("setup.pending.tracked", "tracked on this computer")}
		/>
	) : (
		<FreshnessStamp {...stampOf(enrollments.freshness)} />
	);
}

/**
 * The agent release inside a package runs out before the package would: the
 * device refuses the package from then on, so this is the time to start it by.
 * `made` tells a package that exists from one that would be made now.
 */
export function startByReleaseText(
	t: DevicesT,
	time: Pick<AreaTime, "at">,
	at: number,
	made: boolean,
): string {
	const values = { time: time.at(at) };
	return made
		? t(
				"devices:setup.startBy",
				"Start it on the device before {{time}}: the hub's agent release runs out then.",
				values,
			)
		: t(
				"devices:setup.startByNew",
				"A package made now has to be started on the device before {{time}}: the hub's agent release runs out then.",
				values,
			);
}

/** A device, file or host name inside a sentence. */
export function Mono({
	className,
	children,
}: Readonly<{ className?: string; children?: ReactNode }>) {
	return (
		<span className={cx("font-mono text-[0.95em] wrap-anywhere", className)}>
			{children}
		</span>
	);
}

export interface IconRow {
	id: string;
	icon: LucideIcon;
	text: ReactNode;
}

/**
 * A key fingerprint in blocks of four with its case kept: it is base64url, so
 * re-casing it (as `IdRef`'s `group4` does) would show another key.
 */
export function KeyFingerprint({
	value,
	copyLabel,
}: Readonly<{ value: string; copyLabel: string }>) {
	const { t } = useTranslation("devices");
	const [revealed, setRevealed] = useState(false);
	const { copied, copy } = useCopy();
	const grouped = groupFingerprint(value);
	const short = `${grouped.split(" ").slice(0, 4).join(" ")} …`;
	return (
		<span
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
				{revealed ? grouped : short}
			</button>
			<DvButton
				variant="ghost"
				size="xs"
				iconOnly
				icon={copied ? Check : Copy}
				aria-label={copied ? t("setup.details.copied", "Copied") : copyLabel}
				onClick={() => {
					void copy(value);
				}}
				className="size-5.5 text-muted-foreground"
			/>
		</span>
	);
}

/** A short list of facts, each led by an icon (requirements, tips, explanations). */
export function IconList({
	rows,
	boxed = false,
	className,
}: Readonly<{
	rows: readonly IconRow[];
	boxed?: boolean;
	className?: string;
}>) {
	return (
		<ul
			className={cx(
				"m-0 flex list-none flex-col gap-2 p-0 text-ui text-ink-2",
				boxed &&
					"rounded-lg border border-hairline bg-surface-sunken px-3.5 py-3",
				className,
			)}
		>
			{rows.map(({ id, icon: Icon, text }) => (
				<li key={id} className="flex items-start gap-2.5">
					<Icon
						aria-hidden
						className="mt-px size-4 shrink-0 text-muted-foreground"
					/>
					<span className="min-w-0">{text}</span>
				</li>
			))}
		</ul>
	);
}

/** The uppercase label above a group that is not a block. */
export function GroupLabel({ children }: Readonly<{ children: ReactNode }>) {
	return (
		<p className="text-label font-semibold tracking-[0.06em] text-muted-foreground uppercase">
			{children}
		</p>
	);
}

/**
 * The technical sentence behind a plain one, with the diagnostics to send to
 * an operator. Neither is translated: both are for a support channel (R3).
 */
export function TechnicalDetails({
	detail,
	diagnostics,
}: Readonly<{ detail: string; diagnostics?: string }>) {
	const { t } = useTranslation("devices");
	const { copied, copy } = useCopy();
	return (
		<details className="text-xs">
			<summary className="cursor-pointer text-muted-foreground">
				{t("setup.details.summary", "Details")}
			</summary>
			<pre className="mt-1.5 overflow-auto rounded-lg border border-border bg-surface-sunken px-3 py-2 font-mono text-xs leading-4.5 whitespace-pre-wrap wrap-anywhere">
				{detail}
			</pre>
			<DvButton
				size="xs"
				icon={copied ? Check : Copy}
				className="mt-1.5"
				onClick={() => {
					void copy(diagnostics ?? detail);
				}}
			>
				{copied
					? t("setup.details.copied", "Copied")
					: t("setup.details.copy", "Copy diagnostics")}
			</DvButton>
		</details>
	);
}

/** An inline message under a field: an error, or a warning that does not block. */
export function FieldNote({
	tone,
	icon: Icon,
	children,
}: Readonly<{
	tone: "critical" | "warning" | "info";
	icon: LucideIcon;
	children: ReactNode;
}>) {
	return (
		<span className={cx("flex items-start gap-1 text-xs", TONE_TEXT[tone])}>
			<Icon aria-hidden className="mt-px size-3.25 shrink-0" />
			<span className="min-w-0">{children}</span>
		</span>
	);
}
