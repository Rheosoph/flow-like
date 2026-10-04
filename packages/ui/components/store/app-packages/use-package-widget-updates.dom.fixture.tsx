import { afterAll, afterEach, beforeEach, expect, mock, test } from "bun:test";
import type { WidgetContract } from "@flow-like/widget-sdk";
import {
	QueryClient,
	QueryClientProvider,
	notifyManager,
} from "@tanstack/react-query";
import { Window } from "happy-dom";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import type { AppPackageWidget } from "../../../lib/package-widgets";
import { trackPageSave } from "../../../lib/pending-page-saves";
import type { IPage } from "../../../state/backend-state/page-state";
import type {
	MicroWidgetInstanceComponent,
	SurfaceComponent,
} from "../../a2ui/types";

const window = new Window({ url: "https://packages.local" });
Object.assign(globalThis, {
	window,
	document: window.document,
	navigator: window.navigator,
	localStorage: window.localStorage,
	IS_REACT_ACT_ENVIRONMENT: true,
});
notifyManager.setNotifyFunction((callback) => {
	act(callback);
});

const OLD_HASH = "a".repeat(64);
const NEW_HASH = "b".repeat(64);
const contract: WidgetContract = {
	contractVersion: 1,
	id: "chart",
	inputs: { title: { type: "string", default: "Sales" } },
	events: { selected: { payloadSchema: { type: "string" } } },
	queries: {},
};

function widget(
	id: string,
	packageId = "sales",
	widgetId = "chart",
): SurfaceComponent & { component: MicroWidgetInstanceComponent } {
	return {
		id,
		component: {
			id,
			type: "microWidgetInstance",
			instanceId: id,
			packageId,
			widgetId,
			packageVersion: "1.0.0",
			bundleHash: OLD_HASH,
			contract: { ...contract, id: widgetId },
			props: { title: "Configured title" },
			actionBindings: { selected: { nodeId: "event" } },
			eventHandlers: {
				selected: [{ name: "workflow_event", context: { nodeId: "event" } }],
			},
		},
	};
}

function page(
	id: string,
	boardId: string,
	components: SurfaceComponent[],
): IPage {
	return {
		id,
		name: id,
		boardId,
		layoutType: "freeform",
		components,
		content: [],
		createdAt: "2026-10-01T00:00:00Z",
		updatedAt: "2026-10-01T00:00:00Z",
	};
}

function listedWidget(packageId: string, widgetId: string): AppPackageWidget {
	return {
		packageId,
		packageName: packageId,
		packageVersion: "1.0.0",
		bundleHash: NEW_HASH,
		widget: {
			id: widgetId,
			name: widgetId,
			description: "",
			icon: null,
			thumbnail: null,
			keywords: [],
			contract: { ...contract, id: widgetId },
		},
	};
}

const widgets = [
	listedWidget("sales", "chart"),
	listedWidget("sales", "table"),
	listedWidget("unrelated", "chart"),
];
const latestVersions = new Map<string, string>();
interface Notice {
	kind: string;
	title: string;
	options?: {
		description?: string;
		action?: { label: string; onClick: () => void };
	};
}

