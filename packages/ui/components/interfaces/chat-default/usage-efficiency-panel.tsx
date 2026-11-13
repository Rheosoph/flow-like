import { useTranslation } from "@flow-like/locales";
import DatabaseIcon from "lucide-react/dist/esm/icons/database.js";
import FoldVerticalIcon from "lucide-react/dist/esm/icons/fold-vertical.js";
import type { UsageEfficiency } from "./usage-efficiency";

const number = (value: number) => Math.round(value).toLocaleString();
const percent = (value: number) => `${Math.round(value * 100)}%`;
const duration = (value: number) =>
	value < 1_000 ? `${Math.round(value)} ms` : `${(value / 1_000).toFixed(1)} s`;
const bounded = (value: number) => Math.min(100, Math.max(0, value));

export function UsageEfficiencyPanel({ data }: { data: UsageEfficiency }) {
	const { t } = useTranslation("chat");
	const unknown = t("usageEfficiency.notReported", "Not reported");
	const readShare = data.cacheShare;
	const context = data.context;
	const contextShare = context ? context.tokens / context.context_window : 0;
	const reserveShare = context
		? context.reserved_output_tokens / context.context_window
		: 0;
	const statusLabel = (status: string) => {
		if (status === "completed" || status === "success")
			return t("usageEfficiency.completed", "Completed");
		if (status === "failed") return t("usageEfficiency.failed", "Failed");
		if (status === "reused") return t("usageEfficiency.reused", "Reused");
		if (status === "stale") return t("usageEfficiency.stale", "Superseded");
		if (status === "busy") return t("usageEfficiency.deferred", "Deferred");
		return t("usageEfficiency.pending", "Pending");
	};
	const triggerLabel = (trigger: string) => {
		if (trigger === "soft")
			return t("usageEfficiency.background", "Background");
		if (trigger === "hard")
			return t("usageEfficiency.beforeRequest", "Before request");
		if (trigger === "manual")
			return t("usageEfficiency.requested", "Requested");
		if (trigger === "reused" || trigger === "reuse" || trigger === "carry")
			return t("usageEfficiency.previousTurn", "Previous turn");
		return t("usageEfficiency.contextLimit", "Context threshold");
	};

	return (
		<section
			aria-label={t("usageEfficiency.title", "Cache and context")}
			className="space-y-4"
		>
			<div className="rounded-xl border bg-muted/10 p-4 space-y-3">
				<div className="flex items-start justify-between gap-3">
					<div>
						<h3 className="flex items-center gap-1.5 text-sm font-medium">
							<DatabaseIcon
								aria-hidden="true"
								className="size-3.5 text-muted-foreground"
							/>
							{t("usageEfficiency.promptCache", "Prompt cache")}
						</h3>
						<p className="mt-1 text-xs text-muted-foreground">
							{t(
								"usageEfficiency.cacheHint",
								"Input reused by the model provider.",
							)}
						</p>
					</div>
					<div className="text-right shrink-0">
						<div className="text-xl font-semibold tabular-nums">
							{readShare === null ? unknown : percent(readShare)}
						</div>
						{readShare !== null && (
							<div className="text-[11px] text-muted-foreground">
								{data.partialReads
									? t("usageEfficiency.reportedInput", "of reported input")
									: t("usageEfficiency.ofInput", "of input")}
							</div>
						)}
					</div>
				</div>
				{readShare !== null && (
					<div
						role="meter"
						aria-label={t("usageEfficiency.cachedInput", "Cached input")}
						aria-valuemin={0}
						aria-valuemax={100}
						aria-valuenow={Math.round(readShare * 100)}
						className="h-2 overflow-hidden rounded-full bg-muted"
					>
						<div
							className="h-full rounded-full bg-emerald-500"
							style={{ width: `${bounded(readShare * 100)}%` }}
						/>
					</div>
				)}
				<dl className="grid grid-cols-2 gap-3 text-xs">
					<div>
						<dt className="text-muted-foreground">
							{t("usageEfficiency.cacheRead", "Read from cache")}
						</dt>
						<dd className="mt-1 font-medium tabular-nums">
							{data.cacheReadTokens === null
								? unknown
								: `${number(data.cacheReadTokens)} ${t("usageEfficiency.tokenUnit", "tokens")}`}
						</dd>
					</div>
					<div>
						<dt className="text-muted-foreground">
							{t("usageEfficiency.cacheWrite", "Written to cache")}
						</dt>
						<dd className="mt-1 font-medium tabular-nums">
							{data.cacheWriteTokens === null
								? unknown
								: `${number(data.cacheWriteTokens)} ${t("usageEfficiency.tokenUnit", "tokens")}`}
							{data.partialWrites && (
								<span className="ml-1 font-normal text-muted-foreground">
									({t("usageEfficiency.partial", "partial")})
								</span>
							)}
						</dd>
					</div>
				</dl>
				{data.partialReads && (
					<p className="text-[11px] text-muted-foreground">
						{t(
							"usageEfficiency.coverage",
							"Cache reporting covers {{reported}} of {{total}} input tokens. Unreported calls are excluded from the percentage.",
							{
								reported: number(data.reportedInputTokens),
								total: number(data.inputTokens),
							},
						)}
					</p>
				)}
				{data.cacheReadTokens === null && (
					<p className="text-[11px] text-muted-foreground">
						{t(
							"usageEfficiency.unknownHint",
							"This response has no cache-read measurement. It may still have used the provider's cache.",
						)}
					</p>
				)}
			</div>

			{context && (
				<div className="rounded-xl border p-4 space-y-3">
					<div className="flex items-center justify-between gap-3">
						<h3 className="text-sm font-medium">
							{t("usageEfficiency.context", "Context window")}
						</h3>
						<span className="font-semibold tabular-nums">
							{percent(contextShare)}
						</span>
					</div>
					<p className="text-xs text-muted-foreground">
						{context.measured
							? t(
									"usageEfficiency.measuredContext",
									"Latest input reported by the model",
								)
							: t(
									"usageEfficiency.estimatedContext",
									"Estimated input for the latest request",
								)}
					</p>
					<div
						role="meter"
						aria-label={t("usageEfficiency.contextFill", "Context window used")}
						aria-valuemin={0}
						aria-valuemax={100}
						aria-valuenow={bounded(Math.round(contextShare * 100))}
						aria-valuetext={`${number(context.tokens)} / ${number(context.context_window)}`}
						className="flex h-2 overflow-hidden rounded-full bg-muted"
					>
						<div
							className={
								contextShare + reserveShare >= 0.9
									? "bg-amber-500"
									: "bg-sky-500"
							}
							style={{ width: `${bounded(contextShare * 100)}%` }}
						/>
						<div
							className="bg-violet-400/50"
							style={{
								width: `${Math.min(bounded(reserveShare * 100), Math.max(0, 100 - contextShare * 100))}%`,
							}}
						/>
					</div>
					<div className="flex flex-wrap justify-between gap-2 text-xs text-muted-foreground">
						<span>
							{number(context.tokens)} / {number(context.context_window)}{" "}
							{t("usageEfficiency.tokenUnit", "tokens")}
						</span>
						<span>
							{t("usageEfficiency.reserved", "{{tokens}} reserved for output", {
								tokens: number(context.reserved_output_tokens),
							})}
						</span>
					</div>
				</div>
			)}

			{data.compactions.length + data.compactionsOmitted > 0 && (
				<div className="space-y-3">
					<h3 className="flex items-center gap-1.5 text-sm font-medium">
						<FoldVerticalIcon
							aria-hidden="true"
							className="size-3.5 text-muted-foreground"
						/>
						{t("usageEfficiency.compactions", "Compaction")}
						<span className="ml-auto text-xs tabular-nums text-muted-foreground">
							{data.compactions.length + data.compactionsOmitted}
						</span>
					</h3>
					<p className="text-xs text-muted-foreground">
						{t(
							"usageEfficiency.compactionHint",
							"Older context is summarized to leave room for new work. Sizes are estimates.",
						)}
					</p>
					<ol className="divide-y rounded-xl border">
						{data.compactions.map((event, index) => {
							const after =
								typeof event.after_tokens === "number" &&
								Number.isFinite(event.after_tokens)
									? event.after_tokens
									: null;
							const reduction =
								after !== null &&
								event.before_tokens > 0 &&
								after < event.before_tokens
									? 1 - after / event.before_tokens
									: null;
							return (
								<li key={`${event.step}-${index}`} className="space-y-2 p-3">
									<div className="flex flex-wrap items-center justify-between gap-2 text-xs">
										<span className="font-medium">
											{event.step} · {triggerLabel(event.trigger)}
										</span>
										<span
											className={
												event.status === "failed"
													? "font-medium text-destructive"
													: "text-muted-foreground"
											}
										>
											{statusLabel(event.status)}
										</span>
									</div>
									<div className="flex flex-wrap items-baseline justify-between gap-2">
										<span className="text-sm tabular-nums">
											{number(event.before_tokens)}{" "}
											<span
												aria-hidden="true"
												className="px-1 text-muted-foreground"
											>
												→
											</span>
											<span className="sr-only">
												{t("usageEfficiency.to", "to")}{" "}
											</span>
											{after === null ? unknown : number(after)}{" "}
											<span className="text-xs text-muted-foreground">
												{t("usageEfficiency.tokenUnit", "tokens")}
											</span>
										</span>
										<span className="text-[11px] text-muted-foreground tabular-nums">
											{reduction !== null && (
												<>
													{t(
														"usageEfficiency.reduction",
														"{{percent}} smaller",
														{ percent: percent(reduction) },
													)}{" "}
													·{" "}
												</>
											)}
											{duration(event.duration_ms)}
										</span>
									</div>
									{after !== null && event.before_tokens > 0 && (
										<div
											aria-hidden="true"
											className="h-1 rounded-full bg-muted overflow-hidden"
										>
											<div
												className="h-full rounded-full bg-violet-400"
												style={{
													width: `${bounded((after / event.before_tokens) * 100)}%`,
												}}
											/>
										</div>
									)}
								</li>
							);
						})}
					</ol>
					{data.compactionsOmitted > 0 && (
						<p className="text-[11px] text-muted-foreground">
							{t(
								"usageEfficiency.compactionsOmitted",
								"{{count}} earlier compactions are not shown.",
								{ count: data.compactionsOmitted },
							)}
						</p>
					)}
				</div>
			)}
			{data.maintenanceTokens !== null && (
				<p className="text-xs text-muted-foreground">
					{t(
						"usageEfficiency.maintenanceTokens",
						"Recorded context and memory work: {{tokens}} tokens, included in usage.",
						{ tokens: number(data.maintenanceTokens) },
					)}
					{data.maintenanceCost !== null && (
						<>
							{" "}
							{t(
								"usageEfficiency.maintenanceCost",
								"Reported cost: {{cost}}.",
								{ cost: `$${data.maintenanceCost.toFixed(4)}` },
							)}
						</>
					)}
				</p>
			)}
			{data.callsOmitted > 0 && (
				<p className="text-[11px] text-muted-foreground">
					{t(
						"usageEfficiency.callsOmitted",
						"Showing recent calls. {{count}} earlier calls are included in totals.",
						{ count: data.callsOmitted },
					)}
				</p>
			)}
			{data.hasMemoryWork && (
				<p className="text-[11px] text-muted-foreground">
					{t(
						"usageEfficiency.memoryTiming",
						"Background memory work can be reported with the following reply.",
					)}
				</p>
			)}
		</section>
	);
}
