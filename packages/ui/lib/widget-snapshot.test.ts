import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import {
	captureWidgetSnapshots,
	createSnapshotStore,
	encodeSnapshotCanvas,
	registerWidgetSnapshotSource,
	unregisterWidgetSnapshotSource,
	widgetSnapshotAttribute,
} from "./widget-snapshot";

interface FakeCallback {
	run: () => void;
	cancelled: boolean;
}

function createFakeScheduler() {
	const timers: FakeCallback[] = [];
	const idle: FakeCallback[] = [];
	const visible: FakeCallback[] = [];
	let hidden = false;
	const enqueue = (list: FakeCallback[]) => (run: () => void) => {
		const entry: FakeCallback = { run, cancelled: false };
		list.push(entry);
		return () => {
			entry.cancelled = true;
		};
	};
	const flush = (list: FakeCallback[]) => {
		for (const entry of list.splice(0)) {
			if (!entry.cancelled) entry.run();
		}
	};
	return {
		setTimer: (run: () => void, _delayMs: number) => enqueue(timers)(run),
		whenIdle: enqueue(idle),
		onVisible: enqueue(visible),
		isHidden: () => hidden,
		setHidden: (value: boolean) => {
			hidden = value;
		},
		flushTimers: () => flush(timers),
		flushIdle: () => flush(idle),
		fireVisible: () => flush(visible),
		pendingTimers: () => timers.filter((entry) => !entry.cancelled).length,
		pendingVisible: () => visible.filter((entry) => !entry.cancelled).length,
	};
}

function fakeElement(id: string, connected = true): HTMLElement {
	return { id, isConnected: connected } as unknown as HTMLElement;
}

function settle(): Promise<void> {
	return new Promise((resolve) => setTimeout(resolve, 0));
}

function createHarness(capacity?: number) {
	const scheduler = createFakeScheduler();
	const captured: string[] = [];
	let sequence = 0;
	const store = createSnapshotStore({
		...scheduler,
		capacity,
		capture: async (element) => {
			captured.push(element.id);
			sequence += 1;
			return `data:image/webp;base64,${element.id}-${sequence}`;
		},
	});
	const settleAll = async () => {
		scheduler.flushTimers();
		scheduler.flushIdle();
		await settle();
	};
	const primed = async (instanceId: string, signature: string) => {
		store.schedule(instanceId, signature, () => fakeElement(instanceId));
		await settleAll();
	};
	return { scheduler, captured, store, settleAll, primed };
}

