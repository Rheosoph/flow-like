import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { AuthContext, type AuthContextProps } from "react-oidc-context";
import { SubscriptionPage } from "../../packages/ui/components/settings/subscription/subscription-page";
import { UsageOverview } from "../../packages/ui/components/settings/subscription/usage-overview";
import { UsageOperations } from "../../packages/ui/components/settings/subscription/usage-operations";
import { UsageOperationDetails } from "../../packages/ui/components/settings/subscription/usage-operation-details";
import {
	GlobalUpgradeDialog,
	FreeUseTips,
} from "../../packages/ui/components/upgrade/upgrade-dialog";
import { TierCard } from "../../packages/ui/components/upgrade/tier-card";
import { Toaster } from "../../packages/ui/components/ui/sonner";
import { TooltipProvider } from "../../packages/ui/components/ui/tooltip";
import {
	useBackend,
	useBackendStore,
} from "../../packages/ui/state/backend-state";
import {
	openUpgradeDialog,
	type UpgradeReason,
} from "../../packages/ui/state/upgrade-dialog-state";
import {
	formatQuota,
	quotaLabels,
	type QuotaOverview,
	type QuotaDetail,
	type QuotaOperation,
} from "../../packages/ui/lib/quota";
import type {
	IPricingResponse,
	ITierInfo,
} from "../../packages/ui/state/backend-state/user-state";
import "../../packages/ui/global.css";

declare const __PRICING_CONFIG__: {
	tiers: Record<string, ITierInfo>;
	conversion: {
		enabled: boolean;
		mode: string;
		headline: string;
		subheadline: string;
		contact: { name: string; email: string; url: string };
		tier_display: Record<
			string,
			Partial<ITierInfo> & {
				monthly_price_cents?: number;
				annual_price_cents?: number;
			}
		>;
	};
	features: Record<string, boolean>;
};
const config = __PRICING_CONFIG__;
const params = new URLSearchParams(location.search);
const view = params.get("view") ?? "subscription";
const scenario = params.get("scenario") ?? "populated";
const currentTier =
	params.get("tier") ?? (view === "upgrade" ? "FREE" : "PREMIUM");
const theme = params.has("light") ? "light" : "dark";
document.documentElement.classList.toggle("dark", theme === "dark");
const periodEnd = "2026-10-01T00:00:00Z";
const noop = async () => {};
const pricing: IPricingResponse = {
	current_tier: currentTier,
	conversion: {
		enabled: true,
		mode: "consumer",
		headline: config.conversion.headline,
		subheadline: config.conversion.subheadline,
		contact_name: config.conversion.contact.name,
		contact_email: config.conversion.contact.email,
		contact_url: config.conversion.contact.url,
	},
	tiers: Object.fromEntries(
		Object.entries(config.tiers).map(([name, tier]) => {
			const display = config.conversion.tier_display[name];
			const prices =
				name === "ENTERPRISE"
					? []
					: [
							{
								id: `fixture_${name}_month`,
								amount: display.monthly_price_cents ?? 0,
								currency: "eur",
								interval: "month",
							},
							{
								id: `fixture_${name}_year`,
								amount: display.annual_price_cents ?? 0,
								currency: "eur",
								interval: "year",
							},
						];
			return [
				name,
				{
					...tier,
					...display,
					name,
					prices,
					price: prices[0],
					contact_url: config.conversion.contact.url,
				},
			];
		}),
	),
};

