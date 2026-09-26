"use client";

import { useAuth } from "react-oidc-context";
import { useQueryClient } from "@tanstack/react-query";
import { UsageOperations } from "./usage-operations";
import { usageFundingLabel, useUsageNames } from "./use-usage-names";
import { useMemo, useState } from "react";
import { useInvoke } from "../../../hooks/use-invoke";
import { useBackend } from "../../../state/backend-state";
import {
	type QuotaResource,
	formatQuota,
	formatQuotaDate,
	quotaLabels,
	usageCsv,
} from "../../../lib/quota";
import {
	ArrowUpRight,
	Database,
	Clock3,
	Sparkles,
	RefreshCw,
	ChevronDown,
} from "lucide-react";
import { asArray, isRecord } from "../../../lib/response-shape";
import { cn } from "../../../lib/utils";
import { openUpgradeDialog } from "../../../state/upgrade-dialog-state";
import { Button } from "../../ui/button";
import { RuntimeCalculator } from "../../upgrade/runtime-calculator";
import {
	availableRuntimeCapacity,
	estimateCloudRuns,
	formatRuntimeSeconds,
	getRuntimeSample,
} from "../../../lib/runtime-estimate";

const PRIMARY_RESOURCES = [
	"hosted_ai_cost_micros",
	"cloud_runtime_ms",
	"storage_bytes",
];

function QuotaMeter({
	resource,
	compact = false,
	runtimeNote,
}: { resource: QuotaResource; compact?: boolean; runtimeNote?: string }) {
	const { resource: key, used, reserved, limit } = resource;
	const total = used + reserved;
	const unlimited = limit < 0;
	const percent = limit > 0 ? (total / limit) * 100 : total > 0 ? 100 : 0;
	const full = !unlimited && (limit === 0 || total >= limit);
	const near = !unlimited && percent >= 75;
	const available = Math.max(0, limit - total);
	const usedWidth = limit > 0 ? Math.min(100, (used / limit) * 100) : 0;
	const pendingWidth =
		limit > 0 ? Math.min(100 - usedWidth, (reserved / limit) * 100) : 0;
	const label = quotaLabels[key] ?? key;
	const Icon =
		key === "hosted_ai_cost_micros"
			? Sparkles
			: key === "storage_bytes"
				? Database
				: Clock3;
	const valueText =
		limit === 0
			? `No allowance included. ${formatQuota(used, key)} used; ${formatQuota(reserved, key)} pending.`
			: `${formatQuota(used, key)} used, ${formatQuota(reserved, key)} pending, ${formatQuota(available, key)} available of ${formatQuota(limit, key)}.`;
	return (
		<article
			aria-label={label}
			className={cn(
				"min-w-0 space-y-4",
				!compact && "rounded-2xl border bg-card p-5",
				full && !compact && "border-destructive/35",
				near && !full && !compact && "border-amber-500/40",
			)}
		>
			<div className="flex items-center justify-between gap-3">
				<h3 className="flex items-center gap-2 text-sm font-medium">
					{!compact && (
						<Icon className="size-4 text-muted-foreground" aria-hidden="true" />
					)}
					{label}
				</h3>
				{(full || near) && (
					<span
						className={cn(
							"shrink-0 rounded-full px-2 py-0.5 text-xs font-medium",
							full
								? "bg-destructive/10 text-destructive"
								: "bg-amber-500/10 text-amber-700 dark:text-amber-400",
						)}
					>
						{limit === 0 ? "Not included" : full ? "Full" : "Near limit"}
					</span>
				)}
			</div>
			<div className="flex flex-wrap items-baseline gap-x-2 gap-y-1 tabular-nums">
				<p
					className={cn(
						"font-semibold tracking-tight",
						compact ? "text-lg" : "text-3xl",
					)}
				>
					{formatQuota(used, key)}
				</p>
				<p className="text-sm text-muted-foreground">
					{unlimited ? "used · unlimited" : `of ${formatQuota(limit, key)}`}
				</p>
			</div>
			{!unlimited && (
				<div
					role="progressbar"
					aria-label={label}
					aria-valuemin={0}
					aria-valuemax={100}
					aria-valuenow={Math.min(100, percent)}
					aria-valuetext={valueText}
					className="flex h-2 overflow-hidden rounded-full bg-muted"
				>
					<div
						data-usage-segment="settled"
						style={{ width: `${usedWidth}%` }}
						className={cn(
							"h-full",
							full
								? "bg-destructive"
								: near
									? "bg-amber-500"
									: "bg-foreground/60",
						)}
					/>
					<div
						data-usage-segment="pending"
						style={{
							width: `${pendingWidth}%`,
							backgroundImage:
								"repeating-linear-gradient(135deg, transparent, transparent 3px, currentColor 3px, currentColor 5px)",
						}}
						className={cn(
							"h-full",
							full
								? "bg-destructive/15 text-destructive/50"
								: near
									? "bg-amber-500/15 text-amber-500/60"
									: "bg-foreground/10 text-foreground/35",
						)}
					/>
				</div>
			)}
			<div className="flex flex-wrap justify-between gap-x-4 gap-y-1 text-xs text-muted-foreground tabular-nums">
				<span>
					{reserved > 0
						? `${formatQuota(reserved, key)} pending`
						: "No pending usage"}
				</span>
				<span>
					{unlimited
						? "No plan limit"
						: `${formatQuota(available, key)} available`}
				</span>
			</div>
			{key === "concurrent_cloud_executions" && (
				<p className="text-xs text-muted-foreground">
					Active runs release their slots when they finish.
				</p>
			)}
			{key === "cloud_runtime_ms" && runtimeNote && (
				<p className="text-xs leading-relaxed text-muted-foreground">
					{runtimeNote}
				</p>
			)}
		</article>
	);
}

