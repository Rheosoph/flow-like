import { afterAll, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import {
	QueryClient,
	QueryClientProvider,
	QueryObserver,
} from "@tanstack/react-query";

const client = new QueryClient({
	defaultOptions: { queries: { retry: false, gcTime: Infinity } },
});
const refetchUsage = mock(async () => ({ data: overview }));
let usageError = false;
import { act, type ButtonHTMLAttributes } from "react";
import { createRoot } from "react-dom/client";
import type { QuotaOverview } from "../../../lib/quota";

let overview: QuotaOverview = {
	plan: "PREMIUM",
	payerId: "payer-ui",
	periodStart: "2026-09-01",
	periodEnd: "2026-10-01",
	updatedAt: "2026-09-13",
	resources: [
		{
			resource: "cloud_runtime_ms",
			used: 54_000_000,
			reserved: 0,
			limit: 72_000_000,
			remaining: 18_000_000,
			unit: "milliseconds",
			threshold: 75,
		},
	],
	usage: Array.from({ length: 25 }, (_, index) => ({
		day: "2026-09-13",
		appId: `app-${index}`,
		modelId: `model-${index % 2}`,
		provider: "example",
		fundingClass: "hosted",
		executionMode: "realtime",
		runtimeMs: 1000,
		aiCostMicros: 10000,
		aiCalls: 1,
		cloudStarts: 1,
	})),
};
mock.module("../../../hooks/use-invoke", () => ({
	useInvoke: () => ({
		data: overview,
		isLoading: false,
		isError: usageError,
		refetch: refetchUsage,
	}),
}));
mock.module("../../../state/backend-state", () => ({
	useBackend: () => ({
		userState: {
			getQuotaUsage: () => {},
			getQuotaOperations: () => {},
			getQuotaOperationDetail: () => {},
		},
	}),
}));
mock.module("react-oidc-context", () => ({
	useAuth: () => ({ user: { profile: { sub: "payer-ui" } } }),
}));
const upgrade = mock(() => {});
mock.module("../../../state/upgrade-dialog-state", () => ({
	openUpgradeDialog: upgrade,
}));
mock.module("./use-usage-names", () => ({
	usageFundingLabel: (value: string) =>
		value === "hosted" ? "Flow-Like allowance" : value,
	useUsageNames: () => ({
		appName: (id: string) => `App ${id}`,
		modelName: (id: string) => `Model ${id}`,
	}),
}));
mock.module("./usage-operations", () => ({ UsageOperations: () => null }));
mock.module("../../ui/button", () => ({
	Button: (props: ButtonHTMLAttributes<HTMLButtonElement>) => (
		<button {...props} />
	),
}));
afterAll(() => mock.restore());

test("usage overview paginates, filters and preserves readable quota context", async () => {
	const window = new Window();
	Object.assign(window, { SyntaxError, TypeError, Error });
	Object.assign(globalThis, {
		window,
		document: window.document,
		HTMLElement: window.HTMLElement,
		Node: window.Node,
		Event: window.Event,
		navigator: window.navigator,
		IS_REACT_ACT_ENVIRONMENT: true,
	});
	const { UsageOverview } = await import("./usage-overview");
	const container = window.document.createElement("div");
	window.document.body.append(container);
	const root = createRoot(container as unknown as HTMLElement);
	await act(async () =>
		root.render(
			<QueryClientProvider client={client}>
				<UsageOverview />
			</QueryClientProvider>,
		),
	);
	expect(container.textContent).toContain("Near limit");
	expect(
		container
			.querySelector('[role="progressbar"]')
			?.getAttribute("aria-valuenow"),
	).toBe("75");
	expect(
		container
			.querySelector('[role="progressbar"]')
			?.getAttribute("aria-valuetext"),
	).toContain("5 h available of 20 h");
	expect(container.textContent).toContain("15 h");
	expect(container.getElementsByTagName("tbody")[0].children.length).toBe(20);
	const next = Array.from(container.getElementsByTagName("button")).find(
		(button) => button.textContent === "Next",
	)!;
	await act(async () => next.click());
	expect(container.getElementsByTagName("tbody")[0].children.length).toBe(5);
	const select = container.getElementsByTagName("select")[0];
	await act(async () => {
		select.value = "app-2";
		select.dispatchEvent(new window.Event("change", { bubbles: true }));
	});
	expect(container.getElementsByTagName("tbody")[0].children.length).toBe(1);
	expect(container.getElementsByTagName("tbody")[0].textContent).toContain(
		"App app-2",
	);
	expect(container.textContent).toContain("Export filtered CSV");
	await act(async () => root.unmount());
	window.happyDOM.abort();
});

test("allowances separate pending capacity, prioritize costs and handle zero and unlimited limits", async () => {
	const window = new Window();
	Object.assign(window, { SyntaxError, TypeError, Error });
	Object.assign(globalThis, {
		window,
		document: window.document,
		HTMLElement: window.HTMLElement,
		Node: window.Node,
		Event: window.Event,
		navigator: window.navigator,
		IS_REACT_ACT_ENVIRONMENT: true,
	});
	const resource = (
		key: string,
		used: number,
		reserved: number,
		limit: number,
	) => ({
		resource: key,
		used,
		reserved,
		limit,
		remaining: limit < 0 ? null : Math.max(0, limit - used - reserved),
		unit: "count",
		threshold: 0,
	});
	overview = {
		...overview,
		usage: [],
		resources: [
			resource("projects", 500, 0, -1),
			resource("cloud_starts", 0, 0, 0),
			resource("cloud_runtime_ms", 30_000, 0, 60_000),
			resource("storage_bytes", 1_000_000_000, 0, 1_000_000_000),
			resource("hosted_ai_cost_micros", 500_000, 400_000, 1_000_000),
		],
	};
	const { UsageOverview } = await import("./usage-overview");
	const container = window.document.createElement("div");
	window.document.body.append(container);
	const root = createRoot(container as unknown as HTMLElement);
	await act(async () =>
		root.render(
			<QueryClientProvider client={client}>
				<UsageOverview />
			</QueryClientProvider>,
		),
	);
	const cards = Array.from(container.querySelectorAll("article"));
	expect(
		cards.slice(0, 3).map((card) => card.getAttribute("aria-label")),
	).toEqual(["Hosted AI usage", "Cloud runtime", "Cloud storage"]);
	const ai = cards[0];
	expect(
		ai.querySelector('[role="progressbar"]')?.getAttribute("aria-valuenow"),
	).toBe("90");
	expect(
		ai.querySelector('[data-usage-segment="settled"]')?.getAttribute("style"),
	).toContain("50%");
	expect(
		ai.querySelector('[data-usage-segment="pending"]')?.getAttribute("style"),
	).toContain("40%");
	expect(
		ai.querySelector('[role="progressbar"]')?.getAttribute("aria-valuetext"),
	).toContain("€0.40 pending, €0.10 available");
	expect(ai.textContent).toContain("Near limit");
	expect(
		ai.querySelector('[data-usage-segment="settled"]')?.className,
	).toContain("amber");
	expect(cards[2].textContent).toContain("Full");
	expect(cards[3].textContent).toContain("unlimited");
	expect(cards[3].querySelector('[role="progressbar"]')).toBeNull();
	expect(cards[4].textContent).toContain("Not included");
	expect(
		cards[4]
			.querySelector('[role="progressbar"]')
			?.getAttribute("aria-valuenow"),
	).toBe("0");
	expect(
		cards[4]
			.querySelector('[role="progressbar"]')
			?.getAttribute("aria-valuetext"),
	).toContain("No allowance included");
	const compare = Array.from(container.querySelectorAll("button")).filter(
		(button) => button.textContent?.includes("Compare plans"),
	);
	expect(compare).toHaveLength(1);
	await act(async () => compare[0].click());
	expect(upgrade).toHaveBeenLastCalledWith(
		expect.objectContaining({
			quota: expect.objectContaining({
				payerId: "payer-ui",
				plan: "PREMIUM",
				resource: "storage_bytes",
				limit: 1_000_000_000,
				periodEnd: undefined,
			}),
		}),
	);
	expect(container.textContent).toContain("No recorded cloud usage");
	overview = {
		...overview,
		resources: [
			resource("cloud_starts", 0, 0, 0),
			resource("projects", 0, 0, -1),
		],
	};
	await act(async () =>
		root.render(
			<QueryClientProvider client={client}>
				<UsageOverview />
			</QueryClientProvider>,
		),
	);
	expect(container.textContent).toContain("Not included");
	expect(container.textContent).not.toContain("Compare plans");
	await act(async () => root.unmount());
	window.happyDOM.abort();
});

test("refresh updates active operation queries and invalidates only this account's cached history and details", async () => {
	const window = new Window();
	Object.assign(window, { SyntaxError, TypeError, Error });
	Object.assign(globalThis, {
		window,
		document: window.document,
		HTMLElement: window.HTMLElement,
		Node: window.Node,
		Event: window.Event,
		navigator: window.navigator,
		IS_REACT_ACT_ENVIRONMENT: true,
	});
	client.clear();
	const activeKey = ["getQuotaOperations", null, "payer-ui"];
	const cachedKeys = [
		["getQuotaOperations", "older-page", "payer-ui"],
		["getQuotaOperationDetail", "operation-1", "payer-ui"],
	];
	const unrelatedKeys = [
		["getQuotaOperations", null, "another-payer"],
		["getQuotaOperationDetail", "operation-1", "another-payer"],
		["getAppMeta", "app-1", "payer-ui"],
	];
	for (const key of [activeKey, ...cachedKeys, ...unrelatedKeys])
		client.setQueryData(key, { status: "pending" });
	const reloadHistory = mock(async () => ({ status: "completed" }));
	const observer = new QueryObserver(client, {
		queryKey: activeKey,
		queryFn: reloadHistory,
		staleTime: Infinity,
	});
	const unsubscribe = observer.subscribe(() => {});
	const { UsageOverview } = await import("./usage-overview");
	const container = window.document.createElement("div");
	window.document.body.append(container);
	const root = createRoot(container as unknown as HTMLElement);
	await act(async () =>
		root.render(
			<QueryClientProvider client={client}>
				<UsageOverview />
			</QueryClientProvider>,
		),
	);
	const refresh = Array.from(container.querySelectorAll("button")).find(
		(button) => button.textContent === "Refresh usage",
	)!;
	await act(async () => {
		refresh.click();
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
	expect(refetchUsage).toHaveBeenCalledTimes(1);
	expect(reloadHistory).toHaveBeenCalledTimes(1);
	expect(client.getQueryData<{ status: string }>(activeKey)).toEqual({
		status: "completed",
	});
	for (const key of cachedKeys)
		expect(client.getQueryState(key)?.isInvalidated).toBe(true);
	for (const key of unrelatedKeys)
		expect(client.getQueryState(key)?.isInvalidated).toBe(false);
	usageError = true;
	await act(async () =>
		root.render(
			<QueryClientProvider client={client}>
				<UsageOverview />
			</QueryClientProvider>,
		),
	);
	expect(container.querySelector('[role="status"]')?.textContent).toContain(
		"Some usage could not be refreshed",
	);
	expect(container.querySelector('[role="status"]')?.textContent).toContain(
		"last updated",
	);
	await act(async () => root.unmount());
	unsubscribe();
	client.clear();
	window.happyDOM.abort();
});
