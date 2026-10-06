"use client";

import { useTranslation } from "@flow-like/locales";
import { ChartLine, RefreshCw } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type {
	ModelStats,
	ModelsOverview,
} from "../../../../../lib/device-management/models";
import type { DeviceFailure } from "../../../../../lib/device-management/workspace/errors";
import { errorCopy } from "../../copy/error-copy";
import type { ObserveTarget } from "../../observe/use-observe-target";
import { useAreaTime } from "../../primitives/area-context";
import { Block } from "../../primitives/block";
import { DvButton } from "../../primitives/dv-button";
import { DvSelect } from "../../primitives/form-fields";
import { FreshnessStamp } from "../../primitives/freshness-stamp";
import { Segmented } from "../../primitives/segmented";
import { StateView } from "../../primitives/state-view";
import { stampOf } from "../../shell/attention-popover";
import { useAttentionState } from "../../workspace";
import type { ConsumerContext } from "../models-view";
import { PlaygroundButton } from "../playground/playground-sheet";
import { type ModelsRead, useModelsStats } from "../use-models";
import { ConsumerTable } from "./consumer-table";
import { StatsCharts } from "./stats-charts";
import { emptyText, rangeLabel, statsFormats } from "./stats-copy";
import {
	RANGE_SPEC,
	STATS_RANGES,
	type StatsRange,
	anyRequests,
	consumerRows,
	rangeWindow,
	statsView,
} from "./stats-view";

/*
 * Usage and performance (plan §3.7 item 4): one range and one model scope in
 * a row above everything they scope, the charts, then who called. A new
 * range keeps the last answer on screen, dimmed, until its own arrives.
 */

const ALL = "*";

/** The picked model while the overview still lists it; all models otherwise. */
function hostedModel(overview: ModelsOverview, id: string) {
	for (const model of overview.models) if (model.id === id) return model;
	return undefined;
}

/** Grant id → account from the device's verified access rules. */
function useConsumerContext(target: ObserveTarget): ConsumerContext {
	const grants = target.policy?.grants;
	const myGrantId = target.view?.keys.grantId;
	return useMemo(() => {
		const users = new Map<string, string>();
		for (const grant of grants ?? []) users.set(grant.grant_id, grant.user_id);
		return {
			owner: target.owner,
			...(myGrantId ? { myGrantId } : {}),
			grantUser: (grantId: string) => users.get(grantId),
		};
	}, [grants, myGrantId, target.owner]);
}

/**
 * The read's answer, or while a new query loads the previous answer marked
 * busy. A previous answer without requests isn't kept: its "No requests…"
 * sentence would name the new range and model.
 */
function useShown(read: ModelsRead<ModelStats>) {
	const [kept, setKept] = useState<ModelStats>();
	useEffect(() => {
		if (read.data) setKept(read.data);
	}, [read.data]);
	if (read.data) return { stats: read.data, busy: false };
	return read.loading && kept && anyRequests(kept)
		? { stats: kept, busy: true }
		: { stats: undefined, busy: false };
}

function StatsToolbar({
	range,
	onRange,
	modelId,
	onModel,
	overview,
}: Readonly<{
	range: StatsRange;
	onRange: (range: StatsRange) => void;
	modelId: string;
	onModel: (modelId: string) => void;
	overview: ModelsOverview;
}>) {
	const { t } = useTranslation("devices");
	const ranges = STATS_RANGES.map((value) => ({
		value,
		label: rangeLabel(t, value),
	}));
	const models = [
		{ value: ALL, label: t("devices:models.stats.allModels", "All models") },
		...overview.models.map((model) => ({
			value: model.id,
			label: model.display_name,
		})),
	];
	return (
		<>
			<Segmented
				label={t("devices:models.stats.rangeLabel", "Time range")}
				options={ranges}
				value={range}
				onChange={onRange}
				size="sm"
			/>
			<DvSelect
				aria-label={t("devices:models.stats.modelLabel", "Model")}
				value={modelId}
				onValueChange={onModel}
				options={models}
				size="sm"
				className="max-w-65"
			/>
		</>
	);
}

