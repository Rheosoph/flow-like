import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AdminDashboardPage } from "../../packages/ui/components/pages/admin/admin-dashboard-page";
import { responseFor } from "./responses";

const fixture = vi.hoisted(() => ({
	permission: 1,
	empty: false,
	failPath: "",
	calls: [] as string[],
	get: vi.fn(),
}));

vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({
		t: (key: string, fallback?: string, values?: Record<string, unknown>) =>
			(fallback ?? key).replace(/\{\{(\w+)\}\}/g, (_, name) =>
				String(values?.[name] ?? ""),
			),
	}),
	i18n: { language: "en" },
}));

vi.mock("../../packages/ui/state/backend-state", () => ({
	useBackend: () => ({
		userState: {
			getProfile: async function getProfile() {
				return { id: "fixture-profile", hub: "https://fixture.invalid" };
			},
			getInfo: async function getInfo() {
				return { id: "fixture-user", permission: fixture.permission };
			},
			getSettingsProfile: async function getSettingsProfile() {
				return {
					hub_profile: {
						id: "fixture-profile",
						hub: "https://fixture.invalid",
					},
				};
			},
		},
		apiState: { get: fixture.get },
	}),
}));

// The widget barrel also exports the workflow editor. Keep real primitives while
// excluding that unrelated application graph from this dashboard component test.
vi.mock(
	"../../packages/ui/lib",
	async () => import("../../packages/ui/lib/utils"),
);
vi.mock("../../packages/ui/components/ui", async () => ({
	...(await import("../../packages/ui/components/ui/badge")),
	...(await import("../../packages/ui/components/ui/button")),
	...(await import("../../packages/ui/components/ui/card")),
	...(await import("../../packages/ui/components/ui/chart")),
	...(await import("../../packages/ui/components/ui/input")),
	...(await import("../../packages/ui/components/ui/select")),
	...(await import("../../packages/ui/components/ui/skeleton")),
	...(await import("../../packages/ui/components/ui/table")),
	...(await import("../../packages/ui/components/ui/tooltip")),
	...(await import("../../packages/ui/components/ui/relative-time")),
}));

let root: Root;
let container: HTMLDivElement;
let client: QueryClient;

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	fixture.permission = 1;
	fixture.empty = false;
	fixture.failPath = "";
	fixture.calls = [];
	fixture.get.mockReset();
	fixture.get.mockImplementation(async (_profile: unknown, path: string) => {
		fixture.calls.push(path);
		if (path.startsWith(fixture.failPath) && fixture.failPath)
			throw new Error("Service unavailable");
		return responseFor(path, fixture.empty, "GET");
	});
	client = new QueryClient({
		defaultOptions: {
			queries: {
				retry: false,
				retryDelay: 0,
				refetchOnWindowFocus: false,
				gcTime: Number.POSITIVE_INFINITY,
			},
		},
	});
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	client.clear();
	container.remove();
});

async function render(infoEnabled = true) {
	await act(async () =>
		root.render(
			<QueryClientProvider client={client}>
				<AdminDashboardPage infoEnabled={infoEnabled} />
			</QueryClientProvider>,
		),
	);
}

async function settled(assertion: () => void) {
	let lastError: unknown;
	for (let attempt = 0; attempt < 100; attempt++) {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 20));
		});
		try {
			assertion();
			return;
		} catch (error) {
			lastError = error;
		}
	}
	throw lastError;
}

function queue(title: string) {
	const row = [
		...container.querySelectorAll(
			"section[aria-labelledby='admin-attention-heading'] a",
		),
	].find((element) => element.textContent?.startsWith(title));
	if (!row) throw new Error(`Missing review queue: ${title}`);
	return row;
}

async function openTab(label: string) {
	const tab = [
		...container.querySelectorAll<HTMLButtonElement>("[role='tab']"),
	].find((element) => element.textContent === label);
	if (!tab) throw new Error(`Missing tab: ${label}`);
	await act(async () => {
		tab.dispatchEvent(
			new MouseEvent("mousedown", { bubbles: true, button: 0 }),
		);
		tab.click();
	});
}