const apps: Record<string, string> = {
	"invoice-review": "Invoice Review",
	"research-assistant": "Research Assistant",
	"knowledge-search": "Knowledge Search",
};
const models: Record<string, string> = {
	"hosted-small": "Everyday text model",
	"hosted-reasoning": "Reasoning model",
	"embedding-model": "Document embeddings",
	"own-provider": "My connected model",
};
const allowance = config.tiers[currentTier];
const usage: QuotaOverview["usage"] = [
	{
		day: "2026-09-13",
		appId: "invoice-review",
		modelId: "hosted-small",
		provider: "Hosted provider",
		fundingClass: "hosted",
		executionMode: "realtime",
		runtimeMs: 10800000,
		aiCostMicros: 630000,
		aiCalls: 1420,
		cloudStarts: 360,
	},
	{
		day: "2026-09-13",
		appId: "research-assistant",
		modelId: "hosted-reasoning",
		provider: "Hosted provider",
		fundingClass: "hosted",
		executionMode: "async",
		runtimeMs: 16200000,
		aiCostMicros: 1200000,
		aiCalls: 870,
		cloudStarts: 240,
	},
	{
		day: "2026-09-12",
		appId: "knowledge-search",
		modelId: "embedding-model",
		provider: "Internal embeddings",
		fundingClass: "hosted",
		executionMode: "async",
		runtimeMs: 18000000,
		aiCostMicros: 650000,
		aiCalls: 5100,
		cloudStarts: 2000,
	},
	{
		day: "2026-09-12",
		appId: "invoice-review",
		modelId: "own-provider",
		provider: "Your provider",
		fundingClass: "byok",
		executionMode: "realtime",
		runtimeMs: 3600000,
		aiCostMicros: 0,
		aiCalls: 0,
		cloudStarts: 2800,
	},
	{
		day: "2026-09-11",
		appId: "invoice-review",
		modelId: "hosted-small",
		provider: "Hosted provider",
		fundingClass: "hosted",
		executionMode: "async",
		runtimeMs: 5400000,
		aiCostMicros: 220000,
		aiCalls: 810,
		cloudStarts: 100,
	},
].flatMap((row) => [
	{
		...row,
		runtimeMs: 0,
		cloudStarts: 0,
		executionMode: row.fundingClass === "byok" ? "customer_ai" : "hosted_ai",
	},
	{
		day: row.day,
		appId: row.appId,
		modelId: null,
		provider: null,
		fundingClass: "cloud",
		executionMode: row.executionMode,
		runtimeMs: row.runtimeMs,
		cloudStarts: row.cloudStarts,
		aiCostMicros: 0,
		aiCalls: 0,
	},
]);
const resourceConfig = [
	["cloud_runtime_ms", allowance.max_runtime_ms, 0.75, 0.01, "ms"],
	[
		"hosted_ai_cost_micros",
		allowance.max_ai_cost_micros,
		0.9,
		0.04,
		"micro_eur",
	],
	["hosted_ai_calls", allowance.max_llm_calls, 0.82, 0.002, "operations"],
	["cloud_starts", allowance.max_remote_executions, 0.55, 0, "starts"],
	[
		"concurrent_cloud_executions",
		allowance.max_concurrent_executions,
		0.4,
		0,
		"executions",
	],
	["storage_bytes", allowance.max_total_size, 0.62, 0.02, "bytes"],
	["projects", allowance.max_non_visible_projects, 0.14, 0, "projects"],
] as const;
const overview: QuotaOverview = {
	plan: currentTier,
	payerId: "preview-user",
	periodStart: "2026-09-01T00:00:00Z",
	periodEnd,
	updatedAt: "2026-09-13T12:00:00Z",
	warnings: [],
	trackingSince: scenario === "cutover" ? "2026-09-05T08:30:00Z" : undefined,
	resources: resourceConfig.map(
		([resource, rawLimit, fraction, pending, unit]) => {
			const limit = rawLimit ?? 0;
			const used =
				scenario === "empty"
					? 0
					: limit < 0
						? 28
						: Math.floor(limit * fraction);
			const reserved =
				scenario === "empty" ? 0 : Math.floor(Math.max(0, limit) * pending);
			return {
				resource,
				used,
				reserved,
				limit,
				remaining: limit < 0 ? null : limit - used - reserved,
				unit,
				threshold: fraction >= 0.9 ? 90 : fraction >= 0.75 ? 75 : 0,
			};
		},
	),
	usage:
		scenario === "empty"
			? []
			: usage.map((row) => ({
					...row,
					runtimeMs: Math.round(
						(row.runtimeMs * (allowance.max_runtime_ms ?? 0)) / 72000000,
					),
					aiCostMicros: Math.round(
						(row.aiCostMicros * (allowance.max_ai_cost_micros ?? 0)) / 3000000,
					),
					aiCalls: Math.round(
						(row.aiCalls * (allowance.max_llm_calls ?? 0)) / 10000,
					),
					cloudStarts: Math.round(
						(row.cloudStarts * allowance.max_remote_executions) / 10000,
					),
				})),
};
if (view === "warning") {
	const level = Number(params.get("level") ?? 75);
	const resource = overview.resources[1];
	resource.used = Math.floor((resource.limit * level) / 100);
	resource.reserved = 0;
	resource.threshold = level;
	for (const other of overview.resources)
		if (other !== resource) other.threshold = 0;
	// Match the retained notice returned by quota_warnings::record_warnings.
	overview.warnings = [
		{
			id: `preview-warning-${level}`,
			resource: resource.resource,
			threshold: level,
			episode: 0,
			title: `${currentTier}: Hosted AI usage ${level === 100 ? "allowance reached" : `${level}% used`}`,
			description: `Your monthly allowance renews ${periodEnd}. View usage to compare plans and ways to keep building. Local execution stays free; your own models use no hosted AI allowance.`,
		},
	];
}
const operations: QuotaOperation[] = [
	{
		id: "example-hosted-completed",
		appId: "invoice-review",
		modelId: "hosted-small",
		provider: "Hosted provider",
		kind: "chat",
		fundingClass: "hosted",
		executionMode: "realtime",
		status: "completed",
		createdAt: "2026-09-13T11:42:08Z",
		used: { runtimeMs: 8800, aiCostMicros: 4200, aiCalls: 1, cloudStarts: 1 },
		reserved: { runtimeMs: 0, aiCostMicros: 0, aiCalls: 0, cloudStarts: 0 },
	},
	{
		id: "example-provider-pending",
		appId: "research-assistant",
		modelId: "hosted-reasoning",
		provider: "Hosted provider",
		kind: "chat",
		fundingClass: "hosted",
		executionMode: "async",
		status: "unknown",
		createdAt: "2026-09-13T11:40:00Z",
		used: { runtimeMs: 29000, aiCostMicros: 0, aiCalls: 1, cloudStarts: 1 },
		reserved: {
			runtimeMs: 60000,
			aiCostMicros: 120000,
			aiCalls: 0,
			cloudStarts: 0,
		},
	},
	{
		id: "example-own-model",
		appId: "invoice-review",
		modelId: "own-provider",
		provider: "Your provider",
		kind: "chat",
		fundingClass: "byok",
		executionMode: "realtime",
		status: "completed",
		createdAt: "2026-09-13T11:30:00Z",
		used: { runtimeMs: 5400, aiCostMicros: 0, aiCalls: 0, cloudStarts: 1 },
		reserved: { runtimeMs: 0, aiCostMicros: 0, aiCalls: 0, cloudStarts: 0 },
	},
	{
		id: "example-embeddings",
		appId: "knowledge-search",
		modelId: "embedding-model",
		provider: "Internal embeddings",
		kind: "embedding",
		fundingClass: "hosted",
		executionMode: "async",
		status: "completed",
		createdAt: "2026-09-13T11:20:00Z",
		used: { runtimeMs: 18500, aiCostMicros: 680, aiCalls: 1, cloudStarts: 1 },
		reserved: { runtimeMs: 0, aiCostMicros: 0, aiCalls: 0, cloudStarts: 0 },
	},
];
const reasons: Record<string, string> = {
	"ai-budget": "hosted_ai_cost_micros",
	runtime: "cloud_runtime_ms",
	"ai-calls": "hosted_ai_calls",
	executions: "cloud_starts",
	concurrency: "concurrent_cloud_executions",
	storage: "storage_bytes",
	"project-limit": "projects",
	"model-tier": "hosted_model_access",
};

