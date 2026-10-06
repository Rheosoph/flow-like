"use client";

import { useTranslation } from "@flow-like/locales";
import { useId, useMemo } from "react";
import {
	type FormModel,
	LAYOUT,
	type RunEntry,
	type StageProps,
	type WorkbenchLayout,
} from "../contracts";
import { shortWordsOf } from "../model/date-text";
import { formatTypedNumber } from "../run/format";
import { stageRunOf } from "../run/run-view";
import { CompareHeader, ComparePlaceholder } from "./compare";
import { type CompareView, compareViewOf } from "./compare-model";
import { type EmptyHint, EmptyStage } from "./empty-stage";
import { type PaneEnv, padOf } from "./pane-env";
import { type PaneContext, paneViewOf } from "./pane-model";
import { RunPane } from "./run-pane";
import { RunChips, RunStrip } from "./run-strip";
import { secretCheckOf } from "./stage-text";
import { stripTabsOf, ticksClock } from "./strip-model";
import { useNow } from "./use-now";

function emptyHintOf(runs: readonly RunEntry[]): EmptyHint {
	if (runs.some((run) => run.status === "sending")) return "sending";
	return runs.length > 0 ? "queued" : "press";
}

/** The stage's width: the box, less the rail while one is drawn beside it. */
const stageWidthOf = (layout: WorkbenchLayout, hasRail: boolean) =>
	layout.width - (hasRail && layout.split ? LAYOUT.railWidth : 0);

interface PanesProps {
	readonly run: RunEntry;
	readonly compare: CompareView;
	readonly context: PaneContext;
	readonly env: PaneEnv;
	readonly form: FormModel;
	readonly overlay: StageProps["state"]["view"]["overlay"];
}

/** One pane, or two in Compare: the pinned run on the left, the run on the stage (or a hint while they are the same) on the right. */
function Panes({
	run,
	compare,
	context,
	env,
	form,
	overlay,
}: Readonly<PanesProps>) {
	const comparing = compare.mode === "panes";
	const pinned = comparing ? compare.pinned : null;
	const pane = (
		item: RunEntry,
		flags: { selected: boolean; pinned: boolean; edge: boolean },
	) => (
		<RunPane
			key={item.id}
			pane={paneViewOf(item, context)}
			env={env}
			overlay={overlay}
			form={form}
			comparing={comparing}
			{...flags}
		/>
	);
	return (
		<div className="flex min-h-0 flex-1">
			{pinned
				? pane(pinned, {
						selected: compare.samePinned,
						pinned: true,
						edge: false,
					})
				: null}
			{compare.samePinned && comparing ? (
				<ComparePlaceholder pinned={run.n} />
			) : (
				pane(run, { selected: true, pinned: false, edge: comparing })
			)}
		</div>
	);
}

/**
 * Run strip, run bar, sections and Compare: the right of the workbench (and the phone's Output pane). Every
 * run is a tab; the run on the stage shows its bar, change chips, section links and body. A pinned run sits
 * beside it from a 600 px stage. The polite live region that announces run ends is the shell's.
 */
export function Stage(props: Readonly<StageProps>) {
	const { state, actions, layout, routes } = props;
	const { t } = useTranslation("interfaces");
	const uid = useId();
	const { viewer } = state.form;
	const hasFields = state.form.fields.length > 0;
	const now = useNow(state.runs.some(ticksClock));
	const stageRun = stageRunOf(state);

	const chipWords = useMemo(
		() => shortWordsOf(t, viewer.locale),
		[t, viewer.locale],
	);
	const tableWords = useMemo(
		() => ({
			...chipWords,
			none: t("workbench.stage.words.none", "None"),
			empty: t("workbench.stage.words.empty", "Empty"),
		}),
		[t, chipWords],
	);
	const isSecret = useMemo(
		() => secretCheckOf(state.memory.prefs.noSave),
		[state.memory.prefs.noSave],
	);
	const number = useMemo(
		() => (text: string) =>
			formatTypedNumber(text, viewer.locale, viewer.decimalSign),
		[viewer.locale, viewer.decimalSign],
	);
	const resultFormat = useMemo(
		() => ({
			locale: viewer.locale,
			date: chipWords.date,
			words: {
				yes: t("workbench.stage.words.yes", "Yes"),
				no: t("workbench.stage.words.no", "No"),
				none: t("workbench.stage.words.none", "None"),
				emptyText: t("workbench.stage.words.emptyText", "Empty text"),
			},
		}),
		[t, viewer.locale, chipWords],
	);

	const compare = compareViewOf({
		state,
		layout,
		stageRun,
		words: tableWords,
		isSecret,
		number,
	});
	const comparing = compare.mode === "panes";
	const env = useMemo<PaneEnv>(
		() => ({
			actions,
			layout,
			routes,
			uid,
			mac: viewer.mac,
			decimalSign: viewer.decimalSign,
			pad: padOf(layout, comparing),
			touch: layout.touch,
			resultFormat,
		}),
		[
			actions,
			layout,
			routes,
			uid,
			viewer.mac,
			viewer.decimalSign,
			comparing,
			resultFormat,
		],
	);

	if (state.runs.length === 0)
		return hasFields ? (
			<div
				data-fw-stage=""
				className="flex h-full min-h-0 min-w-0 flex-1 flex-col"
			>
				<EmptyStage form={state.form} narrow={!layout.split} hint="press" />
			</div>
		) : null;

	const canPin = layout.compare && state.runs.length > 1;
	const selectedId = stageRun?.id ?? null;
	const pinnedId = compare.pinned?.id ?? state.view.pinnedRunId;
	const stageWidth = stageWidthOf(layout, hasFields);
	const context: PaneContext = {
		state,
		layout,
		now,
		words: chipWords,
		isSecret,
		number,
		comparing,
		canPin,
	};

	return (
		<div
			data-fw-stage=""
			className="flex h-full min-h-0 min-w-0 flex-1 flex-col bg-background"
		>
			{layout.split ? (
				<RunStrip
					view={props}
					env={env}
					tabs={stripTabsOf({
						runs: state.runs,
						selectedId,
						pinnedId,
						stageWidth,
						withRunButton: !hasFields,
					})}
					now={now}
					selectedId={selectedId}
					pinnedId={pinnedId}
					canPin={canPin}
					zeroFields={!hasFields}
					locale={viewer.locale}
					words={chipWords}
				/>
			) : (
				<RunChips
					runs={state.runs}
					env={env}
					now={now}
					selectedId={selectedId}
				/>
			)}
			{compare.mode !== "off" ? (
				<CompareHeader
					view={compare}
					words={tableWords}
					env={env}
					stageWidth={stageWidth}
				/>
			) : null}
			{stageRun ? (
				<Panes
					run={stageRun}
					compare={compare}
					context={context}
					env={env}
					form={state.form}
					overlay={state.view.overlay}
				/>
			) : (
				<EmptyStage
					form={state.form}
					narrow={!layout.split}
					hint={emptyHintOf(state.runs)}
				/>
			)}
		</div>
	);
}
