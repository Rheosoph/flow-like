"use client";

import { useTranslation } from "@flow-like/locales";
import type { TFunction } from "i18next";
import { type KeyboardEvent, type ReactNode, useId, useMemo } from "react";
import { TONE_TEXT, cx } from "../../../settings/devices/primitives/tone";
import { RUN_STATUS_TONE, type RunEntry, type RunStatus } from "../contracts";
import { type FormWords, shortWordsOf } from "../model/date-text";
import { linePlaces } from "../model/queue";
import { formatTypedNumber } from "../run/format";
import { stageRunOf } from "../run/run-view";
import { RUN_STATUS_ICON, RUN_STATUS_SPINS, statusWord } from "../status-look";
import {
	type RailPartProps,
	type RunChanges,
	changeText,
	dayHeading,
	groupRunsByDay,
	previewOf,
	secretCheck,
	waitOf,
} from "./rail-model";
import {
	durationText,
	hasLiveClock,
	keepIds,
	previewText,
	runChangesOf,
	timeOf,
	waitText,
} from "./run-text";
import { useNow } from "./use-now";
import { useScrollMemory } from "./use-scroll-memory";

type InterfacesT = TFunction<"interfaces">;

const UNWORDED: ReadonlySet<RunStatus> = new Set(["done", "empty"]);

/** The icon alone says "Done"; every other status has its word beside it. */
function StatusIcon({
	status,
	word,
}: Readonly<{ status: RunStatus; word: string }>) {
	const Icon = RUN_STATUS_ICON[status];
	const unworded = UNWORDED.has(status);
	return (
		<Icon
			role={unworded ? "img" : undefined}
			aria-label={unworded ? word : undefined}
			aria-hidden={unworded ? undefined : true}
			className={cx(
				"mt-px size-4 flex-none",
				TONE_TEXT[RUN_STATUS_TONE[status]],
				RUN_STATUS_SPINS[status] && "animate-spin motion-reduce:animate-none",
			)}
		/>
	);
}

function changeLine(t: InterfacesT, changes: RunChanges) {
	if (changes.kind === "first")
		return t("interfaces:workbench.rail.runs.firstRun", "First run");
	if (changes.kind === "same")
		return t(
			"interfaces:workbench.rail.runs.sameAs",
			"Same inputs as run {{n}}",
			{ n: changes.n },
		);
	return changeText(changes.items);
}

interface RowProps {
	readonly run: RunEntry;
	readonly changes: RunChanges;
	readonly selected: boolean;
	readonly place: number | undefined;
	readonly now: number;
	readonly locale: string;
	readonly onSelect: () => void;
}

function RunRow({
	run,
	changes,
	selected,
	place,
	now,
	locale,
	onSelect,
}: Readonly<RowProps>) {
	const { t } = useTranslation("interfaces");
	const live = hasLiveClock(run);
	const wait = waitOf(run, place);
	const tail = wait ? waitText(t, wait) : durationText(run, now);
	const when = [timeOf(run.createdAt, locale), tail]
		.filter((part) => part !== null && part !== "")
		.join(" · ");
	const preview = previewOf(run);
	const word = statusWord(t, run.status);
	const changed = changeLine(t, changes);
	return (
		<li>
			<button
				type="button"
				data-run-row=""
				data-run-id={run.id}
				aria-current={selected ? "true" : undefined}
				onClick={onSelect}
				className={cx(
					"grid w-full grid-cols-[16px_minmax(0,1fr)] gap-x-2.5 gap-y-0.5 px-5 py-2.25 text-left text-foreground hover:bg-row-hover focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring",
					selected && "bg-row-selected",
				)}
			>
				<StatusIcon status={run.status} word={word} />
				<span className="flex min-w-0 items-baseline gap-2">
					<span className="flex-none text-[13px]/[18px] font-semibold whitespace-nowrap">
						{t("workbench.rail.runs.runN", "Run {{n}}", { n: run.n })}
					</span>
					{UNWORDED.has(run.status) ? null : (
						<span className="flex-none text-xs/[18px] whitespace-nowrap text-ink-2">
							{word}
						</span>
					)}
					<span className="min-w-0 flex-1" />
					<span
						title={when}
						aria-hidden={live || undefined}
						className="min-w-0 truncate text-right font-mono text-xs/[18px] text-muted-foreground tabular-nums [word-spacing:-0.25em]"
					>
						{when}
					</span>
				</span>
				<span
					title={changed}
					className="col-start-2 line-clamp-2 text-xs/[17px] text-ink-2 wrap-anywhere"
				>
					{keepIds(changed)}
				</span>
				{preview ? (
					<span className="col-start-2 line-clamp-2 text-xs/[17px] text-muted-foreground wrap-anywhere">
						{keepIds(previewText(t, preview, run.origin))}
					</span>
				) : null}
			</button>
		</li>
	);
}