let root: Root;
let client: QueryClient;
let pages: IPage[];
let remoteVersion: string;
let preparedVersion: string | undefined;
let manifestVersion: string | undefined;
let prepareError: Error | undefined;
let missingPackage: boolean;
let failedPageId: string | undefined;
let writeGate: Promise<void> | undefined;
let scanReadGate: Promise<void> | undefined;
let scanReadStarted: boolean;
let scanReadFinished: boolean;
const notices: Notice[] = [];
const trace: string[] = [];
const saves: IPage[] = [];
const attempts: string[] = [];
const requestedPackages: string[] = [];
const readers: ReturnType<typeof usePackageWidgetUpdates>[] = [];
const backend = {
	appState: {
		listPackages: async () => {
			trace.push("list-packages");
			return client.fetchQuery({
				queryKey: ["app", "app", "packages", "local-pins"],
				queryFn: async () => {
					trace.push("sync-pins");
					return { sales: remoteVersion, unrelated: "1.0.0" };
				},
				staleTime: Number.POSITIVE_INFINITY,
			});
		},
	},
	registryState: {
		getPackage: async (packageId: string, appId: string) => {
			expect(appId).toBe("app");
			requestedPackages.push(packageId);
			trace.push(`manifest:${packageId}`);
			if (packageId === "unrelated") throw new Error("Unrelated access denied");
			if (missingPackage) return null;
			const pins = client.getQueryData<{ sales: string }>([
				"app",
				"app",
				"packages",
				"local-pins",
			]);
			return {
				version: manifestVersion ?? pins?.sales ?? "1.0.0",
				manifest: {
					widgetBundleHash: NEW_HASH,
					widgets: widgets
						.filter((entry) => entry.packageId === packageId)
						.map((entry) => entry.widget),
				},
			};
		},
	},
	pageState: {
		getPagesAuthoritative: async () =>
			pages.map((stored) => ({
				pageId: stored.id,
				name: stored.name,
				boardId: stored.boardId,
			})),
		getPageAuthoritative: async (
			_appId: string,
			pageId: string,
			boardId?: string,
		) => {
			const stored = pages.find((item) => item.id === pageId);
			expect(stored?.boardId).toBe(boardId);
			const snapshot = structuredClone(stored);
			if (scanReadGate && pageId === "quiz") {
				const gate = scanReadGate;
				scanReadGate = undefined;
				scanReadStarted = true;
				await gate;
				scanReadFinished = true;
			}
			return snapshot;
		},
		updatePage: async (_appId: string, updated: IPage) => {
			attempts.push(updated.id);
			await writeGate;
			if (updated.id === failedPageId) throw new Error("Page save failed");
			saves.push(structuredClone(updated));
			pages = pages.map((stored) =>
				stored.id === updated.id ? updated : stored,
			);
		},
		getPage: async () => {},
		getPages: async () => {},
		getPageBootstrap: async () => {},
	},
	boardState: { getBoard: async () => {} },
};

async function preparePackage(packageId: string) {
	expect(packageId).toBe("sales");
	trace.push("prepare");
	if (prepareError) throw prepareError;
	if (preparedVersion) remoteVersion = preparedVersion;
	return preparedVersion;
}

const translate = (
	_key: string,
	fallback: string,
	values: Record<string, unknown> = {},
) =>
	fallback.replace(/\{\{(\w+)\}\}/g, (_, name: string) =>
		String(values[name] ?? ""),
	);
mock.module("@flow-like/locales", () => ({
	useTranslation: () => ({ t: translate }),
}));
mock.module("../../../state/backend-state", () => ({
	useBackend: () => backend,
}));
const notice =
	(kind: string) => (title: string, options?: Notice["options"]) => {
		notices.push({ kind, title, options });
		return "notice";
	};
mock.module("sonner", () => ({
	toast: {
		error: notice("error"),
		warning: notice("warning"),
		info: notice("info"),
		success: notice("success"),
	},
}));

const { usePackageWidgetUpdates } = await import(
	"./use-package-widget-updates"
);

function Reader({ index }: { index: number }) {
	readers[index] = usePackageWidgetUpdates({
		appId: "app",
		widgets,
		latestVersions,
		enabled: true,
		preparePackage,
	});
	return null;
}

async function settle() {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
}

async function until(condition: () => boolean) {
	for (let index = 0; index < 50; index++) {
		if (condition()) return;
		await settle();
	}
	throw new Error("The expected package widget update state did not arrive");
}

beforeEach(async () => {
	pages = [
		page("overview", "board-a", [
			widget("overview-chart"),
			widget("overview-other", "unrelated"),
		]),
		page("quiz", "board-b", [
			widget("quiz-chart"),
			widget("quiz-table", "sales", "table"),
		]),
	];
	remoteVersion = "1.0.0";
	preparedVersion = "2.0.0";
	manifestVersion = undefined;
	prepareError = undefined;
	missingPackage = false;
	failedPageId = undefined;
	writeGate = undefined;
	scanReadGate = undefined;
	scanReadStarted = false;
	scanReadFinished = false;
	for (const items of [
		readers,
		notices,
		trace,
		saves,
		attempts,
		requestedPackages,
	])
		items.length = 0;
	client = new QueryClient({
		defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
	});
	client.setQueryData(["app", "app", "packages", "local-pins"], {
		sales: "1.0.0",
		unrelated: "1.0.0",
	});
	const host = window.document.createElement("div");
	window.document.body.appendChild(host);
	root = createRoot(host as unknown as HTMLElement);
	await act(async () => {
		root.render(
			<QueryClientProvider client={client}>
				<Reader index={0} />
				<Reader index={1} />
			</QueryClientProvider>,
		);
	});
	await until(
		() =>
			readers.length === 2 &&
			readers.every((reader) => reader.byPackage !== undefined) &&
			client.isFetching() === 0,
	);
});

