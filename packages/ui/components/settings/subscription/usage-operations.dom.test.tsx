import { afterAll, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
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
// bun keeps a module mock for every later file in the process, so each mocked module is
// captured first and put back in afterAll. Radix picks its layout effect when first imported,
// so the real modules load under a document.
const documentDescriptor = Object.getOwnPropertyDescriptor(
	globalThis,
	"document",
);
Object.assign(globalThis, { document: new Window().document });
const actual = {
	invoke: { ...(await import("../../../hooks/use-invoke")) },
	backendState: { ...(await import("../../../state/backend-state")) },
	oidc: { ...(await import("react-oidc-context")) },
	operationDetails: { ...(await import("./usage-operation-details")) },
	usageNames: { ...(await import("./use-usage-names")) },
};
if (documentDescriptor)
	Object.defineProperty(globalThis, "document", documentDescriptor);
else Reflect.deleteProperty(globalThis, "document");
mock.module("../../../hooks/use-invoke", () => ({
	...actual.invoke,
	useInvoke: () => ({ data: { items }, isLoading: false, isError: false }),
}));
mock.module("../../../state/backend-state", () => ({
	...actual.backendState,
	useBackend: () => ({ userState: { getQuotaOperations: () => {} } }),
}));
mock.module("react-oidc-context", () => ({
	...actual.oidc,
	useAuth: () => ({ user: { profile: { sub: "payer" } } }),
}));
mock.module("./usage-operation-details", () => ({
	...actual.operationDetails,
	UsageOperationDetails: () => null,
}));
mock.module("./use-usage-names", () => ({
	...actual.usageNames,
	useUsageNames: () => ({
		appName: () => "Example app",
		modelName: () => "Your model",
	}),
	usageFundingLabel: () => "Your provider",
	usageStatusLabel: () => "In progress",
}));
afterAll(() => {
	mock.restore();
	mock.module("../../../hooks/use-invoke", () => actual.invoke);
	mock.module("../../../state/backend-state", () => actual.backendState);
	mock.module("react-oidc-context", () => actual.oidc);
	mock.module("./usage-operation-details", () => actual.operationDetails);
	mock.module("./use-usage-names", () => actual.usageNames);
});

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
