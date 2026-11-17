"use client";

import { useTranslation } from "@flow-like/locales";
import type { TFunction } from "i18next";
import CopyIcon from "lucide-react/dist/esm/icons/copy.js";
import DatabaseIcon from "lucide-react/dist/esm/icons/database.js";
import FoldVerticalIcon from "lucide-react/dist/esm/icons/fold-vertical.js";
import InfoIcon from "lucide-react/dist/esm/icons/info.js";
import LaptopIcon from "lucide-react/dist/esm/icons/laptop.js";
import XIcon from "lucide-react/dist/esm/icons/x.js";
import {
	type ComponentType,
	type ReactNode,
	createContext,
	useCallback,
	useContext,
	useMemo,
} from "react";
import { toast } from "sonner";
import { useModelIdentities } from "../../../hooks/use-model-names";
import {
	type IModelIdentity,
	describeModel,
} from "../../../lib/bit/model-display-name";
import type { IChatUsageStat } from "../../../lib/llm/usage";
import { cn } from "../../../lib/utils";
import { Button } from "../../ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../../ui/tabs";
import {
	type UsageBreakdown,
	type UsageCallRow,
	type UsageModelRow,
	type UsagePurpose,
	type UsageStepRow,
	formatCost,
	formatDuration,
	formatTokens,
} from "./usage-breakdown";
import type { UsageEfficiency } from "./usage-efficiency";
import { UsageEfficiencyPanel } from "./usage-efficiency-panel";

type T = TFunction<"chat">;

const MODEL_SWATCHES = [
	"bg-teal-500",
	"bg-violet-500",
	"bg-amber-400",
	"bg-rose-400",
	"bg-lime-500",
	"bg-fuchsia-400",
] as const;
const OTHER_SWATCH = "bg-muted-foreground/60";

const CACHED_FILL = "bg-sky-300 dark:bg-sky-800";
const INPUT_FILL = "bg-sky-600 dark:bg-sky-400";
const OUTPUT_FILL = "bg-primary";

const ROW_GRID =
	"grid-cols-[minmax(0,1fr)_3.25rem_4.25rem] sm:grid-cols-[minmax(0,1fr)_4.5rem_3.25rem_4.25rem]";
/** Clears the global prose table styling (cell borders, padding, fills). */
const CELL = "border-0 bg-transparent px-0";

interface ModelInfo {
	identity: IModelIdentity;
	swatch: string;
}

const ModelInfoContext = createContext<ReadonlyMap<string, ModelInfo>>(
	new Map(),
);

function useModelInfo(model: string): ModelInfo {
	return (
		useContext(ModelInfoContext).get(model) ?? {
			identity: describeModel(model),
			swatch: OTHER_SWATCH,
		}
	);
}

const percent = (value: number) => `${Math.round(value * 100)}%`;

function modelTitle(identity: IModelIdentity, t: T): string {
	if (identity.missing) return t("usageReceipt.unknownModel", "Unknown model");
	if (identity.opaque) return t("usageReceipt.unlistedModel", "Unlisted model");
	return identity.name;
}

/** Where the model ran and, unless the exact id is shown, which pinned version answered. */
function modelByline(
	identity: IModelIdentity,
	t: T,
	withSnapshot: boolean,
): string[] {
	const parts: string[] = [];
	if (identity.onDevice)
		parts.push(t("usageReceipt.onDevice", "On this device"));
	else if (identity.provider) parts.push(identity.provider);
	if (withSnapshot && identity.snapshot)
		parts.push(
			t("usageReceipt.snapshot", "snapshot {{date}}", {
				date: identity.snapshot,
			}),
		);
	return parts;
}

function costLabel(
	cost: number | null,
	identity: IModelIdentity | null,
	t: T,
): string {
	if (cost != null) return formatCost(cost);
	return identity?.onDevice ? t("usageReceipt.localCost", "local") : "—";
}

function purposeLabel(purpose: UsagePurpose, t: T): string {
	if (purpose === "context_summary")
		return t("usageEfficiency.checkpoint", "Context summary");
	if (purpose === "memory")
		return t("usageEfficiency.memoryWork", "Memory work");
	return t("usageEfficiency.visionWork", "Image analysis");
}