/** Why the read failed and, unless the answer is too large to ever arrive, a retry. */
function StatsReadFailed({
	device,
	failure,
	onRetry,
}: Readonly<{
	device: string;
	failure: DeviceFailure;
	onRetry: () => Promise<void>;
}>) {
	const { t } = useTranslation("devices");
	const [busy, setBusy] = useState(false);
	const reason = failure.rejection?.error;
	const title = t(
		"devices:models.stats.failedTitle",
		"{{device}} didn't return its usage statistics",
		{ device },
	);
	if (failure.code === "rejected_limit")
		return (
			<StateView
				kind="error"
				title={title}
				text={t(
					"devices:models.stats.tooLarge",
					"They are larger than {{device}} can send over this connection. Pick a shorter range or a single model.",
					{ device },
				)}
			/>
		);
	return (
		<StateView
			kind="error"
			title={title}
			text={
				<>
					{errorCopy(t, failure.code, { device })}
					{reason ? ` “${reason}”` : null}
				</>
			}
			actions={
				<DvButton
					size="sm"
					icon={RefreshCw}
					busy={busy}
					onClick={() => {
						setBusy(true);
						void onRetry().finally(() => setBusy(false));
					}}
				>
					{t("devices:models.stats.retry", "Try again")}
				</DvButton>
			}
		/>
	);
}

interface ContentProps {
	stats: ModelStats;
	busy: boolean;
	range: StatsRange;
	model?: string;
	context: ConsumerContext;
}

function StatsContent({
	stats,
	busy,
	range,
	model,
	context,
}: Readonly<ContentProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { input } = useAttentionState();
	const view = useMemo(() => statsView(stats), [stats]);
	const formats = useMemo(() => statsFormats(t, time.locale), [t, time.locale]);
	const rows = useMemo(
		() => consumerRows(stats.consumers, context),
		[stats.consumers, context],
	);
	if (!view.totals.requests)
		return <StateView kind="empty" title={emptyText(t, range, model)} />;
	return (
		<>
			<StatsCharts view={view} partialLast={view.to > input.now} busy={busy} />
			{rows.length ? (
				<section
					data-consumers=""
					className="flex min-w-0 flex-col gap-2 border-t border-hairline pt-3"
				>
					<h3 className="text-xs font-medium text-ink-2">
						{t("devices:models.stats.consumers.title", "Who called")}
					</h3>
					<ConsumerTable rows={rows} formats={formats} />
				</section>
			) : null}
		</>
	);
}

interface BodyProps extends Omit<ContentProps, "stats" | "busy"> {
	device: string;
	read: ModelsRead<ModelStats>;
}

/** The answer when there is one, else why there isn't. */
function StatsBody({ read, device, ...rest }: Readonly<BodyProps>) {
	const { t } = useTranslation("devices");
	const shown = useShown(read);
	if (shown.stats)
		return <StatsContent stats={shown.stats} busy={shown.busy} {...rest} />;
	if (read.failure)
		return (
			<StatsReadFailed
				device={device}
				failure={read.failure}
				onRetry={read.refetch}
			/>
		);
	if (read.loading)
		return (
			<StateView
				kind="loading"
				rows={4}
				title={t(
					"devices:models.stats.reading",
					"Reading usage statistics from {{device}}…",
					{ device },
				)}
			/>
		);
	return (
		<StateView
			kind="notloaded"
			title={t(
				"devices:models.stats.notLoaded",
				"Connect live to read the usage statistics of {{device}}.",
				{ device },
			)}
		/>
	);
}

export interface ModelStatsBlockProps {
	target: ObserveTarget;
	overview: ModelsOverview;
}

/**
 * Tokens, requests, first-token time, speed, queue wait and callers of one
 * model or all, over 24 h, 7 d or 90 d; the playground opens from its head.
 */
export function ModelStatsBlock({
	target,
	overview,
}: Readonly<ModelStatsBlockProps>) {
	const { t } = useTranslation("devices");
	const { input } = useAttentionState();
	const [range, setRange] = useState<StatsRange>("day");
	const [picked, setPicked] = useState(ALL);
	const model = hostedModel(overview, picked);
	const modelId = model ? model.id : ALL;
	const read = useModelsStats(
		target.deviceId,
		overview.models.length ? rangeWindow(range, input.now, model?.id) : null,
		{ pollS: RANGE_SPEC[range].pollS },
	);
	const context = useConsumerContext(target);
	if (!overview.models.length) return null;
	return (
		<Block
			id="models-stats"
			icon={ChartLine}
			title={t("devices:models.stats.title", "Usage and performance")}
			stamp={<FreshnessStamp {...stampOf(read.freshness)} />}
			tools={
				<PlaygroundButton
					deviceId={target.deviceId}
					device={target.name}
					models={overview.models}
					{...(model ? { modelId: model.id } : {})}
				/>
			}
			toolbar={
				<StatsToolbar
					range={range}
					onRange={setRange}
					modelId={modelId}
					onModel={setPicked}
					overview={overview}
				/>
			}
		>
			<StatsBody
				read={read}
				device={target.name}
				range={range}
				{...(model ? { model: model.display_name } : {})}
				context={context}
			/>
		</Block>
	);
}
