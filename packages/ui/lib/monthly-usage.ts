export interface MonthlyUsageLimits {
	aiCostMicros: number | null;
	aiCalls: number | null;
	runtimeMs: number | null;
}

export interface MonthlyUsageAmounts {
	aiCostMicros: number;
	aiCalls: number;
	runtimeMs: number;
	cloudStarts: number;
}

export interface UsageBudgetView {
	scope: "bot" | "bot_group" | "app";
	subjectId: string;
	groupId: string | null;
	name: string | null;
	archived?: boolean;
	ownerLimits: MonthlyUsageLimits;
	adminLimits: MonthlyUsageLimits | null;
	defaultLimits: MonthlyUsageLimits;
	effectiveLimits: MonthlyUsageLimits;
	periodStart: string;
	periodEnd: string;
	used: MonthlyUsageAmounts;
	reserved: MonthlyUsageAmounts;
	inputTokens: number;
	outputTokens: number;
	embeddingTokens: number;
	models: {
		modelId: string | null;
		provider: string | null;
		calls: number;
		inputTokens: number;
		outputTokens: number;
		embeddingTokens: number;
		costMicroDollars: number;
	}[];
	groups: UsageBudgetView[];
}

export interface EnterpriseUsageDefaults {
	apps: MonthlyUsageLimits;
	bots: MonthlyUsageLimits;
}

export interface BotUsageOverview {
	items: UsageBudgetView[];
	total: number;
}

export const EMPTY_MONTHLY_LIMITS: MonthlyUsageLimits = {
	aiCostMicros: null,
	aiCalls: null,
	runtimeMs: null,
};

export function formatMonthlyLimit(value: number | null, scale = 1): string {
	if (value === null) return "";
	const amount = BigInt(value);
	const unit = BigInt(scale);
	const fraction = (amount % unit)
		.toString()
		.padStart(String(scale).length - 1, "0")
		.replace(/0+$/, "");
	return `${amount / unit}${fraction ? `.${fraction}` : ""}`;
}

/** Blank inherits the enclosing limit; zero stops new usage. */
export function parseMonthlyLimit(value: string, scale = 1): number | null {
	if (!value.trim()) return null;
	if (!/^\d+(?:\.\d+)?$/.test(value.trim())) {
		throw new Error("Enter a nonnegative number, or leave the field blank.");
	}
	const [whole, fraction = ""] = value.trim().split(".");
	const denominator = 10n ** BigInt(fraction.length);
	const numerator = BigInt(`${whole}${fraction}`) * BigInt(scale);
	const amount = numerator / denominator;
	if (
		numerator % denominator !== 0n ||
		amount > BigInt(Number.MAX_SAFE_INTEGER)
	) {
		throw new Error("Enter a smaller amount with fewer decimal places.");
	}
	return Number(amount);
}

export function botUsagePath(botId: string, groupId?: string | null): string {
	const base = `bots/${encodeURIComponent(botId)}/usage`;
	return groupId ? `${base}/rooms/${encodeURIComponent(groupId)}` : base;
}

export function adminUsageLimitPath(view: UsageBudgetView): string {
	if (view.scope === "app") {
		return `admin/usage/apps/${encodeURIComponent(view.subjectId)}/budget/limits`;
	}
	const base = `admin/usage/bots/${encodeURIComponent(view.subjectId)}`;
	return view.groupId
		? `${base}/rooms/${encodeURIComponent(view.groupId)}/limits`
		: `${base}/limits`;
}