function listFormat(items: string[], language: string | undefined): string {
	try {
		return new Intl.ListFormat(language, {
			style: "long",
			type: "conjunction",
		}).format(items);
	} catch {
		return items.join(", ");
	}
}

function Swatch({ className }: Readonly<{ className: string }>) {
	return (
		<span
			aria-hidden="true"
			className={cn("mt-1.5 size-2 shrink-0 rounded-full", className)}
		/>
	);
}

function TokenMix({
	breakdown,
	efficiency,
}: Readonly<{ breakdown: UsageBreakdown; efficiency: UsageEfficiency }>) {
	const { t } = useTranslation("chat");
	const cached = Math.min(
		efficiency.cacheReadTokens ?? 0,
		breakdown.promptTokens,
	);
	const other = breakdown.promptTokens - cached;
	const output = breakdown.completionTokens;
	if (cached + other + output === 0) return null;
	const segments = [
		{
			key: "cached",
			value: cached,
			fill: CACHED_FILL,
			label: t("usageEfficiency.cachedInput", "Cached input"),
		},
		{
			key: "input",
			value: other,
			fill: INPUT_FILL,
			label:
				efficiency.cacheReadTokens === null
					? t("input", "Input")
					: t("usageReceipt.otherInput", "Other input"),
		},
		{
			key: "output",
			value: output,
			fill: OUTPUT_FILL,
			label: t("output", "Output"),
		},
	].filter((segment) => segment.value > 0);
	return (
		<div className="space-y-1.5">
			<div
				role="img"
				aria-label={segments
					.map((segment) => `${segment.label} ${formatTokens(segment.value)}`)
					.join(", ")}
				className="flex h-2.5 gap-0.5 overflow-hidden rounded-full"
			>
				{segments.map((segment) => (
					<span
						key={segment.key}
						className={cn("block h-full min-w-1", segment.fill)}
						style={{ flex: `${segment.value} 1 0px` }}
					/>
				))}
			</div>
			<div
				aria-hidden="true"
				className="flex flex-wrap gap-x-4 gap-y-0.5 text-xs text-muted-foreground tabular-nums"
			>
				{segments.map((segment) => (
					<span key={segment.key} className="inline-flex items-center gap-1.5">
						<span className={cn("size-2 rounded-xs", segment.fill)} />
						{segment.label}
						<span className="font-semibold text-foreground">
							{formatTokens(segment.value)}
						</span>
					</span>
				))}
			</div>
		</div>
	);
}

function ShareBar({
	share,
	swatch,
	className,
}: Readonly<{ share: number; swatch: string; className: string }>) {
	return (
		<div
			aria-hidden="true"
			className={cn("overflow-hidden rounded-full bg-muted", className)}
		>
			<div
				className={cn("h-full rounded-full", swatch)}
				style={{ width: `${Math.max(2, share * 100)}%` }}
			/>
		</div>
	);
}

function ModelRow({ row }: Readonly<{ row: UsageModelRow }>) {
	const { t } = useTranslation("chat");
	const { identity, swatch } = useModelInfo(row.model);
	const title = modelTitle(identity, t);
	const showRaw = identity.known || identity.opaque;
	const byline = [
		...modelByline(identity, t, !showRaw),
		t("countCalls", "{{count}} call", { count: row.calls }),
	].join(" · ");
	return (
		<li
			className={cn(
				"grid items-center gap-2.5 rounded-md px-2 py-2 tabular-nums hover:bg-muted/60",
				ROW_GRID,
			)}
		>
			<div className="flex min-w-0 gap-2.5">
				<Swatch className={swatch} />
				<div className="min-w-0 flex-1">
					<p className="truncate text-sm leading-5 font-medium">{title}</p>
					<p className="truncate text-xs leading-4 text-muted-foreground">
						{byline}
					</p>
					{showRaw && (
						<p
							className="mt-0.5 truncate font-mono text-[11px] leading-4 text-muted-foreground/80"
							title={identity.raw}
						>
							{identity.raw}
						</p>
					)}
					<ShareBar
						share={row.share}
						swatch={swatch}
						className="mt-1.5 h-1 sm:hidden"
					/>
				</div>
			</div>
			<ShareBar
				share={row.share}
				swatch={swatch}
				className="hidden h-1.5 sm:block"
			/>
			<span className="text-right text-xs">
				{formatTokens(row.totalTokens)}
			</span>
			<span className="text-right text-xs">
				{costLabel(row.cost, identity, t)}
			</span>
		</li>
	);
}