export function UsageOverview() {
	const backend = useBackend();
	const auth = useAuth();
	const queryClient = useQueryClient();
	const [refreshing, setRefreshing] = useState(false);
	const [refreshFailed, setRefreshFailed] = useState(false);
	const [page, setPage] = useState(0);
	const usage = useInvoke(
		backend.userState.getQuotaUsage,
		backend.userState,
		[],
		!!auth.user?.profile.sub,
		[auth.user?.profile.sub],
	);
	const refreshUsage = async () => {
		const userId = auth.user?.profile.sub;
		if (!userId) return;
		setRefreshing(true);
		setRefreshFailed(false);
		const operationQueryNames = new Set([
			backend.userState.getQuotaOperations.name || "backendFn",
			backend.userState.getQuotaOperationDetail.name || "backendFn",
		]);
		const results = await Promise.allSettled([
			usage.refetch({ throwOnError: true }),
			queryClient.invalidateQueries(
				{
					predicate: ({ queryKey }) =>
						queryKey.length === 3 &&
						operationQueryNames.has(String(queryKey[0])) &&
						queryKey[2] === userId,
				},
				{ throwOnError: true },
			),
		]);
		setRefreshFailed(results.some((result) => result.status === "rejected"));
		setRefreshing(false);
	};
	const [app, setApp] = useState("");
	const [model, setModel] = useState("");
	const rows = asArray(usage.data?.usage);
	const filtered = useMemo(
		() =>
			rows.filter(
				(row) =>
					(!app || (row.appId ?? "standalone") === app) &&
					(!model || row.modelId === model),
			),
		[rows, app, model],
	);
	const currentPage = Math.min(
		page,
		Math.max(0, Math.ceil(filtered.length / 20) - 1),
	);
	const displayed = filtered.slice(currentPage * 20, (currentPage + 1) * 20);
	const names = useUsageNames(displayed);
	if (usage.isLoading)
		return <p className="text-sm text-muted-foreground">Loading usage…</p>;
	if (!isRecord(usage.data) || !Array.isArray(usage.data.resources))
		return (
			<div className="rounded-xl border p-4">
				<p>Usage is temporarily unavailable.</p>
				<Button variant="outline" size="sm" onClick={refreshUsage}>
					Try again
				</Button>
			</div>
		);
	const overview = usage.data;
	const runtimeSample = getRuntimeSample({
		usage: rows,
		usageTruncated: overview.usageTruncated,
	});
	const runtimeResource = overview.resources.find(
		(item) => item.resource === "cloud_runtime_ms",
	);
	const startsResource = overview.resources.find(
		(item) => item.resource === "cloud_starts",
	);
	const availableRuntime = availableRuntimeCapacity(runtimeResource);
	const availableStarts = availableRuntimeCapacity(startsResource);
	const runsRemaining = runtimeSample
		? estimateCloudRuns(
				availableRuntime,
				availableStarts,
				runtimeSample.averageRuntimeMs,
			)
		: null;
	const runtimeNote =
		runtimeSample && runsRemaining?.runs != null
			? `About ${runsRemaining.runs.toLocaleString()} more cloud runs at your recent average of ${formatRuntimeSeconds(runtimeSample.averageRuntimeMs)} seconds per run.${runsRemaining.startLimited ? " Cloud-start limit reached first." : ""} Other plan limits still apply.`
			: "Use the calculator below to estimate how many cloud runs this covers.";
	const primary = PRIMARY_RESOURCES.flatMap((key) =>
		overview.resources.filter((resource) => resource.resource === key),
	);
	const secondary = overview.resources.filter(
		(resource) => !PRIMARY_RESOURCES.includes(resource.resource),
	);
	const attention = overview.resources
		.filter(
			(resource) =>
				resource.limit >= 0 &&
				resource.resource !== "concurrent_cloud_executions" &&
				(resource.limit === 0
					? resource.used + resource.reserved > 0
					: resource.used + resource.reserved >= resource.limit * 0.75),
		)
		.sort(
			(a, b) =>
				(b.used + b.reserved) / Math.max(1, b.limit) -
				(a.used + a.reserved) / Math.max(1, a.limit),
		);
	const hasPending = overview.resources.some(
		(resource) => resource.reserved > 0,
	);
	const apps = [...new Set(rows.map((row) => row.appId ?? "standalone"))];
	const models = [
		...new Set(rows.flatMap((row) => (row.modelId ? [row.modelId] : []))),
	];
	const exportUsage = () => {
		const url = URL.createObjectURL(
			new Blob([usageCsv(filtered)], {
				type: "text/csv;charset=utf-8",
			}),
		);
		const link = document.createElement("a");
		link.href = url;
		link.download = "flow-like-usage.csv";
		link.click();
		URL.revokeObjectURL(url);
	};
	return (
		<section className="space-y-6" aria-label="Current usage">
			<div className="flex flex-wrap items-start justify-between gap-3">
				<div className="space-y-1">
					<h2 className="text-xl font-semibold">
						Your {overview.plan?.toLowerCase()} usage
					</h2>
					<p className="text-sm text-muted-foreground">
						Monthly allowances renew {formatQuotaDate(overview.periodEnd)}.
						Storage and projects measure current usage.
					</p>
					{overview.trackingSince &&
						new Date(overview.trackingSince) >
							new Date(overview.periodStart) && (
							<p className="text-xs text-muted-foreground">
								Tracked since {formatQuotaDate(overview.trackingSince)}. Usage
								before tracking began is excluded from this first period.
							</p>
						)}
				</div>
				<Button
					variant="outline"
					size="sm"
					onClick={refreshUsage}
					disabled={refreshing || usage.isFetching}
				>
					<RefreshCw
						className={cn(
							"size-3.5",
							(refreshing || usage.isFetching) && "animate-spin",
						)}
						aria-hidden="true"
					/>
					Refresh usage
				</Button>
			</div>
			{(usage.isError || refreshFailed) && (
				<p
					role="status"
					className="rounded-xl border border-amber-500/30 bg-amber-500/5 px-4 py-3 text-sm text-amber-800 dark:text-amber-300"
				>
					Some usage could not be refreshed. The overview was last updated{" "}
					{new Date(overview.updatedAt).toLocaleString(undefined, {
						dateStyle: "medium",
						timeStyle: "short",
					})}
					. Try refreshing again.
				</p>
			)}
			<div className="grid gap-4 md:grid-cols-3">
				{primary.map((resource) => (
					<QuotaMeter
						key={resource.resource}
						resource={resource}
						runtimeNote={runtimeNote}
					/>
				))}
			</div>
			{hasPending && (
				<p className="flex items-start gap-2 text-xs text-muted-foreground">
					<span
						aria-hidden="true"
						className="mt-0.5 h-3 w-3 shrink-0 rounded-sm border border-foreground/30 bg-foreground/10"
					/>
					Striped usage is reserved for work in progress or awaiting a final
					cost. Unused reservations return to your available allowance.
				</p>
			)}
			{runtimeResource && (
				<RuntimeCalculator
					sample={runtimeSample}
					remaining={{
						id: "remaining",
						name: "Available now",
						runtimeMs: availableRuntime,
						cloudStarts: availableStarts,
					}}
					plans={[
						{
							id: "monthly",
							name: "Full monthly allowance",
							runtimeMs: runtimeResource.limit,
							cloudStarts: startsResource?.limit,
						},
					]}
				/>
			)}
			{attention.length > 0 && (
				<div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-amber-500/25 bg-amber-500/5 px-4 py-3">
					<p className="min-w-0 text-sm">
						{attention.length === 1
							? `${quotaLabels[attention[0].resource] ?? attention[0].resource} is ${attention[0].limit === 0 ? "not included in your plan" : attention[0].used + attention[0].reserved >= attention[0].limit ? "at your plan limit" : "nearing your plan limit"}.`
							: `${attention.length} allowances are nearing or at your plan limits.`}
					</p>
					<Button
						size="sm"
						variant="outline"
						className="shrink-0"
						onClick={() =>
							openUpgradeDialog({
								reason: "generic",
								message: `${quotaLabels[attention[0].resource] ?? attention[0].resource}: ${formatQuota(attention[0].used, attention[0].resource)} used${attention[0].reserved > 0 ? ` and ${formatQuota(attention[0].reserved, attention[0].resource)} pending` : ""} of ${formatQuota(attention[0].limit, attention[0].resource)}.`,
								quota: {
									...attention[0],
									scope: "user",
									payerId: overview.payerId,
									plan: overview.plan,
									periodEnd:
										attention[0].resource === "storage_bytes" ||
										attention[0].resource.includes("project")
											? undefined
											: overview.periodEnd,
								},
							})
						}
					>
						Compare plans
						<ArrowUpRight className="size-3.5" aria-hidden="true" />
					</Button>
				</div>
			)}
			{secondary.length > 0 && (
				<details className="group rounded-xl border">
					<summary className="flex cursor-pointer list-none items-center justify-between gap-3 px-5 py-4 [&::-webkit-details-marker]:hidden">
						<div>
							<h3 className="text-sm font-medium">Other plan limits</h3>
							<p className="mt-0.5 text-xs text-muted-foreground">
								AI operations, cloud starts, projects and concurrent runs
							</p>
						</div>
						<ChevronDown
							className="size-4 shrink-0 text-muted-foreground transition-transform group-open:rotate-180"
							aria-hidden="true"
						/>
					</summary>
					<div className="grid gap-6 border-t p-5 sm:grid-cols-2">
						{secondary.map((resource) => (
							<QuotaMeter key={resource.resource} resource={resource} compact />
						))}
					</div>
				</details>
			)}

			<div className="rounded-xl border p-4 space-y-4">
				<div className="flex flex-wrap items-center gap-3">
					<div className="mr-auto">
						<h3 className="font-medium">Usage by app and model</h3>
						<p className="mt-1 text-xs text-muted-foreground">
							Past 30 days, in UTC. This can span billing periods.
						</p>
					</div>
					<select
						aria-label="Filter by app"
						className="min-w-0 max-w-full truncate rounded border bg-background p-2 text-sm sm:max-w-64"
						value={app}
						onChange={(event) => {
							setApp(event.target.value);
							setPage(0);
						}}
					>
						<option value="">All apps</option>
						{apps.map((id) => (
							<option key={id} value={id}>
								{id === "standalone" ? "Standalone AI" : names.appName(id)}
							</option>
						))}
					</select>
					<select
						aria-label="Filter by model"
						className="min-w-0 max-w-full truncate rounded border bg-background p-2 text-sm sm:max-w-64"
						value={model}
						onChange={(event) => {
							setModel(event.target.value);
							setPage(0);
						}}
					>
						<option value="">All models</option>
						{models.map((id) => (
							<option key={id} value={id}>
								{names.modelName(id)}
							</option>
						))}
					</select>
					<Button size="sm" variant="outline" onClick={exportUsage}>
						Export filtered CSV
					</Button>
				</div>
				<div className="overflow-auto">
					<table className="w-full text-sm text-left">
						<thead>
							<tr className="border-b">
								{[
									"Day",
									"App",
									"Model / provider",
									"Paid through",
									"Cloud runtime",
									"Cloud starts",
									"Hosted AI usage",
									"AI operations",
								].map((label) => (
									<th key={label} className="p-2 font-medium">
										{label}
									</th>
								))}
							</tr>
						</thead>
						<tbody>
							{displayed.map((row, i) => (
								<tr
									key={`${row.day}:${row.appId}:${row.modelId}:${row.fundingClass}:${i}`}
									className="border-b last:border-0"
								>
									<td className="p-2 whitespace-nowrap">{row.day}</td>
									<td className="p-2">{names.appName(row.appId)}</td>
									<td className="p-2">
										{names.modelName(row.modelId)}
										{row.provider ? ` / ${row.provider}` : ""}
									</td>
									<td className="p-2">{usageFundingLabel(row.fundingClass)}</td>
									<td className="p-2">
										{formatQuota(row.runtimeMs, "cloud_runtime_ms")}
									</td>
									<td className="p-2">
										{(row.cloudStarts ?? 0).toLocaleString()}
									</td>
									<td className="p-2">
										{formatQuota(row.aiCostMicros, "hosted_ai_cost_micros")}
									</td>
									<td className="p-2">{(row.aiCalls ?? 0).toLocaleString()}</td>
								</tr>
							))}
						</tbody>
					</table>
					{filtered.length === 0 && (
						<p className="py-6 text-center text-muted-foreground">
							No recorded cloud usage in this view yet.
						</p>
					)}
				</div>
				<div className="flex items-center justify-between gap-2 text-sm">
					<Button
						variant="outline"
						size="sm"
						disabled={currentPage === 0}
						onClick={() => setPage(currentPage - 1)}
					>
						Previous
					</Button>
					<span>
						{filtered.length ? currentPage * 20 + 1 : 0}–
						{Math.min((currentPage + 1) * 20, filtered.length)} of{" "}
						{filtered.length} rows
					</span>
					<Button
						variant="outline"
						size="sm"
						disabled={(currentPage + 1) * 20 >= filtered.length}
						onClick={() => setPage(currentPage + 1)}
					>
						Next
					</Button>
				</div>
				{overview.usageTruncated && (
					<p className="text-sm text-amber-700 dark:text-amber-400">
						This summary contains the latest 1,000 daily rows. The CSV exports
						those loaded rows. Browse operation history below for more detail.
					</p>
				)}
				<p className="text-xs text-muted-foreground">
					Hosted AI usage is shown in EUR. Your own models use no hosted AI
					allowance; cloud runs still use runtime. Local execution and local
					embeddings are always free.
				</p>
			</div>
			<UsageOperations />
			{rows.some((row) => row.aiCostMicros > 0) && (
				<details className="rounded-xl border px-4 py-3">
					<summary className="cursor-pointer text-sm font-medium">
						Ways to use less hosted AI
					</summary>
					<p className="pt-3 text-sm leading-relaxed text-muted-foreground">
						Choose a smaller model, shorten prompts, reuse embeddings, or
						connect your own provider. Compare model costs above before changing
						a workflow.
					</p>
				</details>
			)}
		</section>
	);
}
