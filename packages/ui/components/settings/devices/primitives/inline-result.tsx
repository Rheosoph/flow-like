"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CircleCheck,
	CircleDashed,
	LoaderCircle,
	type LucideIcon,
	OctagonX,
	TriangleAlert,
} from "lucide-react";
import type { ReactNode } from "react";
import { DvButton } from "./dv-button";
import { TONE_SURFACE, TONE_TEXT, cx } from "./tone";

export type ResultTone = "good" | "info" | "warning" | "critical" | "unknown";

const RESULT_ICON: Record<ResultTone, LucideIcon> = {
	good: CircleCheck,
	info: LoaderCircle,
	warning: TriangleAlert,
	critical: OctagonX,
	unknown: CircleDashed,
};

/** SPEC §4.34 / R9: the outcome next to the control that started it; stays until dismissed. */
export function InlineResult({
	tone,
	actions,
	onDismiss,
	className,
	children,
}: Readonly<{
	tone: ResultTone;
	/** Follow-ups ("Check result", "Follow in activity"). */
	actions?: ReactNode;
	onDismiss?: () => void;
	className?: string;
	children: ReactNode;
}>) {
	const { t } = useTranslation("devices");
	const Icon = RESULT_ICON[tone];
	return (
		<div
			role={tone === "critical" ? "alert" : "status"}
			data-result={tone}
			className={cx(
				"flex max-w-[72ch] flex-wrap items-start gap-x-1.5 gap-y-1 rounded-lg border px-2 py-1.5 text-xs text-foreground",
				TONE_SURFACE[tone],
				className,
			)}
		>
			<Icon
				aria-hidden
				className={cx(
					"mt-px size-3.25 shrink-0",
					TONE_TEXT[tone],
					tone === "info" && "animate-spin",
				)}
			/>
			<span className="min-w-0 flex-[1_1_24ch]">{children}</span>
			{actions}
			{onDismiss ? (
				<DvButton variant="link" size="xs" onClick={onDismiss}>
					{t("common.result.dismiss", "Dismiss")}
				</DvButton>
			) : null}
		</div>
	);
}
