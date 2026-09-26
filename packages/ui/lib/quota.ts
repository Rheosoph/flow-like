export interface QuotaDetail {
	resource: string;
	scope: string;
	payerId: string;
	plan: string;
	used: number;
	reserved: number;
	requested?: number;
	limit: number;
	unit: string;
	periodEnd?: string;
	actions?: string[];
	requiredModelTier?: string;
}

export function quotaBelongsToAnotherPayer(
	quota: QuotaDetail | undefined,
	userId?: string,
): boolean {
	return !!quota?.payerId && quota.payerId !== userId;
}

export function supportsHostedModel(
	tier: { llm_tiers: string[] },
	requiredModelTier?: string,
): boolean {
	return (
		!requiredModelTier ||
		tier.llm_tiers.some(
			(modelTier) =>
				modelTier.toUpperCase() === requiredModelTier.toUpperCase(),
		)
	);
}

export interface QuotaResource {
	resource: string;
	used: number;
	reserved: number;
	limit: number;
	remaining: number | null;
	unit: string;
	threshold: number;
}

export interface DailyQuotaUsage {
	day: string;
	appId?: string | null;
	modelId?: string | null;
	provider?: string | null;
	fundingClass: string;
	executionMode?: string | null;
	runtimeMs: number;
	aiCostMicros: number;
	aiCalls: number;
	cloudStarts: number;
}

export interface QuotaOverview {
	plan: string;
	payerId: string;
	periodStart: string;
	periodEnd: string;
	trackingSince?: string;
	resources: QuotaResource[];
	usage: DailyQuotaUsage[];
	updatedAt: string;
	/** Client-side timestamp when counters were refreshed without loading history. */
	countersUpdatedAt?: string;
	usageTruncated?: boolean;
	warnings?: {
		id: string;
		resource: string;
		threshold: number;
		episode: number;
		title: string;
		description: string;
	}[];
}

/** Keep history and its freshness while applying a newer summary for its owner. */
export function mergeQuotaCounters(
	overview: QuotaOverview,
	summary: QuotaOverview,
): QuotaOverview {
	if (overview.payerId !== summary.payerId) return overview;
	const incoming = Date.parse(summary.updatedAt);
	const current = Date.parse(overview.countersUpdatedAt ?? overview.updatedAt);
	if (
		!Number.isFinite(incoming) ||
		!Number.isFinite(current) ||
		incoming < current
	) {
		return overview;
	}
	return {
		...overview,
		plan: summary.plan,
		periodStart: summary.periodStart,
		periodEnd: summary.periodEnd,
		trackingSince: summary.trackingSince,
		resources: summary.resources,
		countersUpdatedAt: summary.updatedAt,
	};
}

export const quotaLabels: Record<string, string> = {
	cloud_runtime_ms: "Cloud runtime",
	hosted_ai_cost_micros: "Hosted AI usage",
	hosted_ai_calls: "Hosted AI operations",
	hosted_model_access: "Hosted model access",
	cloud_starts: "Cloud starts",
	concurrent_cloud_executions: "Concurrent cloud executions",
	storage_bytes: "Cloud storage",
	projects: "Private cloud projects",
	project_count: "Private cloud projects",
};

export function formatQuotaDate(value: string): string {
	const date = new Date(value);
	if (Number.isNaN(date.getTime())) return "Not available";
	return new Intl.DateTimeFormat(undefined, {
		day: "numeric",
		month: "short",
		year: "numeric",
	}).format(date);
}

export function formatQuota(value: number, resource: string): string {
	if (!Number.isFinite(value)) return "Not available";
	if (value < 0) return "Unlimited";
	if (resource === "cloud_runtime_ms") {
		return value >= 3_600_000
			? `${(value / 3_600_000).toLocaleString(undefined, { maximumFractionDigits: 2 })} h`
			: `${(value / 60_000).toLocaleString(undefined, { maximumFractionDigits: 1 })} min`;
	}
	if (resource === "hosted_ai_cost_micros") {
		return new Intl.NumberFormat(undefined, {
			style: "currency",
			currency: "EUR",
			maximumFractionDigits: 4,
		}).format(value / 1_000_000);
	}
	if (resource === "storage_bytes") {
		return `${(value / 1_000_000_000).toLocaleString(undefined, { maximumFractionDigits: 2 })} GB`;
	}
	return value.toLocaleString();
}

export function quotaWarningKey(
	overview: QuotaOverview,
	resource: QuotaResource,
	threshold: number,
): string {
	const period =
		resource.resource === "storage_bytes" ||
		resource.resource.includes("project")
			? "occupancy"
			: overview.periodStart;
	return `quota:${overview.payerId}:${period}:${resource.resource}:${threshold}`;
}

export interface QuotaOperation {
	id: string;
	appId?: string | null;
	modelId?: string | null;
	provider?: string | null;
	kind: string;
	fundingClass: string;
	executionMode: string;
	status: string;
	used: {
		runtimeMs: number;
		aiCostMicros: number;
		aiCalls: number;
		cloudStarts: number;
	};
	reserved: {
		runtimeMs: number;
		aiCostMicros: number;
		aiCalls: number;
		cloudStarts: number;
	};
	createdAt: string;
}
export interface QuotaOperationsPage {
	items: QuotaOperation[];
	nextCursor?: string | null;
}

export function usageCsv(rows: DailyQuotaUsage[]): string {
	const cell = (value: unknown) => {
		let text = String(value ?? "");
		if (/^[=+@\-\t\r]/.test(text)) text = `'${text}`;
		return `"${text.replaceAll('"', '""')}"`;
	};
	return [
		[
			"day",
			"app_id",
			"model_id",
			"provider",
			"funding",
			"execution_mode",
			"runtime_ms",
			"ai_eur",
			"ai_operations",
			"cloud_starts",
		],
		...rows.map((row) => [
			row.day,
			row.appId,
			row.modelId,
			row.provider,
			row.fundingClass,
			row.executionMode,
			row.runtimeMs,
			row.aiCostMicros / 1_000_000,
			row.aiCalls,
			row.cloudStarts,
		]),
	]
		.map((row) => row.map(cell).join(","))
		.join("\r\n");
}

export interface QuotaOperationDetail {
	meteringBasis?: string | null;
	inputBytes?: number | null;
	providerReportedWords?: number | null;
	tokenCountEstimated?: boolean;
	operationId: string;
	kind: string;
	fundingClass: string;
	status: string;
	inputTokens?: number | null;
	outputTokens?: number | null;
	embeddingTokens?: number | null;
	providerCostMicroUsd?: number | null;
	providerFundingCostMicroUsd?: number | null;
	servingCostMicroUsd?: number | null;
	costMicroEur?: number | null;
	rateVersion?: string | null;
	usdMicroPerEur?: number | null;
	providerRequestId?: string | null;
	estimated: boolean;
	servingCostEstimated: boolean;
}
