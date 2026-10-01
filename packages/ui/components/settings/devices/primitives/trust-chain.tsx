"use client";

import { useTranslation } from "@flow-like/locales";
import type { LucideIcon } from "lucide-react";
import { Fragment, type ReactNode } from "react";
import { cx } from "./tone";

export type ChainState = "good" | "warning" | "critical" | "locked" | "unknown";

export interface ChainLink {
	id: string;
	icon: LucideIcon;
	state: ChainState;
	/** "Keys on this computer". */
	title: ReactNode;
	/** "Owner keys · backed up to your account (v3)". */
	text: ReactNode;
	/** The sentence joining this link to the next one ("you trusted this device's keys on 14 Mar 2026"). */
	join?: ReactNode;
}

const ICON_LOOK: Record<ChainState, string> = {
	good: "rounded-full border-good-line bg-good-bg text-good",
	warning: "rounded-full border-warning-line bg-warning-bg text-warning",
	critical: "rounded-sm border-critical-line bg-critical-bg text-critical",
	locked: "rounded-full border-locked-line bg-locked-bg text-locked",
	unknown:
		"rounded-full border-dashed border-border-strong bg-card text-muted-foreground",
};

/**
 * SPEC §4.28: keys → identity → access rules → certificates, joined by the
 * sentence that makes each link trust the next. `compact` drops the joins.
 */
export function TrustChain({
	links,
	compact = false,
	label,
	className,
}: Readonly<{
	links: readonly ChainLink[];
	compact?: boolean;
	label?: string;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	const stateWord: Record<ChainState, string> = {
		good: t("view.chain.good", "OK"),
		warning: t("view.chain.warning", "Needs attention"),
		critical: t("view.chain.critical", "Broken"),
		locked: t("view.chain.locked", "Locked"),
		unknown: t("view.chain.unknown", "Unknown"),
	};
	return (
		<ol
			aria-label={label ?? t("view.chain.label", "Trust chain")}
			data-compact={compact ? "" : undefined}
			className={cx(
				"relative flex flex-col before:absolute before:top-3.5 before:bottom-3.5 before:left-[11px] before:w-px before:bg-border-strong",
				className,
			)}
		>
			{links.map((link, index) => {
				const Icon = link.icon;
				const join =
					!compact && link.join && index < links.length - 1 ? link.join : null;
				return (
					<Fragment key={link.id}>
						<li
							data-state={link.state}
							className="relative grid grid-cols-[22px_minmax(0,1fr)] items-start gap-2.5 py-1"
						>
							<span
								className={cx(
									"inline-flex size-5.5 items-center justify-center border",
									ICON_LOOK[link.state],
								)}
							>
								<Icon aria-hidden className="size-3" />
							</span>
							<div className="min-w-0">
								<b className="text-ui font-semibold">
									{link.title}
									<span className="sr-only">
										{t("view.chain.stateSr", " ({{state}})", {
											state: stateWord[link.state],
										})}
									</span>
								</b>
								<p className="mt-px text-xs text-muted-foreground">
									{link.text}
								</p>
							</div>
						</li>
						{join ? (
							<li
								data-join=""
								className="relative pt-0.5 pb-1 pl-8 text-xs text-muted-foreground"
							>
								{join}
							</li>
						) : null}
					</Fragment>
				);
			})}
		</ol>
	);
}