interface UsageNote {
	key: string;
	icon: ComponentType<{ className?: string }>;
	text: string;
}

function useUsageNotes(
	breakdown: UsageBreakdown,
	efficiency: UsageEfficiency,
): UsageNote[] {
	const { t, i18n } = useTranslation("chat");
	const info = useContext(ModelInfoContext);
	const language = i18n?.language;
	return useMemo(() => {
		const notes: UsageNote[] = [];
		const name = (model: string) =>
			modelTitle(info.get(model)?.identity ?? describeModel(model), t);
		const summary = breakdown.summary;
		if (summary) {
			const compaction = efficiency.compactions.find(
				(event) => typeof event.after_tokens === "number",
			);
			const parts = [
				summary.cost != null && breakdown.cost
					? t(
							"usageReceipt.noteSummary",
							"Summarizing older turns took {{tokens}} tokens and {{cost}}, {{percent}} of this reply.",
							{
								tokens: formatTokens(summary.totalTokens),
								cost: formatCost(summary.cost),
								percent: percent(summary.cost / breakdown.cost),
							},
						)
					: t(
							"usageReceipt.noteSummaryTokens",
							"Summarizing older turns took {{tokens}} tokens.",
							{ tokens: formatTokens(summary.totalTokens) },
						),
			];
			if (compaction)
				parts.push(
					t(
						"usageReceipt.noteSummaryShrink",
						"The history went from {{before}} to {{after}} tokens.",
						{
							before: formatTokens(compaction.before_tokens),
							after: formatTokens(compaction.after_tokens ?? 0),
						},
					),
				);
			notes.push({
				key: "summary",
				icon: FoldVerticalIcon,
				text: parts.join(" "),
			});
		}
		if (efficiency.cacheShare !== null) {
			notes.push({
				key: "cache",
				icon: DatabaseIcon,
				text: efficiency.partialReads
					? t(
							"usageReceipt.noteCachePartial",
							"{{percent}} of the reported input came from the prompt cache.",
							{ percent: percent(efficiency.cacheShare) },
						)
					: t(
							"usageReceipt.noteCache",
							"{{percent}} of the input came from the prompt cache.",
							{ percent: percent(efficiency.cacheShare) },
						),
			});
			if (breakdown.cacheUnreported.models.length > 0)
				notes.push({
					key: "uncached",
					icon: InfoIcon,
					text: t(
						"usageReceipt.noteUncached",
						"{{models}} didn't report cache use, so their {{tokens}} input tokens are left out of that share.",
						{
							models: listFormat(
								breakdown.cacheUnreported.models.map(name),
								language,
							),
							tokens: formatTokens(breakdown.cacheUnreported.promptTokens),
						},
					),
				});
		} else if (breakdown.promptTokens > 0) {
			notes.push({
				key: "no-cache",
				icon: InfoIcon,
				text: t(
					"usageReceipt.noteNoCache",
					"No call reported cache use. The provider may still have reused cached input.",
				),
			});
		}
		const local = breakdown.models
			.filter((row) => info.get(row.model)?.identity.onDevice)
			.map((row) => name(row.model));
		if (local.length > 0)
			notes.push({
				key: "local",
				icon: LaptopIcon,
				text: t(
					"usageReceipt.noteOnDevice",
					"{{models}} ran on this device, so no cost applies.",
					{ models: listFormat(local, language) },
				),
			});
		return notes;
	}, [breakdown, efficiency, info, t, language]);
}