describe("AdminDashboardPage", () => {
	it("renders Rust registry counts and combines disjoint governance risk groups", async () => {
		await render();
		await settled(() => {
			expect(queue("Package reviews").textContent).toMatch(/4$/);
			expect(queue("Governance findings").textContent).toMatch(/7$/);
			expect(queue("Governance findings").textContent).toContain(
				"2 apps have critical scores",
			);
			expect(queue("Unacknowledged alerts").textContent).toMatch(/2$/);
			expect(container.textContent).toContain("Active packages42");
			expect(container.textContent).toContain("Package downloads28,493");
		});
		const first = container.querySelector(
			"section[aria-labelledby='admin-attention-heading'] a",
		);
		expect(first?.textContent).toContain("Governance findings");
	});

	it("only fetches and links queues allowed by the user's role", async () => {
		fixture.permission = 2050;
		await render();
		await settled(() =>
			expect(queue("Solution requests").textContent).toMatch(/3$/),
		);
		expect(
			fixture.calls.filter((path) => path.startsWith("admin/")),
		).toHaveLength(4);
		expect(
			fixture.calls.some((path) =>
				/packages|telemetry|usage|resources|logs/.test(path),
			),
		).toBe(false);
		expect(container.querySelector('a[href="/admin/packages"]')).toBeNull();
		expect(container.querySelector('a[href="/admin/users"]')).toBeNull();
		expect(
			[...container.querySelectorAll("[role='tab']")].map(
				(tab) => tab.textContent,
			),
		).toEqual(["Overview", "Governance"]);
	});

	it("keeps admin API calls gated while account information is disabled", async () => {
		await render(false);
		await settled(() => expect(fixture.calls).toContain("info/features"));
		expect(fixture.calls.filter((path) => path.startsWith("admin/"))).toEqual(
			[],
		);
		expect(container.querySelector('a[href="/admin/packages"]')).toBeNull();
	});

	it("shows an unavailable count and recovery action instead of claiming the queues are clear", async () => {
		fixture.empty = true;
		fixture.failPath = "admin/packages/stats";
		await render();
		await settled(() =>
			expect(queue("Package reviews").textContent).toContain("Unavailable"),
		);
		expect(container.textContent).not.toContain("You're all caught up");
		const retry = container.querySelector<HTMLButtonElement>(
			'button[aria-label="Retry: Package reviews"]',
		);
		expect(retry).not.toBeNull();
		fixture.failPath = "";
		await act(async () => retry?.click());
		await settled(() =>
			expect(container.textContent).toContain("You're all caught up"),
		);
	});

	it("treats a malformed successful count as unavailable", async () => {
		fixture.empty = true;
		fixture.get.mockImplementation(async (_profile: unknown, path: string) =>
			path === "admin/packages/stats"
				? { pending_review: "0" }
				: responseFor(path, true, "GET"),
		);
		await render();
		await settled(() =>
			expect(queue("Package reviews").textContent).toContain("Unavailable"),
		);
		expect(container.textContent).not.toContain("You're all caught up");
	});

	it("defers usage and system queries until their tabs open", async () => {
		fixture.empty = true;
		await render();
		await settled(() =>
			expect(container.textContent).toContain("You're all caught up"),
		);
		expect(fixture.calls.some((path) => path.startsWith("admin/usage/"))).toBe(
			false,
		);
		expect(fixture.calls).not.toContain("admin/resources");
		expect(fixture.calls.some((path) => path.startsWith("admin/logs/"))).toBe(
			false,
		);
		await openTab("Usage & limits");
		await settled(() =>
			expect(
				fixture.calls.some((path) => path.startsWith("admin/usage/overview")),
				`${container.textContent?.slice(-600)} Calls: ${fixture.calls.join(", ")}`,
			).toBe(true),
		);
		expect(fixture.calls).not.toContain("admin/resources");
		await openTab("System health");
		await settled(() => {
			expect(fixture.calls).toContain("admin/resources");
			expect(fixture.calls).toContain("admin/logs/chain-status");
			expect(client.isFetching()).toBe(0);
		});
	});

	it("filters navigation by descriptions and restores the tool list when cleared", async () => {
		await render();
		await settled(() =>
			expect(container.querySelector('a[href="/admin/sinks"]')).not.toBeNull(),
		);
		const input = container.querySelector<HTMLInputElement>(
			'input[aria-label="Find an admin tool"]',
		);
		if (!input) throw new Error("Missing admin tool search");
		await act(async () => {
			Object.getOwnPropertyDescriptor(
				HTMLInputElement.prototype,
				"value",
			)?.set?.call(input, "credentials");
			input.dispatchEvent(new Event("input", { bubbles: true }));
		});
		const navigation = container.querySelector(
			"section[aria-labelledby='admin-manage-heading']",
		);
		expect(navigation?.querySelectorAll("a")).toHaveLength(1);
		expect(navigation?.querySelector("a")?.getAttribute("href")).toBe(
			"/admin/sinks",
		);
		await act(async () =>
			container
				.querySelector<HTMLButtonElement>('button[aria-label="Clear search"]')
				?.click(),
		);
		expect(navigation?.querySelectorAll("a").length).toBeGreaterThan(10);
	});
});
