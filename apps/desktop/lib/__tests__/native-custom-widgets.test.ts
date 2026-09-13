import {
	type NativeCustomWidget,
	type NativeWidgetDefinition,
	type NativeWidgetPageDefinition,
	nativeWidgetShell,
	newNativeWidgetDefinition,
	saveNativeWidgetDefinitions,
} from "@flow-like/flow-like-ui/lib/native-widget";
import type { NativeWidgetPageCaptureDetail } from "@flow-like/flow-like-ui/lib/native-widget-page";
import type { IBackendState } from "@flow-like/flow-like-ui/state/backend-state";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	loadChart: vi.fn(),
	loadPage: vi.fn(),
	resolveMedia: vi.fn(),
	assertApp: vi.fn(),
	icons: vi.fn(),
	disposeIcons: vi.fn(),
}));
vi.mock("@flow-like/flow-like-ui/lib/native-widget-data", () => ({
	loadNativeWidgetChart: mocks.loadChart,
	assertNativeWidgetApp: mocks.assertApp,
	assertNativeWidgetActive: (signal?: AbortSignal) => {
		if (signal?.aborted) throw new DOMException("Cancelled", "AbortError");
	},
}));
vi.mock("@flow-like/flow-like-ui/lib/native-widget-page", () => ({
	NATIVE_WIDGET_PAGE_CAPTURE: "flow-like:native-widget-page-capture",
	loadNativeWidgetPage: mocks.loadPage,
	resolveNativeWidgetPageMedia: mocks.resolveMedia,
}));
vi.mock("../native-notification-icons", () => ({
	createNativeNotificationIconResolver: () => ({
		resolve: mocks.icons,
		dispose: mocks.disposeIcons,
	}),
}));

import {
	createNativeCustomWidgetPublisher,
	materializeNativeWidgetPage,
	readNativeCustomWidgetCache,
} from "../native-custom-widgets";

const scope = "account-a";
const backend = { pageState: {} } as IBackendState;
const disposals: Array<() => void> = [];
const definition = (id = "page-widget"): NativeWidgetPageDefinition => ({
	id,
	title: "Orders",
	kind: "page",
	appId: "app",
	path: "/orders",
	queryParams: [],
	accent: "orange",
	refreshMinutes: 30,
	updatedAt: "2026-09-13T10:00:00.000Z",
});
const projection = (
	text = "Static",
	pageRevision = "v1",
	requiresCapture = false,
) => ({
	root: { id: "root", kind: "text" as const, text },
	supported: true,
	warnings: [] as string[],
	media: [] as Array<{ nodeId: string; source: string }>,
	pageId: "page",
	pageRevision,
	canonicalRoute: "/orders",
	title: "Orders",
	requiresCapture,
});
function deferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (error: unknown) => void;
	const promise = new Promise<T>((done, fail) => {
		resolve = done;
		reject = fail;
	});
	return { promise, resolve, reject };
}
async function flush() {
	for (let i = 0; i < 40; i++) await Promise.resolve();
}
function publisher(selectedScope = scope, current = () => true) {
	const publish = vi.fn(async (_widgets: NativeCustomWidget[]) => {});
	const service = createNativeCustomWidgetPublisher({
		backend,
		scope: selectedScope,
		viewerId: "viewer",
		isCurrent: current,
		publish,
	});
	disposals.push(service.dispose);
	return {
		service,
		publish,
		latest: () => publish.mock.calls.at(-1)?.[0] ?? [],
	};
}
function capture(
	text: string,
	override: Partial<NativeWidgetPageCaptureDetail> = {},
) {
	const detail: NativeWidgetPageCaptureDetail = {
		scope,
		definitionId: definition().id,
		revision: definition().updatedAt,
		capturedAt: Date.now(),
		pageId: "page",
		pageRevision: "v1",
		result: projection(text, "v1", true),
		...override,
	};
	window.dispatchEvent(
		new CustomEvent("flow-like:native-widget-page-capture", { detail }),
	);
}
function seed(
	selectedScope: string,
	widget: NativeCustomWidget,
	pageRevision = "v1",
) {
	localStorage.setItem(
		`flow-like:native-widget-content:v1:${selectedScope}`,
		JSON.stringify([
			{
				revision: definition().updatedAt,
				widget,
				pageId: "page",
				pageRevision,
				captured: true,
			},
		]),
	);
}