function OverviewTab({
	breakdown,
	efficiency,
}: Readonly<{ breakdown: UsageBreakdown; efficiency: UsageEfficiency }>) {
	const { t } = useTranslation("chat");
	const notes = useUsageNotes(breakdown, efficiency);
	return (
		<div className="space-y-5">
			{breakdown.models.length > 0 && (
				<section aria-label={t("usageReceipt.models", "Models")}>
					<div
						aria-hidden="true"
						className={cn(
							"grid gap-2.5 px-2 pb-1 text-[11px] text-muted-foreground",
							ROW_GRID,
						)}
					>
						<span>{t("usageReceipt.model", "Model")}</span>
						<span className="hidden sm:inline">
							{t("usageReceipt.share", "Share")}
						</span>
						<span className="text-right">
							{t("usageReceipt.tokensColumn", "Tokens")}
						</span>
						<span className="text-right">{t("usageReceipt.cost", "Cost")}</span>
					</div>
					<ul>
						{breakdown.models.map((row) => (
							<ModelRow key={row.model} row={row} />
						))}
					</ul>
				</section>
			)}
			{notes.length > 0 && (
				<section className="space-y-1">
					<h3 className="px-2 text-[11px] leading-4 font-normal tracking-normal text-muted-foreground">
						{t("usageReceipt.worthKnowing", "Worth knowing")}
					</h3>
					<ul className="space-y-0.5">
						{notes.map(({ key, icon: Icon, text }) => (
							<li
								key={key}
								className="flex gap-2.5 px-2 py-1.5 text-xs leading-relaxed"
							>
								<Icon
									aria-hidden="true"
									className="mt-0.5 size-3.5 shrink-0 text-muted-foreground"
								/>
								<span>{text}</span>
							</li>
						))}
					</ul>
				</section>
			)}
		</div>
	);
}

function CallRow({ call }: Readonly<{ call: UsageCallRow }>) {
	const { t } = useTranslation("chat");
	const { identity, swatch } = useModelInfo(call.model);
	const title = modelTitle(identity, t);
	const meta = call.cachedTokens
		? t(
				"usageReceipt.callMetaCached",
				"{{input}} in ({{cached}} cached) · {{output}} out",
				{
					input: formatTokens(call.promptTokens),
					cached: formatTokens(call.cachedTokens),
					output: formatTokens(call.completionTokens),
				},
			)
		: t("usageReceipt.callMeta", "{{input}} in · {{output}} out", {
				input: formatTokens(call.promptTokens),
				output: formatTokens(call.completionTokens),
			});
	return (
		<tr className="align-top hover:bg-muted/60">
			<td className={cn(CELL, "rounded-l-md py-1.5 pr-2 pl-2")}>
				<div className="flex min-w-0 gap-2.5">
					<Swatch className={swatch} />
					<div className="min-w-0">
						<p className="truncate text-[13px] leading-5" title={identity.raw}>
							{call.purpose ? (
								<>
									{purposeLabel(call.purpose, t)}
									<span className="text-muted-foreground"> · {title}</span>
								</>
							) : (
								title
							)}
						</p>
						<p className="truncate text-[11px] leading-4 text-muted-foreground">
							{meta}
						</p>
					</div>
				</div>
			</td>
			<td className={cn(CELL, "py-1.5 text-right leading-5")}>
				{formatTokens(call.totalTokens)}
			</td>
			<td className={cn(CELL, "py-1.5 text-right leading-5")}>
				{costLabel(call.cost, identity, t)}
			</td>
			<td
				className={cn(
					CELL,
					"rounded-r-md py-1.5 pr-2 text-right leading-5 text-muted-foreground",
				)}
			>
				{call.durationMs == null ? "—" : formatDuration(call.durationMs)}
			</td>
		</tr>
	);
}

