import { afterAll, afterEach, beforeEach, expect, mock, test } from "bun:test";
import type { WidgetContract } from "@flow-like/widget-sdk";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Window } from "happy-dom";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import type { IPage, PageContent } from "../../state/backend-state/page-state";
import type { MicroWidgetReloader } from "../a2ui/micro-widget-reload";
import type {
	MicroWidgetInstanceComponent,
	SurfaceComponent,
} from "../a2ui/types";
import type { BuilderContextType, BuilderSnapshot } from "./BuilderContext";

const window = new Window({ url: "https://builder.local" });
Object.assign(window, { SyntaxError, TypeError });
Object.assign(globalThis, {
	window,
	document: window.document,
	navigator: window.navigator,
	localStorage: window.localStorage,
	IS_REACT_ACT_ENVIRONMENT: true,
});

const OLD_HASH = "a".repeat(64);
const NEW_HASH = "b".repeat(64);
const contract: WidgetContract = {
	contractVersion: 1,
	id: "chart",
	inputs: { title: { type: "string", default: "Sales" } },
	events: {},
	queries: {},
};

function widget(id: string): SurfaceComponent {
	return {
		id,
		component: {
			id,
			type: "microWidgetInstance",
			instanceId: id,
			packageId: "sales",
			widgetId: "chart",
			packageVersion: "1.0.0",
			bundleHash: OLD_HASH,
			contract,
			props: { title: "Unsaved sales title" },
		},
	};
}

