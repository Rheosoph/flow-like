"use client";

import { useTranslation } from "@flow-like/locales";
import { ChevronDown, OctagonX } from "lucide-react";
import { cx } from "../../../settings/devices/primitives/tone";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../../../ui/dropdown-menu";
import type { FormSessionState, RunEntry } from "../contracts";
import type { FormWords } from "../model/date-text";
import { formatClock, formatTook } from "../run/format";
import { elapsedMs } from "../run/run-view";
import { statusWord } from "../status-look";
import { MENU_CONTENT, MENU_ITEM } from "./menu-style";
import type { PaneEnv } from "./pane-env";
import { momentOf } from "./stage-text";
import { StatusIcon } from "./status-icon";
import { type StripTabs, overflowMenuOf, ticksClock } from "./strip-model";

export interface OverflowMenuProps {
	readonly state: Pick<FormSessionState, "view">;
	readonly env: PaneEnv;
	readonly tabs: StripTabs;
	readonly now: number;
	readonly locale: string;
	readonly words: FormWords;
}

/** "4:07 PM · 48 s" for a finished run, the status word for one that has not started. */
function whenText(
	run: RunEntry,
	now: number,
	locale: string,
	words: FormWords,
	statusText: string,
) {
	const { time } = momentOf(run.createdAt, now, locale, words);
	if (ticksClock(run)) return `${time} · ${formatClock(elapsedMs(run, now))}`;
	if (run.startedAt !== null && run.endedAt !== null)
		return `${time} · ${formatTook(elapsedMs(run, now))}`;
	return `${time} · ${statusText}`;
}

function OverflowRow({
	run,
	env,
	now,
	locale,
	words,
}: Readonly<{
	run: RunEntry;
	env: PaneEnv;
	now: number;
	locale: string;
	words: FormWords;
}>) {
	const { t } = useTranslation("interfaces");
	const moment = momentOf(run.createdAt, now, locale, words);
	return (
		<DropdownMenuItem
			className={cx(MENU_ITEM, "h-8 gap-2", env.touch && "h-11")}
			onSelect={() => env.actions.selectRun(run.id, "menu")}
		>
			<StatusIcon status={run.status} className="size-3.5" />
			<span className="font-medium">
				{t("workbench.stage.tab.run", "Run {{n}}", { n: run.n })}
			</span>
			<span className="min-w-0 flex-1 truncate text-muted-foreground text-xs">
				{moment.day}
			</span>
			<span className="whitespace-nowrap font-mono text-muted-foreground text-xs tabular-nums [word-spacing:-0.25em]">
				{whenText(run, now, locale, words, statusWord(t, run.status))}
			</span>
		</DropdownMenuItem>
	);
}

/** "N more", with "1 failed" while a failure nobody has looked at hides in it (spec M5). */
export function OverflowMenu({
	state,
	env,
	tabs,
	now,
	locale,
	words,
}: Readonly<OverflowMenuProps>) {
	const { t } = useTranslation("interfaces");
	const open = state.view.overlay?.id === "moreRuns";
	const { failures, others } = overflowMenuOf(tabs);
	const row = (run: RunEntry) => (
		<OverflowRow
			key={run.id}
			run={run}
			env={env}
			now={now}
			locale={locale}
			words={words}
		/>
	);
	return (
		<DropdownMenu
			modal={false}
			open={open}
			onOpenChange={(next) => {
				if (next) env.actions.openOverlay({ id: "moreRuns" });
				else if (open) env.actions.closeOverlay();
			}}
		>
			<DropdownMenuTrigger asChild>
				<button
					type="button"
					className={cx(
						"inline-flex items-center gap-1.5 whitespace-nowrap rounded-lg py-0 pr-2 pl-2.5 font-medium text-[12.5px] text-ink-2 outline-ring hover:bg-row-hover focus-visible:outline-2 focus-visible:outline-offset-1",
						env.touch ? "h-11" : "h-7",
					)}
				>
					<span>
						{t("workbench.stage.more.label", "{{count}} more", {
							count: tabs.overflow.length,
							defaultValue_one: "{{count}} more",
						})}
					</span>
					{failures.length > 0 ? (
						<span className="inline-flex items-center gap-1.5 text-critical">
							<OctagonX aria-hidden className="size-3.25" />
							<span>
								{t("workbench.stage.more.failed", "{{count}} failed", {
									count: failures.length,
									defaultValue_one: "{{count}} failed",
								})}
							</span>
						</span>
					) : null}
					<ChevronDown aria-hidden className="size-3.25" />
				</button>
			</DropdownMenuTrigger>
			<DropdownMenuContent
				align="end"
				className={cx(MENU_CONTENT, "max-h-90 w-66")}
			>
				{failures.map(row)}
				{failures.length > 0 && others.length > 0 ? (
					<DropdownMenuSeparator className="my-1 bg-hairline" />
				) : null}
				{others.map(row)}
			</DropdownMenuContent>
		</DropdownMenu>
	);
}