function StepRows({
	step,
	spaced,
}: Readonly<{ step: UsageStepRow; spaced: boolean }>) {
	const { t } = useTranslation("chat");
	const detail =
		step.iterations != null && step.iterations > 1
			? t("iterationsIterations", "{{iterations}} iterations", {
					iterations: step.iterations,
				})
			: t("countCalls", "{{count}} call", {
					count: Math.max(step.calls.length, 1),
				});
	const cell = cn(CELL, "p-0 font-semibold", spaced && "pt-2");
	const band = "bg-muted/70 py-1.5";
	return (
		<tbody>
			<tr>
				<th scope="rowgroup" className={cn(cell, "text-left")}>
					<div className={cn(band, "truncate rounded-l-md pl-2")}>
						{step.name}
						<span className="font-normal text-muted-foreground">
							{" "}
							· {detail}
						</span>
					</div>
				</th>
				<td className={cell}>
					<div className={cn(band, "text-right")}>
						{formatTokens(step.totalTokens)}
					</div>
				</td>
				<td className={cell}>
					<div className={cn(band, "text-right")}>
						{step.cost == null ? "—" : formatCost(step.cost)}
					</div>
				</td>
				<td className={cell}>
					<div
						className={cn(
							band,
							"rounded-r-md pr-2 text-right font-normal text-muted-foreground",
						)}
					>
						{step.durationMs == null ? "—" : formatDuration(step.durationMs)}
					</div>
				</td>
			</tr>
			{step.calls.map((call, index) => (
				<CallRow key={`${call.model}-${index}`} call={call} />
			))}
		</tbody>
	);
}

function CallsTab({
	breakdown,
	efficiency,
}: Readonly<{ breakdown: UsageBreakdown; efficiency: UsageEfficiency }>) {
	const { t } = useTranslation("chat");
	return (
		<div className="space-y-3">
			<table className="my-0 w-full table-fixed border-separate border-spacing-0 text-xs tabular-nums">
				<colgroup>
					<col />
					<col className="w-14" />
					<col className="w-16" />
					<col className="w-13" />
				</colgroup>
				<thead>
					<tr className="text-[11px] text-muted-foreground">
						<th
							scope="col"
							className={cn(CELL, "pt-0 pb-1.5 pl-2 text-left font-normal")}
						>
							{t("usageReceipt.stepAndCall", "Step and call")}
						</th>
						<th
							scope="col"
							className={cn(CELL, "pt-0 pb-1.5 text-right font-normal")}
						>
							{t("usageReceipt.tokensColumn", "Tokens")}
						</th>
						<th
							scope="col"
							className={cn(CELL, "pt-0 pb-1.5 text-right font-normal")}
						>
							{t("usageReceipt.cost", "Cost")}
						</th>
						<th
							scope="col"
							className={cn(CELL, "pt-0 pr-2 pb-1.5 text-right font-normal")}
						>
							{t("usageReceipt.time", "Time")}
						</th>
					</tr>
				</thead>
				{breakdown.steps.map((step, index) => (
					<StepRows
						key={`${step.name}-${index}`}
						step={step}
						spaced={index > 0}
					/>
				))}
			</table>
			{efficiency.callsOmitted > 0 && (
				<p className="px-2 text-[11px] leading-4 text-muted-foreground">
					{t(
						"usageEfficiency.callsOmitted",
						"Showing recent calls. {{count}} earlier calls are included in totals.",
						{ count: efficiency.callsOmitted },
					)}
				</p>
			)}
		</div>
	);
}

function Headline({ breakdown }: Readonly<{ breakdown: UsageBreakdown }>) {
	const { t } = useTranslation("chat");
	const tokens = (
		<span>
			<b className="font-semibold">{formatTokens(breakdown.totalTokens)}</b>{" "}
			<span className="text-muted-foreground">
				{t("usageEfficiency.tokenUnit", "tokens")}
			</span>
		</span>
	);
	return (
		<div className="flex flex-wrap items-baseline gap-x-4 gap-y-1 text-sm tabular-nums">
			{breakdown.cost != null ? (
				<>
					<span className="text-3xl leading-none font-semibold tracking-tight">
						{formatCost(breakdown.cost)}
					</span>
					{tokens}
				</>
			) : (
				<span className="text-3xl leading-none font-semibold tracking-tight">
					{formatTokens(breakdown.totalTokens)}{" "}
					<span className="text-sm font-normal tracking-normal text-muted-foreground">
						{t("usageEfficiency.tokenUnit", "tokens")}
					</span>
				</span>
			)}
			{breakdown.durationMs != null && (
				<span>
					<b className="font-semibold">
						{formatDuration(breakdown.durationMs)}
					</b>{" "}
					<span className="text-muted-foreground">
						{t("usageReceipt.modelTime", "model time")}
					</span>
				</span>
			)}
		</div>
	);
}