function page(id: string, boardId: string): IPage {
	return {
		id,
		name: id,
		boardId,
		layoutType: "freeform",
		components: [widget(`${id}-widget`)],
		content: [],
		createdAt: "2026-10-01T00:00:00Z",
		updatedAt: "2026-10-01T00:00:00Z",
	};
}

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
let builder: BuilderContextType;
const readers: MicroWidgetReloader[] = [];
const notices: Notice[] = [];
const savedRemote: IPage[] = [];
const savedActive: BuilderSnapshot[] = [];
const activeAttempts: BuilderSnapshot[] = [];
let pages: IPage[];
let remoteGate: Promise<void> | undefined;
let remoteStarted = false;
let missingPackage = false;
let failActiveSave = false;
let listings = 0;
const backend = {
	appState: {
		listPackages: async () => ({ sales: "1.0.0" }),
	},
	registryState: {
		getPackage: async () =>
			missingPackage
				? null
				: {
						version: "1.0.0",
						manifest: {
							widgetBundleHash: NEW_HASH,
							widgets: [{ id: "chart", name: "Chart", contract }],
						},
					},
	},
	pageState: {
		getPagesAuthoritative: async (appId: string) => {
			expect(appId).toBe("app");
			listings++;
			return pages.map((page) => ({
				pageId: page.id,
				name: page.name,
				boardId: page.boardId,
			}));
		},
		getPageAuthoritative: async (
			_appId: string,
			pageId: string,
			boardId?: string,
		) => {
			const stored = pages.find((page) => page.id === pageId);
			expect(stored?.boardId).toBe(boardId);
			return structuredClone(stored);
		},
		updatePage: async (_appId: string, updated: IPage) => {
			remoteStarted = true;
			await remoteGate;
			savedRemote.push(structuredClone(updated));
			pages = pages.map((page) => (page.id === updated.id ? updated : page));
		},
		getPage: async () => {},
		getPages: async () => {},
		getPageBootstrap: async () => {},
	},
	boardState: { getBoard: async () => {} },
};

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
mock.module("../../state/backend-state", () => ({ useBackend: () => backend }));
mock.module("../a2ui/micro-widget-purpose-card", () => ({
	WidgetSourceLevelBadge: () => null,
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

const { BuilderProvider, useBuilder } = await import("./BuilderContext");
const { useMicroWidgetReload } = await import("./use-micro-widget-reload");
const { MicroWidgetReloadAction } = await import(
	"../a2ui/micro-widget-capability-dialog"
);

function Reader({ index }: { index: number }) {
	builder = useBuilder();
	const reloader = useMicroWidgetReload();
	readers[index] = reloader;
	const placed = builder.getComponent("active-widget")?.component;
	if (
		index !== 0 ||
		placed?.type !== "microWidgetInstance" ||
		!reloader.updateFor(placed)
	)
		return null;
	return (
		<MicroWidgetReloadAction
			onReload={() => void reloader.reload("active-widget")}
			onReloadAll={reloader.reloadAll}
			isReloadingAll={reloader.isReloadingAll}
		/>
	);
}

function updateAllButton() {
	return Array.from(window.document.querySelectorAll("button")).find((button) =>
		button.textContent?.includes("Update all widgets"),
	);
}

async function settle() {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
}

async function until(condition: () => boolean) {
	for (let i = 0; i < 50; i++) {
		if (condition()) return;
		await settle();
	}
	throw new Error("The expected widget update state did not arrive");
}

beforeEach(async () => {
	pages = [
		page("active", "board-a"),
		page("remote-a", "board-a"),
		page("remote-b", "board-b"),
	];
	readers.length = 0;
	notices.length = 0;
	savedRemote.length = 0;
	savedActive.length = 0;
	activeAttempts.length = 0;
	remoteStarted = false;
	remoteGate = undefined;
	missingPackage = false;
	failActiveSave = false;
	listings = 0;
	client = new QueryClient({
		defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
	});
	const host = window.document.createElement("div");
	window.document.body.appendChild(host);
	root = createRoot(host as unknown as HTMLElement);
	await act(async () => {
		root.render(
			<QueryClientProvider client={client}>
				<BuilderProvider
					initialComponents={pages[0].components}
					actionContext={{
						appId: "app",
						pageId: "active",
						boardId: "board-a",
						pageContent: [],
						saveWidgetUpdates: async (
							snapshot: BuilderSnapshot,
							_content: PageContent[],
						) => {
							activeAttempts.push(structuredClone(snapshot));
							if (failActiveSave) throw new Error("Active page save failed");
							savedActive.push(structuredClone(snapshot));
						},
					}}
				>
					<Reader index={0} />
					<Reader index={1} />
				</BuilderProvider>
			</QueryClientProvider>,
		);
	});
	await until(
		() =>
			readers.length === 2 &&
			client.isFetching() === 0 &&
			updateAllButton() !== undefined,
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

test("the placeholder updates all boards, preserves live edits, and shares pending state", async () => {
	let release = () => {};
	remoteGate = new Promise<void>((resolve) => {
		release = resolve;
	});
	expect(updateAllButton()).toBeDefined();
	await act(async () => {
		updateAllButton()?.click();
	});
	await until(
		() => remoteStarted && readers.every((reader) => reader.isReloadingAll),
	);
	expect(updateAllButton()?.disabled).toBe(true);
	expect(updateAllButton()?.querySelector(".animate-spin")).not.toBeNull();
	expect(listings).toBe(1);
	await act(async () => {
		updateAllButton()?.click();
		await readers[1].reloadAll?.();
	});
	expect(listings).toBe(1);
	const current = builder.getComponent("active-widget")
		?.component as MicroWidgetInstanceComponent;
	await act(async () => {
		builder.updateComponent("active-widget", {
			component: {
				...current,
				props: { title: "Edited while other pages save" },
			},
		});
	});
	await act(async () => {
		release();
	});
	await until(() => readers.every((reader) => !reader.isReloadingAll));
	expect(updateAllButton()).toBeUndefined();
	expect(savedRemote.map((page) => [page.id, page.boardId])).toEqual([
		["remote-a", "board-a"],
		["remote-b", "board-b"],
	]);
	expect(savedActive).toHaveLength(1);
	const saved = savedActive[0].components[0]
		.component as MicroWidgetInstanceComponent;
	expect(saved.bundleHash).toBe(NEW_HASH);
	expect(saved.props?.title).toBe("Edited while other pages save");
	expect(
		(
			builder.getComponent("active-widget")
				?.component as MicroWidgetInstanceComponent
		).props?.title,
	).toBe("Edited while other pages save");
	expect(notices.at(-1)?.kind).toBe("success");
});

test("a missing package fails the strict refresh before any page is written", async () => {
	missingPackage = true;
	await act(async () => {
		await readers[0].reloadAll?.();
	});
	expect(listings).toBe(0);
	expect(savedRemote).toHaveLength(0);
	expect(activeAttempts).toHaveLength(0);
	expect(notices.at(-1)?.kind).toBe("error");
	expect(notices.at(-1)?.title).toContain("Package sales could not be loaded");
});

test("a partial failure retry saves an already updated active working copy", async () => {
	failActiveSave = true;
	await act(async () => {
		await readers[0].reloadAll?.();
	});
	await until(() => readers.every((reader) => !reader.isReloadingAll));
	expect(activeAttempts).toHaveLength(1);
	expect(savedActive).toHaveLength(0);
	expect(
		(
			builder.getComponent("active-widget")
				?.component as MicroWidgetInstanceComponent
		).bundleHash,
	).toBe(NEW_HASH);
	const warning = notices.find((notice) => notice.kind === "warning");
	expect(warning?.options?.description).toContain("Active page save failed");
	expect(warning?.options?.action?.label).toBe("Retry");
	failActiveSave = false;
	await act(async () => {
		warning?.options?.action?.onClick();
	});
	await until(() => savedActive.length === 1 && client.isMutating() === 0);
	expect(activeAttempts).toHaveLength(2);
	expect(savedRemote).toHaveLength(2);
	expect(
		(savedActive[0].components[0].component as MicroWidgetInstanceComponent)
			.bundleHash,
	).toBe(NEW_HASH);
});