function Fixture() {
	const original = useBackend();
	const [ready, setReady] = useState(false);
	useEffect(() => {
		useBackendStore.getState().setBackend({
			...original,
			userState: {
				...original.userState,
				getProfile: async function getProfile() {
					return { hub: location.origin, secure: false };
				},
				getSettingsProfile: async function getSettingsProfile() {
					return { hub_profile: { hub: location.origin, secure: false } };
				},
				getPricing: async function getPricing() {
					if (scenario === "pricing-error")
						throw new Error("Simulated pricing outage");
					return pricing;
				},
				getQuotaUsage: async function getQuotaUsage() {
					if (scenario === "error") throw new Error("Simulated usage outage");
					return overview;
				},
				getQuotaOperations: async function getQuotaOperations() {
					return { items: scenario === "empty" ? [] : operations };
				},
				getQuotaOperationDetail: async function getQuotaOperationDetail(
					id: string,
				) {
					const operation =
						operations.find((item) => item.id === id) ?? operations[0];
					if (id === "example-provider-pending")
						return { ...operation, operationId: operation.id, estimated: true };
					return {
						...operation,
						operationId: operation.id,
						servingCostEstimated: true,
						inputTokens: id === "example-embeddings" ? null : 1640,
						outputTokens: id === "example-embeddings" ? null : 480,
						embeddingTokens: id === "example-embeddings" ? 8192 : 0,
						inputBytes: id === "example-embeddings" ? 8192 : undefined,
						providerReportedWords:
							id === "example-embeddings" ? 1370 : undefined,
						meteringBasis:
							id === "example-embeddings" ? "input_bytes" : "provider_tokens",
						tokenCountEstimated: id === "example-embeddings",
						providerCostMicroUsd: id === "example-embeddings" ? 550 : 4300,
						providerFundingCostMicroUsd: 30,
						servingCostMicroUsd: 260,
						costMicroEur: operation.used.aiCostMicros,
						estimated: id === "example-embeddings",
						rateVersion: "example-rate-2026-09",
						usdMicroPerEur: 1100000,
						providerRequestId: `sample-${id}`,
					};
				},
			},
			apiState: {
				...original.apiState,
				get: async function get() {
					return config.features;
				},
			},
			appState: {
				...original.appState,
				getAppMeta: async function getAppMeta(id: string) {
					return { name: apps[id] ?? id };
				},
			},
			bitState: {
				...original.bitState,
				getBit: async function getBit(id: string) {
					return { meta: { en: { name: models[id] ?? id } } };
				},
			},
		} as unknown as typeof original);
		setReady(true);
	}, []);
	useEffect(() => {
		if (!ready || view !== "upgrade") return;
		const reason = (params.get("reason") ?? "ai-budget") as UpgradeReason;
		const resource = reasons[reason];
		const existing = overview.resources.find(
			(item) => item.resource === resource,
		);
		const quota: QuotaDetail | undefined = resource
			? {
					resource,
					scope: "user",
					payerId: scenario.startsWith("shared")
						? "example-app-owner"
						: "preview-user",
					plan: currentTier,
					used: existing?.limit ?? 0,
					reserved: 0,
					limit: existing?.limit ?? 0,
					unit: existing?.unit ?? "access",
					periodEnd: [
						"storage_bytes",
						"projects",
						"concurrent_cloud_executions",
						"hosted_model_access",
					].includes(resource)
						? undefined
						: periodEnd,
					requiredModelTier:
						resource === "hosted_model_access"
							? (params.get("required") ?? "PRO")
							: undefined,
				}
			: undefined;
		openUpgradeDialog({
			reason,
			quota,
			message:
				resource === "hosted_model_access"
					? quota?.requiredModelTier === "ENTERPRISE"
						? "This hosted model requires an Enterprise agreement."
						: "This hosted model requires Pro or a plan that includes Pro models."
					: quota
						? `${quotaLabels[resource]}: ${formatQuota(quota.used, resource)} of ${formatQuota(quota.limit, resource)} used.`
						: undefined,
		});
	}, [ready]);
	if (!ready) return null;
	return (
		<div
			className="min-h-screen bg-background text-foreground"
			data-preview-ready
		>
			<div className="border-b px-6 py-2 text-xs text-muted-foreground">
				Flow-Like component preview · Sample usage data ·{" "}
				{theme === "dark" ? "Dark" : "Light"} theme
			</div>
			{view === "subscription" ? (
				<SubscriptionPage
					pricing={pricing}
					onUpgrade={noop}
					onManageBilling={noop}
				/>
			) : view === "cards" ? (
				<main className="mx-auto max-w-7xl p-6 space-y-6">
					<h1 className="text-3xl font-bold">Choose your plan</h1>
					<div className="grid gap-6 md:grid-cols-2 xl:grid-cols-4 pt-4">
						{["FREE", "PREMIUM", "PRO", "MAX"].map((key) => (
							<TierCard
								key={key}
								tierKey={key}
								tier={{
									...pricing.tiers[key],
									price: pricing.tiers[key].prices?.find(
										(price) =>
											price.interval === (params.get("interval") ?? "month"),
									),
								}}
								currentTier={currentTier}
								onUpgrade={noop}
								onManageBilling={noop}
							/>
						))}
					</div>
					<FreeUseTips />
				</main>
			) : view === "details" ? (
				<main className="max-w-2xl mx-auto p-6">
					<div className="rounded-xl border bg-card p-6">
						<h1 className="text-xl font-semibold">
							{scenario === "byok"
								? "Your connected model"
								: scenario === "embedding"
									? "Document embeddings"
									: scenario === "pending"
										? "Pending provider usage"
										: "Hosted AI operation"}
						</h1>
						<p className="text-sm text-muted-foreground mb-4">
							Example operation from Invoice Review
						</p>
						<UsageOperationDetails
							id={
								scenario === "byok"
									? "example-own-model"
									: scenario === "embedding"
										? "example-embeddings"
										: scenario === "pending"
											? "example-provider-pending"
											: "example-hosted-completed"
							}
						/>
					</div>
				</main>
			) : view === "history" ? (
				<main className="max-w-7xl mx-auto p-6 space-y-6">
					<h1 className="text-2xl font-semibold">Usage history</h1>
					<UsageOperations />
				</main>
			) : (
				<main className="max-w-6xl mx-auto p-6 space-y-6">
					<h1 className="text-3xl font-bold">Usage and allowances</h1>
					<UsageOverview />
				</main>
			)}
			<GlobalUpgradeDialog />
			<Toaster theme={theme} duration={Infinity} />
		</div>
	);
}
const client = new QueryClient({
	defaultOptions: {
		queries: { retry: false, staleTime: 30000, refetchOnWindowFocus: false },
	},
});
createRoot(document.getElementById("root")!).render(
	<QueryClientProvider client={client}>
		<AuthContext.Provider
			value={
				{
					isAuthenticated: true,
					user: { profile: { sub: "preview-user" } },
				} as AuthContextProps
			}
		>
			<TooltipProvider>
				<Fixture />
			</TooltipProvider>
		</AuthContext.Provider>
	</QueryClientProvider>,
);
