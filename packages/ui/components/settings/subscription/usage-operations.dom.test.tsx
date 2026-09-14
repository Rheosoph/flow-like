import { afterAll, expect, mock, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import type { QuotaOperation } from "../../../lib/quota";

let items: QuotaOperation[] = [
	{
		id: "byok-operation",
		kind: "execution",
		fundingClass: "byok",
		status: "running",
		executionMode: "async",
		createdAt: "2026-09-01T12:00:00Z",
		used: { runtimeMs: 60_000, aiCalls: 0, aiCostMicros: 0, cloudStarts: 1 },
		reserved: {
			runtimeMs: 120_000,
			aiCalls: 3,
			aiCostMicros: 0,
			cloudStarts: 1,
		},
	},
];
mock.module("../../../hooks/use-invoke", () => ({
	useInvoke: () => ({ data: { items }, isLoading: false, isError: false }),
}));
mock.module("../../../state/backend-state", () => ({
	useBackend: () => ({ userState: { getQuotaOperations: () => {} } }),
}));
mock.module("react-oidc-context", () => ({
	useAuth: () => ({ user: { profile: { sub: "payer" } } }),
}));
mock.module("./usage-operation-details", () => ({
	UsageOperationDetails: () => null,
}));
mock.module("./use-usage-names", () => ({
	useUsageNames: () => ({
		appName: () => "Example app",
		modelName: () => "Your model",
	}),
	usageFundingLabel: () => "Your provider",
	usageStatusLabel: () => "In progress",
}));
afterAll(() => mock.restore());

test("operation history explains pending runtime, AI operations and cloud starts without adding columns", async () => {
	const { UsageOperations } = await import("./usage-operations");
	const markup = renderToStaticMarkup(<UsageOperations />);
	expect(markup).toContain("2 min pending");
	expect(markup).toContain("3 pending");
	expect(markup).toContain("1 cloud start pending");
	expect(markup.match(/<th /g)).toHaveLength(7);
	items = [
		{
			...items[0],
			reserved: { runtimeMs: 0, aiCalls: 0, aiCostMicros: 0, cloudStarts: 0 },
		},
	];
	const settled = renderToStaticMarkup(<UsageOperations />);
	expect(settled).not.toContain(" pending</span>");
});
