const now = "2026-09-05T10:30:00Z";

export function responseFor(
	path: string,
	empty: boolean,
	method: string,
	body?: unknown,
): unknown {
	const url = new URL(path, "https://fixture.invalid/");
	const route = url.pathname.slice(1);
	const count = (value: number) => (empty ? 0 : value);
	const list = <T>(items: T[]) => (empty ? [] : items);
	if (route === "info/features") return { telemetry: true, ai_act: true };
	if (route === "info/profiles") return [];
	if (route === "admin/packages/stats")
		return {
			total_packages: count(48),
			total_versions: count(126),
			total_downloads: count(28493),
			pending_review: count(4),
			active_packages: count(42),
			rejected_packages: count(2),
		};
	if (route === "admin/solutions") {
		if (url.searchParams.get("status") !== "PENDING_REVIEW")
			throw new Error("Invalid solution status filter");
		return {
			solutions: [],
			total: count(3),
			page: 1,
			limit: 1,
			hasMore: !empty,
		};
	}
	if (
		route === "admin/publication/requests" ||
		route === "admin/publication/suites"
	) {
		if (url.searchParams.get("status") !== "PENDING")
			throw new Error("Invalid publication status filter");
		return {
			requests: [],
			total: count(route.endsWith("suites") ? 2 : 7),
			page: 1,
			limit: 1,
			has_more: !empty,
		};
	}
	if (route === "admin/governance/scores/summary")
		return {
			criticalApps: count(2),
			flaggedApps: count(5),
			totalApps: count(36),
			worstApps: list([
				{
					appId: "research-assistant",
					appName: "Research assistant",
					worstScore: 3,
					security: 3,
					privacy: 6,
				},
				{
					appId: "invoice-review",
					appName: "Invoice review",
					worstScore: 4,
					security: 7,
					privacy: 4,
				},
			]),
		};
	if (route === "admin/ai-act/inventory/export")
		return list([
			{
				appId: "research-assistant",
				appName: "Research assistant",
				riskCategory: "limited",
				status: "review_required",
				conformityScore: 58,
				conformityBand: "needs_review",
				updatedAt: now,
			},
		]);
	if (route === "admin/usage/alerts")
		return {
			items: list([
				{
					id: "usage-alert-1",
					kind: "budget_warning",
					severity: "warning",
					period: "monthly",
					message: "Research assistant has used 85% of its monthly budget.",
					appId: "research-assistant",
					userId: null,
					thresholdPercent: 85,
					currentCostMicroDollars: 85000000,
					currentTokens: 482930,
					acknowledgedAt: null,
					createdAt: now,
				},
			]),
			total: count(1),
			page: 0,
			pageSize: 5,
		};
	if (route === "admin/usage/invocations")
		return { items: [], total: 0, page: 0, pageSize: 8 };
	if (route === "admin/usage/reconcile")
		return { olderThanMinutes: 30, markedUnknownUsage: 2 };
	if (route.startsWith("admin/usage/") && route.endsWith("/limits"))
		return method === "PUT" ? body : usageLimits;
	if (route === "admin/usage/overview")
		return usageOverview(empty, url.searchParams.get("period") ?? "monthly");
	if (route === "admin/telemetry/alerts/events")
		return {
			events: list([
				{
					id: "alert-1",
					ruleId: "rule-1",
					ruleName: "API latency",
					status: "triggered",
					value: 1450,
					threshold: 1000,
					message: "API latency exceeded 1 second.",
					acknowledgedAt: null,
					createdAt: now,
				},
			]),
			total: count(3),
			page: 0,
			pageSize: 1,
			unacknowledged: count(2),
		};
	if (route === "admin/telemetry/alerts")
		return {
			rules: list([
				{
					id: "rule-1",
					name: "API latency",
					metric: "latency_p95",
					comparator: "gt",
					threshold: 1000,
					mode: "threshold",
					windowMinutes: 15,
					minSamples: 6,
					enabled: true,
					notifyEmail: true,
					notifyPush: false,
					createdAt: now,
					updatedAt: now,
				},
			]),
		};
	if (route === "admin/telemetry/overview")
		return {
			hours: 24,
			totalEvents: count(12893),
			activeInstalls: count(328),
			previousTotalEvents: count(11342),
			topEvents: list([
				{ name: "workflow.executed", count: 2184, installs: 128 },
			]),
			sources: list([
				{ source: "web", count: 8328 },
				{ source: "desktop", count: 4565 },
			]),
			platforms: [],
			versions: [],
			countries: [],
		};
	if (route === "admin/telemetry/timeseries")
		return {
			bucket: "hour",
			points: list(
				Array.from({ length: 12 }, (_, i) => ({
					ts: `2026-09-05T${String(i).padStart(2, "0")}:00:00Z`,
					count: 80 + i * 12,
					installs: 12 + i,
				})),
			),
		};
	if (route === "admin/telemetry/issues")
		return { issues: [], total: 0, page: 0, pageSize: 25 };
	if (route === "admin/telemetry/release-health")
		return {
			hours: 168,
			totalSessions: count(2842),
			crashedSessions: count(3),
			crashFreeSessionRate: empty ? null : 0.9989,
			crashFreeInstallRate: empty ? null : 0.9991,
			totalInstalls: count(328),
			trend: [],
			releases: [],
		};
	if (route === "admin/telemetry/span-stats")
		return {
			operations: list([
				{
					name: "POST /api/execute",
					count: 1845,
					p50: 420,
					p95: 1450,
					errorRate: 0.003,
					totalMs: 748000,
				},
			]),
		};
	if (route === "admin/resources")
		return {
			generatedAt: now,
			cached: false,
			resources: list([
				{
					id: "database",
					kind: "database",
					label: "Primary database",
					backend: "postgres",
					status: "ok",
					latencyMs: 4.2,
					metrics: [
						{
							key: "size_bytes",
							label: "Storage",
							value: 6240000000,
							unit: "bytes",
							freshness: "live",
						},
					],
				},
				{
					id: "cache",
					kind: "cache",
					label: "Cache",
					backend: "redis",
					status: "ok",
					latencyMs: 1.2,
					metrics: [
						{
							key: "memory_bytes",
							label: "Memory",
							value: 82000000,
							unit: "bytes",
							freshness: "live",
						},
					],
				},
			]),
		};
	if (route === "admin/logs/stats")
		return {
			window_hours: 24,
			total_errors: count(14),
			server_errors: count(3),
			client_errors: count(11),
			unique_users_affected: count(8),
			unique_paths: count(4),
			previous_window_total: count(19),
			change_percent: empty ? null : -26.3,
			recent: [],
			top_codes: [],
			top_paths: [],
			top_users: [],
		};
	if (route === "admin/logs/timeseries")
		return { window_hours: 24, bucket: "hour", points: [] };
	if (route === "admin/logs/chain-status")
		return {
			signing_configured: true,
			current_kid: "fixture-key",
			total_entries: count(18563),
			signed_entries: count(18563),
			unsigned_entries: 0,
			branch_chain_count: count(8),
			last_24h_entries: count(472),
			root_chain: {
				label: "Root chain",
				entries: count(2846),
				signed: true,
				valid: true,
				fully_authenticated: true,
				kid: "fixture-key",
				last_entry_at: now,
			},
			recent_branches: [],
		};
	if (route === "admin/packages/ensure-wasm-artifacts")
		return {
			targetPlatform: "linux-x86_64",
			wasmtimeVersion: "41.0.0",
			activePackages: 42,
			checkedVersions: 126,
			skippedVersions: 0,
			alreadyPending: 0,
			jobsStarted: 3,
			alreadyAvailable: 123,
			failed: 0,
			failures: [],
		};
	throw new Error(`Unmocked fixture request: ${method} ${path}`);
}

