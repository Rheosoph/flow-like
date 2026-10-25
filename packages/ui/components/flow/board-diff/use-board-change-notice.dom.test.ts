import {
	afterAll,
	afterEach,
	beforeAll,
	describe,
	expect,
	setSystemTime,
	test,
} from "bun:test";
import { Window } from "happy-dom";
import type { IStoredBoardSnapshot } from "../../../db/board-last-seen-db";
import {
	type PackedSnapshot,
	packSnapshot,
	unpackSnapshot,
} from "../../../lib/board-diff/snapshot-codec";
import { createSnapshotEngine } from "../../../lib/board-diff/snapshot-engine";
import type { IBoard } from "../../../lib/schema/flow/board";
import { convertJsonToUint8Array } from "../../../lib/uint8";

const window = new Window({ url: "https://localhost" });
const GLOBALS = [
	"window",
	"document",
	"navigator",
	"Event",
	"CustomEvent",
	"IS_REACT_ACT_ENVIRONMENT",
] as const;
const saved = GLOBALS.map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
function installDomGlobals() {
	Object.assign(globalThis, {
		window,
		document: window.document,
		navigator: window.navigator,
		Event: window.Event,
		CustomEvent: window.CustomEvent,
		IS_REACT_ACT_ENVIRONMENT: true,
	});
}
installDomGlobals();

// react-dom reads the DOM at module load, so it is imported after the globals exist.
const { act, createElement } = await import("react");
const { createRoot } = await import("react-dom/client");
const { REMOTE_BOARD_APPLIED_EVENT } = await import(
	"../../../lib/flow-history"
);
const { createBoardLastSeenService } = await import(
	"./board-last-seen-service"
);
const { useBoardChangeNotice } = await import("./use-board-change-notice");

type Hook = ReturnType<typeof useBoardChangeNotice>;

function board(model: string): IBoard {
	return {
		id: "board",
		name: "Invoice Intake",
		description: "",
		created_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
		updated_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
		execution_mode: "Hybrid",
		log_level: "Info",
		stage: "Dev",
		version: [1, 2, 1],
		viewport: [0, 0, 1],
		page_ids: [],
		refs: {},
		layers: {},
		comments: {},
		variables: {},
		nodes: {
			ext: {
				id: "ext",
				name: "ai_extract",
				friendly_name: "AI Extractor",
				category: "AI",
				description: "",
				coordinates: [0, 0, 0],
				pins: {
					model: {
						id: "model",
						name: "model",
						friendly_name: "Model",
						description: "",
						pin_type: "Input",
						data_type: "String",
						value_type: "Normal",
						index: 0,
						connected_to: [],
						depends_on: [],
						default_value: convertJsonToUint8Array(model),
					},
				},
			},
		},
	} as unknown as IBoard;
}

const SEEN = board("gpt-4o-mini");
const CHANGED = board("claude-sonnet-5");

function synced(value: IBoard): IBoard {
	return {
		...value,
		version: [1, 2, 2],
		updated_at: { secs_since_epoch: 1, nanos_since_epoch: 0 },
	};
}

async function stored(value: IBoard): Promise<IStoredBoardSnapshot> {
	return { ...(await packSnapshot(JSON.stringify(value))), seenAt: 1 };
}

function memoryService(initial?: IStoredBoardSnapshot) {
	let current = initial;
	const writes: PackedSnapshot[] = [];
	const service = createBoardLastSeenService(
		{
			read: async () => current,
			write: async (_u, _a, _b, snapshot) => {
				writes.push(snapshot);
				current = { ...snapshot, seenAt: Date.now() };
			},
		},
		createSnapshotEngine().handle,
	);
	const lastWritten = async () => {
		const last = writes.at(-1);
		return last
			? (JSON.parse(await unpackSnapshot(last)) as IBoard)
			: undefined;
	};
	return { service, writes, lastWritten };
}

/** Lets the sliced serializer, the engine and the stream codecs finish. */
const settle = () =>
	act(async () => {
		for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 5));
	});

let latest: Hook | undefined;
function Probe(props: Parameters<typeof useBoardChangeNotice>[0]) {
	latest = useBoardChangeNotice(props);
	return null;
}

function mount(service: ReturnType<typeof memoryService>["service"]) {
	const root = createRoot(window.document.createElement("div") as never);
	const render = async (next: IBoard | undefined, fresh: boolean) => {
		await act(async () => {
			root.render(
				createElement(Probe, {
					appId: "app",
					boardId: "board",
					userKey: "user",
					board: next,
					fresh,
					enabled: true,
					service,
				}),
			);
		});
		await settle();
	};
	const unmount = async () => {
		await act(() => root.unmount());
		await settle();
	};
	return { render, unmount };
}