beforeEach(() => {
	vi.useFakeTimers();
	vi.setSystemTime(new Date("2026-09-13T10:05:00.000Z"));
	const values = new Map<string, string>();
	vi.stubGlobal("localStorage", {
		getItem: (key: string) => values.get(key) ?? null,
		setItem: (key: string, value: string) => values.set(key, value),
		removeItem: (key: string) => values.delete(key),
	});
	vi.stubGlobal("window", new EventTarget());
	vi.stubGlobal(
		"document",
		Object.assign(new EventTarget(), { visibilityState: "visible" }),
	);
	vi.stubGlobal("navigator", { onLine: true });
	mocks.loadChart.mockReset();
	mocks.loadPage.mockReset().mockResolvedValue(projection());
	mocks.resolveMedia
		.mockReset()
		.mockImplementation(async (_storage, _appId, projection) => projection);
	mocks.assertApp.mockReset().mockResolvedValue(undefined);
	mocks.icons.mockReset().mockResolvedValue({});
	mocks.disposeIcons.mockReset();
});
afterEach(() => {
	for (const dispose of disposals.splice(0)) dispose();
	vi.useRealTimers();
	vi.unstubAllGlobals();
});

describe("native custom widget publication", () => {
	it("publishes placeholders immediately and fills independent widgets as their reads finish", async () => {
		const slow = deferred<ReturnType<typeof projection>>();
		const first = definition("first");
		const second = definition("second");
		saveNativeWidgetDefinitions(scope, [first, second]);
		mocks.loadPage.mockImplementation(
			(_pageState, item: NativeWidgetDefinition) =>
				item.id === first.id
					? slow.promise
					: Promise.resolve(projection("Fast")),
		);
		const harness = publisher();
		const refreshing = harness.service.refresh();
		await flush();
		expect(
			harness.publish.mock.calls[0][0].every(
				(item) => item.state === "unavailable",
			),
		).toBe(true);
		expect(
			harness.latest().find((item) => item.id === second.id)?.page?.text,
		).toBe("Fast");
		slow.resolve(projection("Slow"));
		await refreshing;
		expect(harness.latest().map((item) => item.page?.text)).toEqual([
			"Slow",
			"Fast",
		]);
	});

	it("discards a late result from a previous account", async () => {
		saveNativeWidgetDefinitions(scope, [definition()]);
		const response = deferred<ReturnType<typeof projection>>();
		mocks.loadPage.mockReturnValue(response.promise);
		let current = true;
		const harness = publisher(scope, () => current);
		const refreshing = harness.service.refresh();
		await flush();
		current = false;
		response.resolve(projection("Private account A data"));
		await refreshing;
		expect(JSON.stringify(harness.publish.mock.calls)).not.toContain(
			"Private account A data",
		);
		expect(
			JSON.stringify(readNativeCustomWidgetCache("account-b")),
		).not.toContain("Private account A data");
	});

	it("cancels deleted or changed configurations before publishing their results", async () => {
		saveNativeWidgetDefinitions(scope, [definition()]);
		const response = deferred<ReturnType<typeof projection>>();
		mocks.loadPage.mockReturnValueOnce(response.promise);
		const harness = publisher();
		harness.service.start();
		await flush();
		saveNativeWidgetDefinitions(scope, []);
		await flush();
		response.resolve(projection("Deleted widget"));
		await flush();
		expect(harness.latest()).toEqual([]);
		expect(JSON.stringify(harness.publish.mock.calls)).not.toContain(
			"Deleted widget",
		);
	});

	it("preserves a captured page's original update time until the page or its revision changes", async () => {
		const item = definition();
		const originalDate = new Date("2026-09-13T09:00:00.000Z");
		seed(scope, {
			...nativeWidgetShell(item, originalDate),
			state: "ready",
			page: projection("Captured content").root,
		});
		saveNativeWidgetDefinitions(scope, [item]);
		mocks.loadPage.mockResolvedValue(
			projection("Initial placeholder", "v1", true),
		);
		const harness = publisher();
		await harness.service.refresh(true);
		expect(harness.latest()[0].page?.text).toBe("Captured content");
		expect(harness.latest()[0].updatedAt).toBe(originalDate.toISOString());
		mocks.loadPage.mockResolvedValue(projection("New page", "v2", true));
		await harness.service.refresh(true);
		expect(harness.latest()[0].page?.text).toBe("New page");
		expect(harness.latest()[0].message).toContain("Open this app page");
	});

	it("accepts only fresh captures for the configured scope, definition and authenticated page revision", async () => {
		saveNativeWidgetDefinitions(scope, [definition()]);
		mocks.loadPage.mockResolvedValue(projection("Static", "v1", true));
		const harness = publisher();
		harness.service.start();
		await flush();
		for (const override of [
			{ scope: "account-b" },
			{ revision: "old" },
			{ capturedAt: Date.now() - 120_000 },
			{ pageId: "another-page" },
			{ pageRevision: "old" },
		]) {
			capture("Rejected capture", override);
			await flush();
		}
		expect(JSON.stringify(harness.publish.mock.calls)).not.toContain(
			"Rejected capture",
		);
		capture("Current live value");
		await flush();
		expect(harness.latest()[0].page?.text).toBe("Current live value");
		expect(harness.latest()[0].message).toBeUndefined();
	});

	it("honors the refresh interval while preserving a captured page's original data timestamp", async () => {
		const item = definition();
		const originalDate = new Date("2026-09-13T09:00:00.000Z");
		seed(scope, {
			...nativeWidgetShell(item, originalDate),
			state: "ready",
			page: projection("Stale captured content").root,
		});
		saveNativeWidgetDefinitions(scope, [item]);
		mocks.loadPage.mockResolvedValue(projection("Static", "v1", true));
		const harness = publisher();
		harness.service.start();
		await flush();
		expect(mocks.loadPage).toHaveBeenCalledTimes(1);
		await vi.advanceTimersByTimeAsync(60_000);
		expect(mocks.loadPage).toHaveBeenCalledTimes(1);
		await vi.advanceTimersByTimeAsync(29 * 60_000);
		expect(mocks.loadPage).toHaveBeenCalledTimes(2);
		expect(harness.latest()[0].updatedAt).toBe(originalDate.toISOString());
	});

	it("removes cached private content when refreshing discovers lost access", async () => {
		const item = definition();
		seed(scope, {
			...nativeWidgetShell(item),
			state: "ready",
			page: projection("Previously authorized").root,
		});
		saveNativeWidgetDefinitions(scope, [item]);
		mocks.assertApp.mockRejectedValue(
			new Error("You no longer have access to this app."),
		);
		const harness = publisher();
		await harness.service.refresh(true);
		expect(harness.latest()[0].state).toBe("error");
		expect(harness.latest()[0].page).toBeUndefined();
	});

	it("clears cached private content when a live capture discovers revoked access", async () => {
		const item = definition();
		seed(scope, {
			...nativeWidgetShell(item),
			state: "ready",
			page: projection("Old private content").root,
		});
		saveNativeWidgetDefinitions(scope, [item]);
		const harness = publisher();
		harness.service.start();
		await flush();
		mocks.assertApp.mockRejectedValue(
			new Error("You no longer have access to this app."),
		);
		capture("Must not publish");
		await flush();
		expect(harness.latest()[0].state).toBe("error");
		expect(harness.latest()[0].page).toBeUndefined();
	});

	it("invalidates an old capture after authentication discovers a changed page revision", async () => {
		const item = definition();
		seed(scope, {
			...nativeWidgetShell(item),
			state: "ready",
			page: projection("Old published page").root,
		});
		saveNativeWidgetDefinitions(scope, [item]);
		const harness = publisher();
		harness.service.start();
		await flush();
		mocks.loadPage.mockResolvedValue(projection("New page", "v2", true));
		capture("An old active page", { pageRevision: "v1" });
		await flush();
		expect(harness.latest()[0].page?.text).not.toBe("Old published page");
		expect(harness.latest()[0].page?.text).not.toBe("An old active page");
		expect(harness.latest()[0].message).toMatch(/open/i);
	});

	it("retains live action output from pages without a load Event", async () => {
		saveNativeWidgetDefinitions(scope, [definition()]);
		mocks.loadPage.mockResolvedValue(
			projection("Initial static page", "v1", false),
		);
		const harness = publisher();
		harness.service.start();
		await flush();
		capture("Button updated the page");
		await flush();
		expect(harness.latest()[0].page?.text).toBe("Button updated the page");
		await harness.service.refresh(true);
		expect(harness.latest()[0].page?.text).toBe("Button updated the page");
	});

	it("expires captured content even when the publisher was already running", async () => {
		const item = definition();
		seed(scope, {
			...nativeWidgetShell(item),
			state: "ready",
			page: projection("Expired captured content").root,
			staleAt: new Date(Date.now() + 30_000).toISOString(),
			expiresAt: new Date(Date.now() + 60_000).toISOString(),
		});
		saveNativeWidgetDefinitions(scope, [item]);
		mocks.loadPage.mockResolvedValue(projection("Static page", "v1", true));
		const harness = publisher();
		await harness.service.refresh();
		expect(harness.latest()[0].page?.text).toBe("Expired captured content");
		vi.setSystemTime(new Date(Date.now() + 120_000));
		await harness.service.refresh();
		expect(harness.latest()[0].page?.text).toBe("Static page");
		expect(harness.latest()[0].message).toMatch(/open/i);
	});

	it("does not run queries while hidden or offline", async () => {
		saveNativeWidgetDefinitions(scope, [definition()]);
		const harness = publisher();
		Object.assign(document, { visibilityState: "hidden" });
		await harness.service.refresh(true);
		expect(mocks.loadPage).not.toHaveBeenCalled();
		Object.assign(document, { visibilityState: "visible" });
		Object.assign(navigator, { onLine: false });
		await harness.service.refresh(true);
		expect(mocks.loadPage).not.toHaveBeenCalled();
		expect(harness.latest()[0].state).toBe("unavailable");
	});

	it("continues to other widgets after a read times out", async () => {
		const hung = deferred<ReturnType<typeof projection>>();
		saveNativeWidgetDefinitions(scope, [
			definition("hung"),
			definition("fast"),
			definition("next"),
		]);
		mocks.loadPage.mockImplementation(
			(_pageState, item: NativeWidgetDefinition) =>
				item.id === "hung"
					? hung.promise
					: Promise.resolve(projection(item.id)),
		);
		const harness = publisher();
		const refreshing = harness.service.refresh(true);
		await flush();
		await vi.advanceTimersByTimeAsync(30_001);
		await refreshing;
		expect(
			harness.latest().find((item) => item.id === "next")?.page?.text,
		).toBe("next");
	});

	it("passes chart reads through the authorized app query adapter", async () => {
		const item = newNativeWidgetDefinition("chart");
		if (item.kind !== "chart") throw new Error("Expected chart definition");
		item.data.appId = "app";
		item.data.table = "orders";
		item.data.sourceKind = "table";
		item.data.visualization = "stat";
		saveNativeWidgetDefinitions(scope, [item]);
		mocks.loadChart.mockImplementation(
			async (_backend, configured: NativeWidgetDefinition) => ({
				...nativeWidgetShell(configured),
				state: "ready",
				chart: {
					type: "stat",
					value: 12,
					formattedValue: "12",
					points: [],
					format: { style: "number", currency: "USD", decimals: 0 },
				},
			}),
		);
		const harness = publisher();
		await harness.service.refresh(true);
		expect(mocks.loadChart).toHaveBeenCalledWith(
			backend,
			expect.objectContaining({ appId: "app" }),
			expect.objectContaining({
				viewerId: "viewer",
				signal: expect.any(AbortSignal),
			}),
		);
		expect(harness.latest()[0].chart?.value).toBe(12);
		expect(mocks.loadPage).not.toHaveBeenCalled();
	});
});

describe("native page image materialization", () => {
	it("keeps signed sources out of display content and reports failed media without losing text", async () => {
		mocks.icons.mockResolvedValue({
			photo: { png: "normalized-png", template: false },
		});
		const result = await materializeNativeWidgetPage({
			supported: true,
			root: {
				id: "root",
				kind: "column",
				children: [
					{ id: "photo", kind: "image", text: "Photo" },
					{ id: "missing", kind: "image", text: "Missing" },
					{ id: "text", kind: "text", text: "Status" },
				],
			},
			warnings: [],
			media: [
				{
					nodeId: "photo",
					source: "https://assets.test/photo?signature=secret",
				},
				{ nodeId: "missing", source: "https://assets.test/missing" },
			],
		});
		expect(result.page?.children?.[0].image?.png).toBe("normalized-png");
		expect(result.page?.children?.[2].text).toBe("Status");
		expect(JSON.stringify(result)).not.toContain("secret");
		expect(result.warnings).toContain("Some images could not be loaded.");
		expect(mocks.disposeIcons).toHaveBeenCalled();
	});
});
