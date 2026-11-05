import { afterAll, afterEach, beforeAll, expect, mock, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { type ReactNode, act, createElement } from "react";
import {
	EMPTY_MONTHLY_LIMITS,
	type UsageBudgetView,
} from "../../lib/monthly-usage";
import type { IProfile } from "../../types";
import {
	fakeI18n,
	installDomGlobals,
	restoreDomGlobals,
	settle,
	translate,
	domWindow as window,
} from "../bots-tab/testing/dom-harness";

// Dialog primitives choose their layout effect at import time, including when
// another test file has already restored the shared harness globals.
installDomGlobals();

const profile = { id: "profile", hub: "https://hub.example" } as IProfile;
const budget: UsageBudgetView = {
	scope: "bot",
	subjectId: "bot-1",
	groupId: null,
	name: "Scout",
	ownerLimits: EMPTY_MONTHLY_LIMITS,
	adminLimits: { ...EMPTY_MONTHLY_LIMITS, aiCalls: 500 },
	defaultLimits: { ...EMPTY_MONTHLY_LIMITS, aiCalls: 100 },
	effectiveLimits: { ...EMPTY_MONTHLY_LIMITS, aiCalls: 500 },
	periodStart: "2026-11-01T00:00:00Z",
	periodEnd: "2026-12-01T00:00:00Z",
	used: {
		aiCalls: 10,
		aiCostMicros: 250_000,
		runtimeMs: 12_000,
		cloudStarts: 0,
	},
	reserved: { aiCalls: 1, aiCostMicros: 50_000, runtimeMs: 0, cloudStarts: 0 },
	inputTokens: 20,
	outputTokens: 10,
	embeddingTokens: 5,
	models: [],
	groups: [],
};
const group: UsageBudgetView = {
	...budget,
	scope: "bot_group",
	groupId: "room-1",
	name: "Launch",
};
const api = {
	get: mock(async (_profile: IProfile, path: string) => {
		if (path === "admin/usage/defaults")
			return { apps: EMPTY_MONTHLY_LIMITS, bots: EMPTY_MONTHLY_LIMITS };
		if (path.startsWith("admin/usage/bots?"))
			return { items: [{ ...budget, groups: [group] }], total: 1 };
		if (path === "bots/bot-1/usage/rooms/room-1") return group;
		if (path === "apps/app-1/usage/budget")
			return { ...budget, scope: "app", subjectId: "app-1" };
		throw new Error(`Unexpected path ${path}`);
	}),
	put: mock(async (_profile: IProfile, _path: string, _value: unknown) => ({})),
	fetch: mock(
		async (_profile: IProfile, _path: string, _options: RequestInit) => ({}),
	),
};
const actual = {
	locales: { ...(await import("@flow-like/locales")) },
	backend: { ...(await import("../../state/backend-state")) },
	auth: { ...(await import("react-oidc-context")) },
};
mock.module("@flow-like/locales", () => ({
	...actual.locales,
	useTranslation: () => ({ t: translate, i18n: fakeI18n }),
}));
mock.module("../../state/backend-state", () => ({
	...actual.backend,
	useBackend: () => ({
		profile,
		apiState: api,
		userState: { getProfile: async () => profile },
	}),
}));
mock.module("react-oidc-context", () => ({
	...actual.auth,
	useAuth: () => ({ user: { profile: { sub: "owner" } } }),
}));
const { createRoot } = await import("react-dom/client");
const { BotUsageButton } = await import("../bots-tab/bot-usage");
const { EnterpriseUsageBudgets } = await import(
	"../pages/admin/enterprise-usage-budgets"
);
const { AppUsageBudget } = await import("./app-usage-budget");
let root: ReturnType<typeof createRoot> | undefined;
let client: QueryClient;
let container: HTMLElement;

beforeAll(() => installDomGlobals());
afterEach(async () => {
	if (root) await act(async () => root?.unmount());
	root = undefined;
	client?.clear();
	container?.remove();
	api.get.mockClear();
	api.put.mockClear();
	api.fetch.mockClear();
});
afterAll(() => {
	mock.module("@flow-like/locales", () => actual.locales);
	mock.module("../../state/backend-state", () => actual.backend);
	mock.module("react-oidc-context", () => actual.auth);
	restoreDomGlobals();
});

async function show(content: ReactNode) {
	container = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	root = createRoot(container);
	client = new QueryClient({
		defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
	});
	await act(async () =>
		root?.render(createElement(QueryClientProvider, { client }, content)),
	);
	await settle();
}

function button(label: string) {
	return [...window.document.querySelectorAll("button")].find(
		(element) =>
			element.textContent?.trim() === label ||
			element.getAttribute("aria-label") === label,
	);
}

test("a room member sees only that bot's group usage and cannot edit its owner limits", async () => {
	await show(
		createElement(BotUsageButton, {
			botId: "bot-1",
			name: "Scout",
			groupId: "room-1",
			canEdit: false,
		}),
	);
	expect(api.get).not.toHaveBeenCalled();
	await act(async () => button("Usage & limits")?.click());
	await settle();
	expect(api.get).toHaveBeenCalledWith(
		profile,
		"bots/bot-1/usage/rooms/room-1",
	);
	expect(window.document.body.textContent).toContain(
		"20 input, 10 output, 5 embedding tokens",
	);
	expect(window.document.querySelectorAll("input")).toHaveLength(0);
	expect(api.put).not.toHaveBeenCalled();
});

test("an administrator can restore the default using an explicit JSON null body", async () => {
	await show(
		createElement(EnterpriseUsageBudgets, { profile, accountId: "owner" }),
	);
	expect(window.document.body.textContent).toContain("Bot and group usage");
	expect(window.document.body.textContent).toContain("Launch");
	await act(async () => button("Use enterprise default")?.click());
	await settle();
	expect(api.fetch).toHaveBeenCalledWith(
		profile,
		"admin/usage/bots/bot-1/limits",
		{ method: "PUT", body: "null" },
	);
});

test("deleted bots retain visible usage without editable enterprise overrides", async () => {
	budget.archived = true;
	group.archived = true;
	try {
		await show(
			createElement(EnterpriseUsageBudgets, { profile, accountId: "owner" }),
		);
		expect(window.document.body.textContent).toContain("Deleted bot");
		expect(window.document.body.textContent).toContain(
			"20 input, 10 output, 5 embedding tokens",
		);
		expect(window.document.body.textContent).not.toContain(
			"Approve a different enterprise ceiling",
		);
		expect(button("Use enterprise default")).toBeUndefined();
	} finally {
		budget.archived = false;
		group.archived = false;
	}
});

test("an app owner saves its own budget through the owner endpoint", async () => {
	await show(createElement(AppUsageBudget, { appId: "app-1", canEdit: true }));
	await act(async () => button("Save monthly limits")?.click());
	await settle();
	expect(api.put).toHaveBeenCalledWith(
		profile,
		"apps/app-1/usage/budget/limits",
		EMPTY_MONTHLY_LIMITS,
	);
	expect(api.fetch).not.toHaveBeenCalled();
});