/** Where ↑, ↓, Home and End land, given the focused row and the last row. */
const ROW_KEYS: Readonly<Record<string, (at: number, last: number) => number>> =
	{
		ArrowDown: (at) => at + 1,
		ArrowUp: (at) => at - 1,
		Home: () => 0,
		End: (_at, last) => last,
	};

/** ↑/↓, Home and End move between the rows; Tab still visits each. */
function onListKey(event: KeyboardEvent<HTMLElement>) {
	const landing = ROW_KEYS[event.key];
	if (!landing || event.altKey || event.ctrlKey || event.metaKey) return;
	const rows = Array.from(
		event.currentTarget.querySelectorAll<HTMLElement>("[data-run-row]"),
	);
	const at = rows.findIndex((row) => row === event.target);
	if (at < 0) return;
	event.preventDefault();
	const last = rows.length - 1;
	rows[Math.max(0, Math.min(last, landing(at, last)))]?.focus();
}

/** What each run's inputs say against the run pressed before it, once per runs list. */
function useRunChanges(state: RailPartProps["state"], words: FormWords) {
	const { fields, viewer } = state.form;
	const { locale, decimalSign } = viewer;
	const noSave = state.memory.prefs.noSave;
	const runs = state.runs;
	return useMemo(() => {
		const context = { fields, words, isSecret: secretCheck(noSave) };
		const number = (text: string) =>
			formatTypedNumber(text, locale, decimalSign);
		return new Map(
			runs.map((run, index) => [
				run.id,
				runChangesOf(run, runs[index + 1], context, number),
			]),
		);
	}, [fields, noSave, runs, words, locale, decimalSign]);
}

function DayGroup({
	heading,
	children,
}: Readonly<{ heading: string; children: ReactNode }>) {
	const id = useId();
	return (
		<section aria-labelledby={id}>
			<h3
				id={id}
				className="m-0 px-5 pt-3 pb-1 text-label/4 font-semibold tracking-[0.06em] text-muted-foreground uppercase"
			>
				{heading}
			</h3>
			<ul className="m-0 list-none p-0">{children}</ul>
		</section>
	);
}

/**
 * The Runs tab: "Runs on this device", grouped by day, queued and sending runs at the top of
 * "Today". A row picks the run for the stage; the clock of a live run ticks once a second.
 */
export function RunsList({ state, actions }: Readonly<RailPartProps>) {
	const { t } = useTranslation("interfaces");
	const titleId = useId();
	const { locale } = state.form.viewer;
	const words = useMemo(() => shortWordsOf(t, locale), [t, locale]);
	const scroll = useScrollMemory(
		`${state.form.appId}\u0001${state.form.eventId}\u0001runs`,
	);
	const now = useNow(state.runs.some(hasLiveClock));
	const places = useMemo(() => linePlaces(state.runs), [state.runs]);
	const changes = useRunChanges(state, words);
	const groups = groupRunsByDay(state.runs, now);
	const selectedId = stageRunOf(state)?.id ?? null;
	const title =
		state.form.host.persistence === "device"
			? t("workbench.rail.runs.title", "Runs on this device")
			: t("workbench.rail.runs.titleSession", "Runs in this session");
	return (
		<section
			ref={scroll.ref}
			aria-labelledby={titleId}
			onScroll={scroll.onScroll}
			onKeyDown={onListKey}
			className="min-h-0 flex-1 overflow-y-auto border-t border-hairline [scrollbar-width:thin]"
		>
			<p
				id={titleId}
				className="m-0 px-5 pt-3 pb-0.5 text-xs/4 text-muted-foreground"
			>
				{title}
			</p>
			{groups.map((group) => (
				<DayGroup
					key={group.day}
					heading={dayHeading(group.at, now, locale, words)}
				>
					{group.runs.map((run) => (
						<RunRow
							key={run.id}
							run={run}
							changes={changes.get(run.id) ?? { kind: "first" }}
							selected={run.id === selectedId}
							place={places[run.id]}
							now={now}
							locale={locale}
							onSelect={() => actions.selectRun(run.id, "list")}
						/>
					))}
				</DayGroup>
			))}
		</section>
	);
}
