"use client";

import { useTranslation } from "@flow-like/locales";
import { Pin } from "lucide-react";
import type { KeyboardEvent } from "react";
import { cx } from "../../../settings/devices/primitives/tone";
import { FOCUS_ATTR, FOCUS_VALUE, type RunEntry } from "../contracts";
import { formatClock, formatTook } from "../run/format";
import { statusWord } from "../status-look";
import type { InterfacesT } from "./copy";
import { paneIdOf, tabIdOf } from "./pane-env";
import { StatusIcon } from "./status-icon";
import { tabTextOf } from "./strip-model";

const MONO = "font-mono text-muted-foreground text-xs font-normal tabular-nums";

/** What follows "Run 14": the live clock, how long a finished run took, or the status word (Inter, not mono). */
export function TabText({
	run,
	now,
}: Readonly<{ run: RunEntry; now: number }>) {
	const { t } = useTranslation("interfaces");
	const text = tabTextOf(run, now);
	if (text.kind === "word")
		return (
			<span className="text-muted-foreground text-xs">
				{statusWord(t, run.status)}
			</span>
		);
	return (
		<span
			className={cx(MONO, text.kind === "took" && "[word-spacing:-0.25em]")}
		>
			{text.kind === "clock" ? formatClock(text.ms) : formatTook(text.ms)}
		</span>
	);
}

/**
 * The tab's accessible name: "Run 14, Done, 48 s". The status word comes with the icon (never colour alone) and a
 * ticking clock stays out of it, so the name does not change every second.
 */
function tabName(t: InterfacesT, run: RunEntry, now: number) {
	const text = tabTextOf(run, now);
	return [
		t("interfaces:workbench.stage.tab.run", "Run {{n}}", { n: run.n }),
		statusWord(t, run.status),
		text.kind === "took" ? formatTook(text.ms) : null,
	]
		.filter((part) => part !== null)
		.join(", ");
}

export interface RunTabProps {
	readonly run: RunEntry;
	readonly uid: string;
	readonly now: number;
	readonly selected: boolean;
	readonly tabbable: boolean;
	readonly pinned: boolean;
	readonly showPin: boolean;
	/** The strip's row of tabs (desktop, a 2 px top bar on the selected one) or the phone's chips. */
	readonly chip: boolean;
	readonly touch: boolean;
	readonly onSelect: (run: RunEntry) => void;
	readonly onPin?: (run: RunEntry) => void;
	readonly onKeyDown: (event: KeyboardEvent<HTMLButtonElement>) => void;
}

function PinButton({
	run,
	pinned,
	touch,
	onPin,
}: Readonly<{
	run: RunEntry;
	pinned: boolean;
	touch: boolean;
	onPin: (run: RunEntry) => void;
}>) {
	const { t } = useTranslation("interfaces");
	const label = pinned
		? t("workbench.stage.tab.unpin", "Unpin run {{n}}", { n: run.n })
		: t("workbench.stage.tab.pin", "Pin run {{n}} to compare", { n: run.n });
	return (
		<button
			type="button"
			aria-label={label}
			title={label}
			aria-pressed={pinned}
			onClick={() => onPin(run)}
			className={cx(
				"mr-2 mb-0.5 inline-flex size-6 shrink-0 items-center justify-center rounded-md border outline-ring focus-visible:outline-2 focus-visible:outline-offset-1",
				touch && "relative after:absolute after:-inset-2.5 after:content-['']",
				pinned
					? "border-foreground bg-foreground text-background"
					: "border-transparent text-muted-foreground hover:bg-row-hover",
			)}
		>
			<Pin aria-hidden className="size-3.25" />
		</button>
	);
}

function TabButton(props: Readonly<RunTabProps>) {
	const { run, uid, now, selected, tabbable, showPin, chip, touch } = props;
	const { t } = useTranslation("interfaces");
	return (
		<button
			type="button"
			role="tab"
			id={tabIdOf(uid, run.id)}
			aria-label={tabName(t, run, now)}
			aria-selected={selected}
			aria-controls={selected ? paneIdOf(uid, run.id) : undefined}
			tabIndex={tabbable ? 0 : -1}
			{...{ [FOCUS_ATTR]: `${FOCUS_VALUE.tabPrefix}${run.id}` }}
			onClick={() => props.onSelect(run)}
			onKeyDown={props.onKeyDown}
			className={cx(
				"inline-flex items-center whitespace-nowrap outline-ring focus-visible:outline-2",
				chip
					? cx(
							"shrink-0 gap-1.75 rounded-full border pr-3.5 pl-3 text-sm",
							touch ? "h-11" : "h-9",
							selected
								? "border-border-strong bg-row-selected font-semibold"
								: "border-border bg-card font-medium text-ink-2",
						)
					: cx(
							"h-full gap-1.75 pb-0.5 pl-3 text-[13px] focus-visible:-outline-offset-2 hover:text-foreground",
							showPin ? "pr-2" : "pr-3",
							selected
								? "font-semibold text-foreground"
								: "font-medium text-ink-2",
						),
			)}
		>
			<StatusIcon
				status={run.status}
				className={chip ? "size-3.75" : "size-3.5"}
			/>
			<span>{t("workbench.stage.tab.run", "Run {{n}}", { n: run.n })}</span>
			<TabText run={run} now={now} />
		</button>
	);
}

/** One run's tab in the strip: icon, "Run 14", clock or time, and the pin on the selected and the pinned tab. */
export function RunTab(props: Readonly<RunTabProps>) {
	const { run, selected, pinned, showPin, chip, touch, onPin } = props;
	if (chip) return <TabButton {...props} />;
	return (
		<div
			role="presentation"
			className={cx(
				"-mb-px flex shrink-0 items-center border-x border-t-2",
				selected
					? "border-x-hairline border-t-foreground bg-background"
					: "border-x-transparent border-t-transparent",
			)}
		>
			<TabButton {...props} />
			{showPin && onPin ? (
				<PinButton run={run} pinned={pinned} touch={touch} onPin={onPin} />
			) : null}
		</div>
	);
}