async function remoteMerge(before: IBoard, after: IBoard) {
	await act(async () => {
		window.dispatchEvent(
			new window.CustomEvent(REMOTE_BOARD_APPLIED_EVENT, {
				detail: {
					appId: "app",
					boardId: "board",
					reason: "sync",
					merge: { before, after },
				},
			}) as never,
		);
	});
}

const refreshPending = () =>
	act(async () => {
		await new Promise((r) => setTimeout(r, 450));
	});

describe("useBoardChangeNotice", () => {
	beforeAll(installDomGlobals);
	afterEach(() => {
		setSystemTime();
		latest = undefined;
	});
	afterAll(() => {
		for (const [key, descriptor] of saved) {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else delete (globalThis as Record<string, unknown>)[key];
		}
	});

	test("waits for a fresh board instead of deciding on a restored cache entry", async () => {
		const { service, writes } = memoryService(await stored(SEEN));
		const view = mount(service);
		await view.render(SEEN, false);
		expect(latest?.notice).toBeUndefined();
		expect(writes).toHaveLength(0);

		await view.render(CHANGED, true);
		expect(latest?.notice?.diff.logicCount).toBe(1);
		expect(latest?.notice?.base.nodes.ext).toBeDefined();
		expect(writes).toHaveLength(0);
		await view.unmount();
	});

	test("reports a hub merge that lands right after opening", async () => {
		const { service } = memoryService(await stored(SEEN));
		const view = mount(service);
		await view.render(SEEN, true);
		expect(latest?.notice).toBeUndefined();

		await remoteMerge(SEEN, CHANGED);
		await view.render(CHANGED, true);
		expect(latest?.notice?.diff.logicCount).toBe(1);
		await view.unmount();
	});

	test("treats a merge long after opening as a live edit", async () => {
		const { service } = memoryService(await stored(SEEN));
		const view = mount(service);
		setSystemTime(new Date("2026-09-30T10:00:00Z"));
		await view.render(SEEN, true);

		setSystemTime(new Date("2026-09-30T10:05:00Z"));
		await remoteMerge(SEEN, CHANGED);
		await view.render(CHANGED, true);
		expect(latest?.notice).toBeUndefined();
		await view.unmount();
	});

	test("does not report an own apply when sync only changes metadata", async () => {
		const { service } = memoryService(await stored(SEEN));
		const view = mount(service);
		await view.render(SEEN, true);
		await view.render(CHANGED, true);

		const merged = synced(CHANGED);
		await remoteMerge(CHANGED, merged);
		await view.render(merged, true);
		expect(latest?.notice).toBeUndefined();
		await view.unmount();
	});

	test("consumes a metadata-only sync before the next own edit", async () => {
		const { service } = memoryService(await stored(SEEN));
		const view = mount(service);
		await view.render(SEEN, true);

		const merged = synced(SEEN);
		await remoteMerge(SEEN, merged);
		await view.render(merged, true);
		expect(latest?.notice).toBeUndefined();

		await view.render(synced(CHANGED), true);
		expect(latest?.notice).toBeUndefined();
		await view.unmount();
	});

	test("reports only the external delta after an own edit", async () => {
		const { service } = memoryService(await stored(SEEN));
		const view = mount(service);
		await view.render(SEEN, true);
		await view.render(CHANGED, true);

		const merged = { ...CHANGED, name: "Remote invoice intake" };
		await remoteMerge(CHANGED, merged);
		await view.render(merged, true);
		expect(latest?.notice?.base).toEqual(CHANGED);
		expect(latest?.notice?.diff.logicCount).toBe(1);
		expect(latest?.notice?.diff.changes.map((change) => change.key)).toEqual([
			"board:settings",
		]);

		await refreshPending();
		await settle();
		expect(latest?.notice?.base).toEqual(CHANGED);
		expect(latest?.notice?.diff.logicCount).toBe(1);
		expect(latest?.notice?.diff.changes.map((change) => change.key)).toEqual([
			"board:settings",
		]);
		await view.unmount();
	});

	test("reopens an own applied board without a notice", async () => {
		const { service, lastWritten } = memoryService(await stored(SEEN));
		const view = mount(service);
		await view.render(SEEN, true);
		await view.render(CHANGED, true);
		const merged = synced(CHANGED);
		await remoteMerge(CHANGED, merged);
		await view.render(merged, true);
		await view.unmount();
		expect(await lastWritten()).toEqual(merged);

		const reopened = mount(service);
		await reopened.render(merged, true);
		expect(latest?.notice).toBeUndefined();
		await reopened.unmount();
	});

	test("detects an external change when its event arrives before the cache update", async () => {
		const { service } = memoryService(await stored(SEEN));
		const view = mount(service);
		await view.render(SEEN, true);
		await remoteMerge(SEEN, CHANGED);
		await settle();
		await refreshPending();
		await settle();
		expect(latest?.notice?.diff.logicCount).toBe(1);

		await view.render(CHANGED, true);
		expect(latest?.notice?.base).toEqual(SEEN);
		expect(latest?.notice?.diff.logicCount).toBe(1);
		await refreshPending();
		await settle();
		expect(latest?.notice?.diff.logicCount).toBe(1);
		await view.render(SEEN, true);
		await refreshPending();
		await settle();
		expect(latest?.notice).toBeUndefined();
		await view.unmount();
	});

	test("keeps an incoming change when a metadata-only merge follows before comparison finishes", async () => {
		const { service } = memoryService(await stored(SEEN));
		const view = mount(service);
		await view.render(SEEN, true);
		const comparison = service.compare;
		const gate = Promise.withResolvers<void>();
		service.compare = async (...args) => {
			await gate.promise;
			return comparison(...args);
		};

		await remoteMerge(SEEN, CHANGED);
		const merged = synced(CHANGED);
		await remoteMerge(CHANGED, merged);
		await view.render(merged, true);
		gate.resolve();
		await settle();
		expect(latest?.notice?.base).toEqual(SEEN);
		expect(latest?.notice?.diff.logicCount).toBe(1);
		await view.unmount();
	});

	test("does not restore an in-flight catch-up after marking the board as seen", async () => {
		const { service } = memoryService(await stored(SEEN));
		const view = mount(service);
		await view.render(SEEN, true);
		const comparison = service.compare;
		const gate = Promise.withResolvers<void>();
		service.compare = async (...args) => {
			await gate.promise;
			return comparison(...args);
		};

		await remoteMerge(SEEN, CHANGED);
		await view.render(CHANGED, true);
		await act(async () => {
			await latest?.markSeen();
		});
		gate.resolve();
		await settle();
		expect(latest?.notice).toBeUndefined();
		await view.unmount();
	});

	test("keeps a pending external change after an own edit and metadata-only sync", async () => {
		const { service } = memoryService(await stored(SEEN));
		const view = mount(service);
		await view.render(CHANGED, true);
		expect(latest?.notice?.diff.byKey.has("node:ext")).toBe(true);

		const edited = { ...CHANGED, name: "My invoice intake" };
		await view.render(edited, true);
		const merged = synced(edited);
		await remoteMerge(edited, merged);
		await view.render(merged, true);
		await refreshPending();
		await settle();
		expect(latest?.notice?.base).toEqual(SEEN);
		expect(latest?.notice?.diff.byKey.has("node:ext")).toBe(true);
		await view.unmount();
	});

	test("never counts the user's own edits and stores them on leave", async () => {
		const { service, lastWritten } = memoryService(await stored(SEEN));
		const view = mount(service);
		await view.render(SEEN, true);
		await view.render(CHANGED, true);
		expect(latest?.notice).toBeUndefined();
		await view.unmount();
		expect((await lastWritten())?.nodes.ext.pins.model.default_value).toEqual(
			CHANGED.nodes.ext.pins.model.default_value,
		);
	});

	test("does not rewrite a board whose text did not change", async () => {
		const { service, writes } = memoryService(await stored(SEEN));
		const view = mount(service);
		await view.render(SEEN, true);
		await view.render(structuredClone(SEEN), true);
		await view.unmount();
		expect(writes).toHaveLength(0);
	});

	test("stores a compressed baseline on the first visit", async () => {
		const { service, writes, lastWritten } = memoryService();
		const view = mount(service);
		await view.render(SEEN, true);
		expect(latest?.notice).toBeUndefined();
		expect(writes).toHaveLength(1);
		expect(writes[0].encoding).toBe("gzip-base64");
		expect((await lastWritten())?.id).toBe("board");
		await view.unmount();
	});

	test("drops the notice when the change is undone while it is pending", async () => {
		const { service } = memoryService(await stored(SEEN));
		const view = mount(service);
		await view.render(CHANGED, true);
		expect(latest?.notice).toBeDefined();
		await view.render(structuredClone(SEEN), true);
		await refreshPending();
		await settle();
		expect(latest?.notice).toBeUndefined();
		await view.unmount();
	});

	test("marking as seen stores the current board and clears the notice", async () => {
		const { service, lastWritten } = memoryService(await stored(SEEN));
		const view = mount(service);
		await view.render(CHANGED, true);
		expect(latest?.notice).toBeDefined();
		await act(async () => {
			await latest?.markSeen();
		});
		expect(latest?.notice).toBeUndefined();
		expect((await lastWritten())?.nodes.ext.pins.model.default_value).toEqual(
			CHANGED.nodes.ext.pins.model.default_value,
		);
		await view.unmount();
	});
});
