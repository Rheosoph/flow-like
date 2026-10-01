"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CircleCheck,
	CircleDashed,
	CircleSlash,
	LoaderCircle,
	type LucideIcon,
	OctagonX,
	TriangleAlert,
} from "lucide-react";
import type { ReactNode } from "react";
import { sourceLabel } from "./freshness-stamp";
import { SOURCE_ICON, type StampSource } from "./icons";
import { cx } from "./tone";

export type CheckState =
	| "pass"
	| "fail"
	| "warn"
	| "pending"
	| "active"
	| "skip";

const LOOK: Record<
	CheckState,
	{ icon: LucideIcon; iconClass: string; labelClass: string }
> = {
	pass: { icon: CircleCheck, iconClass: "text-good", labelClass: "" },
	fail: {
		icon: OctagonX,
		iconClass: "text-critical",
		labelClass: "text-critical",
	},
	warn: { icon: TriangleAlert, iconClass: "text-warning", labelClass: "" },
	pending: {
		icon: CircleDashed,
		iconClass: "text-unknown",
		labelClass: "text-muted-foreground",
	},
	active: {
		icon: LoaderCircle,
		iconClass: "animate-spin text-info motion-reduce:animate-none",
		labelClass: "font-medium text-info",
	},
	skip: {
		icon: CircleSlash,
		iconClass: "text-muted-foreground",
		labelClass: "text-muted-foreground",
	},
};

export interface CheckSource {
	icon: LucideIcon;
	label: string;
}

export interface ChecklistItem {
	id: string;
	state: CheckState;
	label: ReactNode;
	/** Where the check reads from ("Hub", "This computer", "Hub + this computer"). */
	source?: StampSource | CheckSource;
	note?: ReactNode;
	/** The fix control(s) under a failing check. */
	fix?: ReactNode;
}

/** SPEC §4.37 `.checks`: readiness, pre-flight and progress lists. The state is spoken, not only coloured. */
export function Checklist({
	items,
	label,
	className,
}: Readonly<{
	items: readonly ChecklistItem[];
	label?: string;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	const stateText: Record<CheckState, string> = {
		pass: t("view.check.pass", "Passed"),
		fail: t("view.check.fail", "Failed"),
		warn: t("view.check.warn", "Warning"),
		pending: t("view.check.pending", "Not checked yet"),
		active: t("view.check.active", "Checking"),
		skip: t("view.check.skip", "Skipped"),
	};
	return (
		<ul aria-label={label} className={cx("flex flex-col", className)}>
			{items.map((item) => {
				const look = LOOK[item.state];
				const Icon = look.icon;
				const source =
					typeof item.source === "string"
						? {
								icon: SOURCE_ICON[item.source],
								label: sourceLabel(t, item.source),
							}
						: item.source;
				const SourceIcon = source?.icon;
				return (
					<li
						key={item.id}
						data-state={item.state}
						className="grid grid-cols-[18px_minmax(0,1fr)_auto] items-start gap-x-2.5 gap-y-0.5 border-t border-hairline py-1.75 text-ui first:border-t-0"
					>
						<span
							role="img"
							aria-label={stateText[item.state]}
							className="mt-px"
						>
							<Icon aria-hidden className={cx("size-4", look.iconClass)} />
						</span>
						<span className={cx("min-w-0", look.labelClass)}>{item.label}</span>
						{source && SourceIcon ? (
							<span className="inline-flex h-5 items-center gap-1 rounded-sm border border-hairline px-1.5 text-xs whitespace-nowrap text-muted-foreground">
								<SourceIcon aria-hidden className="size-3" />
								{source.label}
							</span>
						) : (
							<span />
						)}
						{item.note ? (
							<p className="col-start-2 col-end-[-1] text-xs text-muted-foreground">
								{item.note}
							</p>
						) : null}
						{item.fix ? (
							<div className="col-start-2 col-end-[-1] mt-1 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
								{item.fix}
							</div>
						) : null}
					</li>
				);
			})}
		</ul>
	);
}
