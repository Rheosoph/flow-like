"use client";

import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { Alert } from "../../../ui/alert";
import { TONE_ICON } from "./icons";
import { TONE_SURFACE, TONE_TEXT, cx } from "./tone";

export type BannerTone = "critical" | "warning" | "info" | "good" | "locked";

export interface BannerProps {
	tone: BannerTone;
	icon?: LucideIcon;
	title?: ReactNode;
	children?: ReactNode;
	actions?: ReactNode;
	className?: string;
}

/** SPEC §4.37: page-level conditions only (hub failing, locked data, revoked device, identity mismatch). */
export function Banner({
	tone,
	icon,
	title,
	children,
	actions,
	className,
}: Readonly<BannerProps>) {
	const Icon = icon ?? TONE_ICON[tone];
	return (
		<Alert
			role={tone === "critical" ? "alert" : "status"}
			data-tone={tone}
			className={cx(
				"flex items-start gap-2.5 rounded-lg border px-3.5 py-3 text-[13px]/[18px] text-foreground",
				TONE_SURFACE[tone],
				className,
			)}
		>
			<span className={cx("mt-px shrink-0", TONE_TEXT[tone])}>
				<Icon aria-hidden className="size-4" />
			</span>
			<div className="flex min-w-0 flex-1 flex-col gap-1">
				{title ? (
					<p className="text-sm font-semibold text-foreground">{title}</p>
				) : null}
				{children ? (
					<div className="max-w-[80ch] text-ink-2">{children}</div>
				) : null}
				{actions ? (
					<div className="mt-1.5 flex flex-wrap items-center gap-2">
						{actions}
					</div>
				) : null}
			</div>
		</Alert>
	);
}