const limitWindow = {
	costMicroDollars: 100000000,
	tokenLimit: 5000000,
	enabled: true,
	hard: false,
	warningThresholdPercent: 80,
};
const usageLimits = {
	weekly: limitWindow,
	monthly: limitWindow,
	yearly: limitWindow,
};

function usageOverview(empty: boolean, period: string) {
	const n = (value: number) => (empty ? 0 : value);
	const totals = {
		llmPrice: n(294000000),
		embeddingPrice: n(18300000),
		totalPrice: n(312300000),
		llmTokens: n(1248000),
		embeddingTokens: n(893000),
		totalTokens: n(2141000),
		llmInvocations: n(4829),
		embeddingInvocations: n(1238),
		executions: n(12839),
		executionMicroseconds: n(942810000),
		averageExecutionMs: empty ? null : 73.4,
	};
	return {
		period,
		startedAt: "2026-09-01T00:00:00Z",
		totals,
		userStats: {
			totalUsers: n(842),
			newUsersToday: n(8),
			newUsersWeekly: n(42),
			newUsersMonthly: n(118),
			activeUsersDaily: n(83),
			activeUsersWeekly: n(218),
			activeUsersMonthly: n(486),
			activeAppsDaily: n(18),
			activeAppsWeekly: n(26),
			activeAppsMonthly: n(34),
			aiUsersMonthly: n(294),
			executionUsersMonthly: n(382),
			powerUsersWeekly: n(28),
			powerUsersMonthly: n(64),
			averageCostPerActiveUser: empty ? null : 642592,
		},
		trend: empty
			? []
			: Array.from({ length: 7 }, (_, i) => ({
					bucket: `2026-09-0${i + 1}`,
					label: `Sep ${i + 1}`,
					newUsers: 8 + i * 2,
					activeUsers: 64 + i * 7,
					executions: 182 + i * 29,
					aiInvocations: 78 + i * 18,
					tokens: 148200 + i * 28700,
					cost: 12800000 + i * 3800000,
				})),
		powerUsers: [],
		users: [],
		technicalUsers: [],
		apps: empty
			? []
			: [
					{
						...totals,
						appId: "research-assistant",
						appName: "Research assistant",
						limits: usageLimits,
					},
				],
		models: empty
			? []
			: [
					{
						kind: "llm",
						modelId: "research-model",
						provider: "Hosted",
						endpoint: null,
						price: 294000000,
						tokens: 1248000,
						invocations: 4829,
						averageLatencyMs: 842,
					},
				],
	};
}