afterEach(async () => {
	await act(() => root.unmount());
	client.clear();
	window.document.body.innerHTML = "";
});

afterAll(async () => {
	await window.happyDOM.abort();
});

test("updates each widget of the selected package across boards and preserves wiring", async () => {
	expect(readers[0].byPackage?.get("sales")).toEqual({ widgets: 3, pages: 2 });
	const originalUnrelated = structuredClone(pages[0].components[1]);
	const original = widget("overview-chart").component;
	await act(async () => {
		await readers[0].update("sales");
	});
	expect(saves.map((saved) => [saved.id, saved.boardId])).toEqual([
		["overview", "board-a"],
		["quiz", "board-b"],
	]);
	expect(pages[0].components[1]).toEqual(originalUnrelated);
	const migrated = pages
		.flatMap((stored) => stored.components)
		.filter(
			(entry) =>
				entry.component.type === "microWidgetInstance" &&
				entry.component.packageId === "sales",
		);
	expect(migrated).toHaveLength(3);
	for (const entry of migrated) {
		const placed = entry.component as MicroWidgetInstanceComponent;
		expect(placed.bundleHash).toBe(NEW_HASH);
		expect(placed.packageVersion).toBe("2.0.0");
		expect(placed.props).toEqual(original.props);
		expect(placed.actionBindings).toEqual(original.actionBindings);
		expect(placed.eventHandlers).toEqual(original.eventHandlers);
	}
	expect(notices.at(-1)?.title).toBe("Updated 3 widgets on 2 pages.");
});

test("applies the release and invalidates cached pins before resolving the manifest", async () => {
	await act(async () => {
		await readers[0].update("sales");
	});
	expect(trace).toEqual([
		"prepare",
		"list-packages",
		"sync-pins",
		"manifest:sales",
	]);
	expect(notices.at(-1)?.kind).toBe("success");
	expect(saves).toHaveLength(2);
});

test("an inaccessible unrelated package does not block the strict targeted refresh", async () => {
	await act(async () => {
		await readers[0].update("sales");
	});
	expect(requestedPackages).toEqual(["sales"]);
	expect(notices.at(-1)?.kind).toBe("success");
	expect(saves).toHaveLength(2);
});

test("a partial failure offers a retry that saves only the remaining page", async () => {
	failedPageId = "quiz";
	await act(async () => {
		await readers[0].update("sales");
	});
	await until(() => readers.every((reader) => !reader.isUpdating));
	expect(saves.map((saved) => saved.id)).toEqual(["overview"]);
	const warning = notices.find((entry) => entry.kind === "warning");
	expect(warning?.title).toBe("Updated 1 widgets on 1 pages.");
	expect(warning?.options?.description).toContain("quiz (Page save failed)");
	expect(warning?.options?.action?.label).toBe("Retry");
	failedPageId = undefined;
	await act(async () => {
		warning?.options?.action?.onClick();
	});
	await until(() => saves.length === 2 && client.isMutating() === 0);
	expect(saves.map((saved) => saved.id)).toEqual(["overview", "quiz"]);
	expect(attempts).toEqual(["overview", "quiz", "quiz"]);
	expect(notices.at(-1)?.kind).toBe("success");
});