describe("createSnapshotStore", () => {
	test("serves the cached capture while the registered signature matches", async () => {
		const { captured, store, primed } = createHarness();
		await primed("w1", "sig-a");
		expect(captured).toEqual(["w1"]);

		const url = await store.snapshot("w1", () => fakeElement("w1"));
		expect(url).toBe("data:image/webp;base64,w1-1");
		expect(captured).toEqual(["w1"]);
	});

	test("captures again once the registered signature changes", async () => {
		const { captured, store, primed } = createHarness();
		await primed("w1", "sig-a");

		store.register("w1", "sig-b");
		const url = await store.snapshot("w1", () => fakeElement("w1"));
		expect(url).toBe("data:image/webp;base64,w1-2");
		expect(captured).toEqual(["w1", "w1"]);

		expect(await store.snapshot("w1", () => fakeElement("w1"))).toBe(url);
		expect(captured).toHaveLength(2);
	});

	test("skips scheduling when the signature is already cached", async () => {
		const { scheduler, store, primed } = createHarness();
		await primed("w1", "sig-a");

		store.schedule("w1", "sig-a", () => fakeElement("w1"));
		expect(scheduler.pendingTimers()).toBe(0);
	});

	test("coalesces rapid schedules into one capture of the latest signature", async () => {
		const { scheduler, captured, store, settleAll } = createHarness();
		for (const signature of ["sig-1", "sig-2", "sig-3"]) {
			store.schedule("w1", signature, () => fakeElement("w1"));
		}
		expect(scheduler.pendingTimers()).toBe(1);

		await settleAll();
		expect(captured).toEqual(["w1"]);
		expect(await store.snapshot("w1", () => fakeElement("w1"))).toBe(
			"data:image/webp;base64,w1-1",
		);
		expect(captured).toEqual(["w1"]);
	});

	test("evicts the least recently used entry beyond capacity", async () => {
		const { captured, store, primed } = createHarness(2);
		await primed("w1", "sig");
		await primed("w2", "sig");
		expect(await store.snapshot("w1", () => fakeElement("w1"))).toBe(
			"data:image/webp;base64,w1-1",
		);
		await primed("w3", "sig");
		expect(captured).toEqual(["w1", "w2", "w3"]);

		expect(await store.snapshot("w1", () => fakeElement("w1"))).toBe(
			"data:image/webp;base64,w1-1",
		);
		expect(await store.snapshot("w3", () => fakeElement("w3"))).toBe(
			"data:image/webp;base64,w3-3",
		);
		expect(captured).toEqual(["w1", "w2", "w3"]);

		expect(await store.snapshot("w2", () => fakeElement("w2"))).toBe(
			"data:image/webp;base64,w2-4",
		);
		expect(captured).toEqual(["w1", "w2", "w3", "w2"]);
	});

	test("defers the idle capture while the document is hidden", async () => {
		const { scheduler, captured, store } = createHarness();
		scheduler.setHidden(true);
		store.schedule("w1", "sig", () => fakeElement("w1"));
		scheduler.flushTimers();
		scheduler.flushIdle();
		await settle();
		expect(captured).toEqual([]);
		expect(scheduler.pendingVisible()).toBe(1);

		scheduler.setHidden(false);
		scheduler.fireVisible();
		expect(captured).toEqual([]);
		scheduler.flushIdle();
		await settle();
		expect(captured).toEqual(["w1"]);
	});

	test("unregister cancels the pending capture", async () => {
		const { captured, store, settleAll } = createHarness();
		store.schedule("w1", "sig", () => fakeElement("w1"));
		store.unregister("w1");
		await settleAll();
		expect(captured).toEqual([]);
	});

	test("does not cache captures of unregistered instances", async () => {
		const { captured, store } = createHarness();
		expect(await store.snapshot("w1", () => fakeElement("w1"))).toBe(
			"data:image/webp;base64,w1-1",
		);
		store.register("w1", "sig");
		expect(await store.snapshot("w1", () => fakeElement("w1"))).toBe(
			"data:image/webp;base64,w1-2",
		);
		expect(captured).toEqual(["w1", "w1"]);
	});

	test("awaits an in-flight pre-capture instead of starting a second one", async () => {
		const scheduler = createFakeScheduler();
		let resolveCapture: (url: string) => void = () => undefined;
		let captures = 0;
		const store = createSnapshotStore({
			...scheduler,
			capture: () =>
				new Promise<string>((resolve) => {
					captures += 1;
					resolveCapture = resolve;
				}),
		});
		store.schedule("w1", "sig", () => fakeElement("w1"));
		scheduler.flushTimers();
		scheduler.flushIdle();
		expect(captures).toBe(1);

		const pending = store.snapshot("w1", () => fakeElement("w1"));
		resolveCapture("data:image/webp;base64,shared");
		expect(await pending).toBe("data:image/webp;base64,shared");
		expect(captures).toBe(1);
		expect(await store.snapshot("w1", () => fakeElement("w1"))).toBe(
			"data:image/webp;base64,shared",
		);
		expect(captures).toBe(1);
	});

	test("skips instances without a connected element", async () => {
		const { captured, store, settleAll } = createHarness();
		store.register("w1", "sig");
		expect(await store.snapshot("w1", () => null)).toBeNull();
		expect(await store.snapshot("w1", () => fakeElement("w1", false))).toBe(
			null,
		);
		store.schedule("w1", "sig", () => null);
		await settleAll();
		expect(captured).toEqual([]);
	});
});

