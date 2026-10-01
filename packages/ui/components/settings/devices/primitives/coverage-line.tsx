"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { CircleDashed } from "lucide-react";
import type { ReactNode } from "react";
import { cx } from "./tone";

export interface CoverageLineProps {
	/** Devices whose status can be read now. */
	readable: number;
	/** Non-revoked devices the viewer can see. */
	total: number;
	live: number;
	snapshot: number;
	/** Not readable now (locked, no keys, no access), excluding `never`. */
	unknown: number;
	/** Registered but never checked in. */
	never?: number;
	/** "Unlock 2…". */
	actions?: ReactNode;
	className?: string;
}

const share = (part: number, total: number) =>
	`${total > 0 ? Math.max(0, Math.min(100, (part / total) * 100)) : 0}%`;

/** SPEC §4.37: whose status the page can vouch for, with a flat live / snapshot / unknown bar. */
export function CoverageLine({
	readable,
	total,
	live,
	snapshot,
	unknown,
	never = 0,
	actions,
	className,
}: Readonly<CoverageLineProps>) {
	const { t } = useTranslation("devices");
	const tail = [
		unknown > 0
			? t("view.coverage.unknown", "· {{count, number}} unknown", {
					count: unknown,
				})
			: null,
		never > 0
			? t("view.coverage.never", {
					count: never,
					defaultValue_one: "· {{count, number}} hasn't checked in",
					defaultValue_other: "· {{count, number}} haven't checked in",
				})
			: null,
	].filter(Boolean);
	const bar = t(
		"view.coverage.bar",
		"{{live, number}} live, {{snapshot, number}} from encrypted snapshots, {{unknown, number}} unknown",
		{ live, snapshot, unknown: unknown + never },
	);
	return (
		<div
			data-coverage=""
			className={cx(
				"flex flex-wrap items-center gap-x-2.5 gap-y-1.5 text-ui text-ink-2",
				className,
			)}
		>
			<CircleDashed aria-hidden className="size-4 text-muted-foreground" />
			<span>
				<Trans
					t={t}
					i18nKey="view.coverage.text"
					defaults="Status from <1>{{readable, number}} of {{total, number}}</1> devices you can see"
					values={{ readable, total }}
					components={{ 1: <b className="font-semibold text-foreground" /> }}
				/>
			</span>
			{tail.map((part) => (
				<span key={part} className="-ml-1">
					{part}
				</span>
			))}
			<span
				role="img"
				aria-label={bar}
				className="inline-flex h-1.5 w-30 gap-px overflow-hidden rounded-[3px] bg-muted"
			>
				<i
					data-part="live"
					className="block h-full bg-good-solid"
					style={{ width: share(live, total) }}
				/>
				<i
					data-part="snapshot"
					className="block h-full bg-unknown-solid"
					style={{ width: share(snapshot, total) }}
				/>
			</span>
			{actions}
		</div>
	);
}