test("all hook readers share pending state and suppress duplicate updates", async () => {
	let release = () => {};
	writeGate = new Promise<void>((resolve) => {
		release = resolve;
	});
	let pending: Promise<void> | undefined;
	await act(async () => {
		pending = readers[0].update("sales");
	});
	await until(
		() => attempts.length === 1 && readers.every((reader) => reader.isUpdating),
	);
	await act(async () => {
		await readers[1].update("sales");
	});
	expect(trace.filter((entry) => entry === "prepare")).toHaveLength(1);
	await act(async () => {
		release();
		await pending;
	});
	await until(() => readers.every((reader) => !reader.isUpdating));
	expect(saves).toHaveLength(2);
});

test("waits for a pending editor save before preparing the package or writing pages", async () => {
	let release = () => {};
	const pendingSave = new Promise<void>((resolve) => {
		release = resolve;
	});
	trackPageSave(backend.pageState, "app", pendingSave);
	let pendingUpdate: Promise<void> | undefined;
	await act(async () => {
		pendingUpdate = readers[0].update("sales");
	});
	await until(() => readers.every((reader) => reader.isUpdating));
	expect(trace).toEqual([]);
	expect(attempts).toEqual([]);
	await act(async () => {
		release();
		await pendingUpdate;
	});
	expect(trace[0]).toBe("prepare");
	expect(saves).toHaveLength(2);
});

test("a failed release preparation leaves every page untouched", async () => {
	prepareError = new Error("Package download failed");
	await act(async () => {
		await readers[0].update("sales");
	});
	expect(attempts).toEqual([]);
	expect(requestedPackages).toEqual([]);
	expect(notices.at(-1)?.kind).toBe("error");
	expect(notices.at(-1)?.title).toContain("Package download failed");
});

test("a manifest still on the previous version prevents page writes", async () => {
	manifestVersion = "1.0.0";
	await act(async () => {
		await readers[0].update("sales");
	});
	expect(attempts).toEqual([]);
	expect(notices.at(-1)?.kind).toBe("error");
	expect(notices.at(-1)?.title).toContain(
		"Package version 2.0.0 could not be loaded",
	);
});

test("a missing target manifest prevents page writes", async () => {
	missingPackage = true;
	await act(async () => {
		await readers[0].update("sales");
	});
	expect(attempts).toEqual([]);
	expect(notices.at(-1)?.kind).toBe("error");
	expect(notices.at(-1)?.title).toContain("Package sales could not be loaded");
});

test("a local rebuild migrates a changed hash without changing the version", async () => {
	preparedVersion = undefined;
	expect(readers[0].byPackage?.get("sales")).toEqual({ widgets: 3, pages: 2 });
	await act(async () => {
		await readers[0].update("sales");
	});
	await until(
		() =>
			!readers[0].isUpdating &&
			!readers[0].isChecking &&
			readers[0].byPackage?.get("sales") === undefined,
	);
	expect(remoteVersion).toBe("1.0.0");
	expect(saves).toHaveLength(2);
	const placed = saves[0].components[0]
		.component as MicroWidgetInstanceComponent;
	expect(placed.packageVersion).toBe("1.0.0");
	expect(placed.bundleHash).toBe(NEW_HASH);
	expect(notices.at(-1)?.kind).toBe("success");
});

test("an older in-flight scan cannot replace the counts refreshed after migration", async () => {
	preparedVersion = undefined;
	let releaseScan = () => {};
	scanReadGate = new Promise<void>((resolve) => {
		releaseScan = resolve;
	});
	let oldScan: Promise<void> | undefined;
	await act(async () => {
		// Simulate the first scan, before the query has any cached result to replace.
		oldScan = client.resetQueries({
			queryKey: ["app-package-widget-usage", "app"],
		});
	});
	await until(() => scanReadStarted && readers[0].isChecking);
	expect(readers[0].isChecking).toBe(true);
	expect(readers[0].byPackage).toBeUndefined();
	try {
		await act(async () => {
			await readers[0].update("sales");
		});
		await until(
			() =>
				!readers[0].isUpdating &&
				!readers[0].isChecking &&
				readers[0].byPackage?.get("sales") === undefined,
		);
		expect(scanReadFinished).toBe(false);
		expect(saves).toHaveLength(2);
	} finally {
		await act(async () => {
			releaseScan();
			await oldScan;
		});
	}
	await until(() => scanReadFinished);
	await settle();
	expect(readers[0].byPackage?.get("sales")).toBeUndefined();
});