describe("encodeSnapshotCanvas", () => {
	test("encodes WebP asynchronously as a data URL", async () => {
		const requested: { type?: string; quality?: unknown }[] = [];
		const url = await encodeSnapshotCanvas({
			toBlob: (callback, type, quality) => {
				requested.push({ type, quality });
				callback(new Blob(["raster"], { type }));
			},
			toDataURL: () => "data:image/png;base64,UE5H",
		});
		expect(requested).toEqual([{ type: "image/webp", quality: 0.8 }]);
		expect(url).toBe(`data:image/webp;base64,${btoa("raster")}`);
	});

	test("falls back to PNG when the engine yields no blob", async () => {
		const url = await encodeSnapshotCanvas({
			toBlob: (callback) => callback(null),
			toDataURL: (type) => `data:${type};base64,UE5H`,
		});
		expect(url).toBe("data:image/png;base64,UE5H");
	});

	test("falls back to PNG when the engine cannot encode WebP", async () => {
		const url = await encodeSnapshotCanvas({
			toBlob: (callback) =>
				callback(new Blob(["raster"], { type: "image/png" })),
			toDataURL: (type) => `data:${type};base64,UE5H`,
		});
		expect(url).toBe("data:image/png;base64,UE5H");
	});
});

const rasterized: { element: HTMLElement; options: Record<string, unknown> }[] =
	[];

mock.module("html2canvas-pro", () => ({
	default: async (element: HTMLElement, options: Record<string, unknown>) => {
		rasterized.push({ element, options });
		return {
			toBlob: (callback: (blob: Blob) => void, mediaType: string) =>
				callback(new Blob([`raster:${element.id}`], { type: mediaType })),
			toDataURL: () => "data:image/png;base64,UE5H",
		};
	},
}));

describe("captureWidgetSnapshots", () => {
	let browserWindow: Window;
	const mounted: string[] = [];

	const mountWidget = (instanceId: string, width = 320, height = 200) => {
		const element = document.createElement("div");
		element.id = instanceId;
		for (const [name, value] of Object.entries(
			widgetSnapshotAttribute(instanceId),
		)) {
			element.setAttribute(name, value);
		}
		element.getBoundingClientRect = () => ({ width, height }) as DOMRect;
		document.body.append(element);
		mounted.push(instanceId);
		return element;
	};

	beforeEach(() => {
		browserWindow = new Window({ url: "https://flow-like.test" });
		Object.assign(browserWindow, { SyntaxError, TypeError });
		Object.assign(globalThis, {
			document: browserWindow.document,
			window: browserWindow,
			CSS: browserWindow.CSS,
		});
		rasterized.length = 0;
	});

	afterEach(() => {
		for (const instanceId of mounted.splice(0)) {
			unregisterWidgetSnapshotSource(instanceId);
		}
		browserWindow.close();
	});

	test("returns cached entries without rasterizing again", async () => {
		mountWidget("cached-widget");
		registerWidgetSnapshotSource("cached-widget", "sig-1");

		const first = await captureWidgetSnapshots(["cached-widget"]);
		expect(first).toEqual([
			`data:image/webp;base64,${btoa("raster:cached-widget")}`,
		]);
		expect(rasterized).toHaveLength(1);

		expect(await captureWidgetSnapshots(["cached-widget"])).toEqual(first);
		expect(rasterized).toHaveLength(1);

		registerWidgetSnapshotSource("cached-widget", "sig-2");
		expect(await captureWidgetSnapshots(["cached-widget"])).toEqual(first);
		expect(rasterized).toHaveLength(2);
	});

	test("omits unrendered instances and keeps order", async () => {
		mountWidget("first");
		mountWidget("second");
		const urls = await captureWidgetSnapshots(["first", "missing", "second"]);
		expect(urls).toHaveLength(2);
		expect(rasterized.map((entry) => entry.element.id)).toEqual([
			"first",
			"second",
		]);
	});

	test("captures at scale 1 and caps the long edge at 1024px", async () => {
		mountWidget("small", 640, 400);
		mountWidget("wide", 2048, 400);
		await captureWidgetSnapshots(["small", "wide"]);
		expect(rasterized.map((entry) => entry.options.scale)).toEqual([1, 0.5]);
	});
});