export interface UsageReceiptProps {
	stats: readonly IChatUsageStat[];
	breakdown: UsageBreakdown;
	efficiency: UsageEfficiency;
	/** Renders a close button; leave out where the host already has one. */
	onClose?: () => void;
	/** Accessible title for hosts that need one, such as a dialog sheet. */
	title?: ReactNode;
	className?: string;
}

/** One reply's model usage: totals, models, calls, and context and cache detail. */
export function UsageReceipt({
	stats,
	breakdown,
	efficiency,
	onClose,
	title,
	className,
}: Readonly<UsageReceiptProps>) {
	const { t } = useTranslation("chat");
	const identities = useModelIdentities(
		useMemo(() => breakdown.models.map((row) => row.model), [breakdown.models]),
	);
	const info = useMemo(
		() =>
			new Map(
				breakdown.models.map((row, index) => [
					row.model,
					{
						identity: identities.get(row.model) ?? describeModel(row.model),
						swatch: MODEL_SWATCHES[index] ?? OTHER_SWATCH,
					},
				]),
			),
		[breakdown.models, identities],
	);
	const copy = useCallback(async () => {
		try {
			await navigator.clipboard.writeText(JSON.stringify(stats, null, 2));
			toast.success(t("usageReceipt.copied", "Usage copied"));
		} catch {
			toast.error(t("usageReceipt.copyFailed", "Couldn't copy the usage"));
		}
	}, [stats, t]);
	const scope = [
		t("usageReceipt.thisReply", "This reply"),
		t("countCalls", "{{count}} call", { count: breakdown.callCount }),
		t("usageReceipt.modelCount", "{{count}} models", {
			count: breakdown.models.length,
		}),
	].join(" · ");

	return (
		<ModelInfoContext.Provider value={info}>
			<div className={cn("flex min-h-0 flex-1 flex-col", className)}>
				{title}
				<header className="space-y-3 px-4 pt-2.5 pb-3">
					<div className={cn("flex items-center gap-1", !onClose && "pr-8")}>
						<p className="text-xs leading-5 text-muted-foreground">{scope}</p>
						<div className="ml-auto flex items-center">
							<Button
								variant="ghost"
								size="icon"
								className="size-8 text-muted-foreground"
								aria-label={t("usageReceipt.copy", "Copy usage as JSON")}
								onClick={copy}
							>
								<CopyIcon className="size-3.5" />
							</Button>
							{onClose && (
								<Button
									variant="ghost"
									size="icon"
									className="size-8 text-muted-foreground"
									aria-label={t("usageReceipt.close", "Close")}
									onClick={onClose}
								>
									<XIcon className="size-4" />
								</Button>
							)}
						</div>
					</div>
					<Headline breakdown={breakdown} />
					<TokenMix breakdown={breakdown} efficiency={efficiency} />
				</header>
				<Tabs defaultValue="overview" className="min-h-0 flex-1 gap-0">
					<TabsList className="mx-4 mb-2 w-[calc(100%-2rem)]">
						<TabsTrigger value="overview">
							{t("usageReceipt.overview", "Overview")}
						</TabsTrigger>
						<TabsTrigger value="calls">
							{t("usageReceipt.calls", "Calls")}
							<span className="text-muted-foreground tabular-nums">
								{breakdown.callCount}
							</span>
						</TabsTrigger>
						<TabsTrigger value="context">
							{t("usageReceipt.contextTab", "Context & cache")}
						</TabsTrigger>
					</TabsList>
					<TabsContent
						value="overview"
						className="min-h-0 overflow-y-auto px-2 pb-4"
					>
						<OverviewTab breakdown={breakdown} efficiency={efficiency} />
					</TabsContent>
					<TabsContent
						value="calls"
						className="min-h-0 overflow-y-auto px-2 pb-4"
					>
						<CallsTab breakdown={breakdown} efficiency={efficiency} />
					</TabsContent>
					<TabsContent
						value="context"
						className="min-h-0 overflow-y-auto px-4 pb-4 [&_p]:leading-5"
					>
						<UsageEfficiencyPanel data={efficiency} />
					</TabsContent>
				</Tabs>
			</div>
		</ModelInfoContext.Provider>
	);
}
