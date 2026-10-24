import { afterAll, expect, test } from "bun:test";

type Posted = { kind: string; uri?: string; doc?: { text?: string } };
const posted: Posted[] = [];

class RecordingWorker {
	onmessage: ((event: MessageEvent) => void) | null = null;
	onerror: (() => void) | null = null;
	postMessage(message: Posted) {
		posted.push(message);
	}
	terminate() {}
}

const globals = globalThis as { window?: unknown; Worker?: unknown };
const original = { window: globals.window, Worker: globals.Worker };
Object.assign(globalThis, { window: globalThis, Worker: RecordingWorker });
const { FLOWSCRIPT_WORKER_MIN_TEXT_LENGTH, requestFlowScriptWorkerFolding } =
	await import("./flowscript-worker-client");

afterAll(() => {
	Object.assign(globalThis, original);
	if (original.window === undefined) Reflect.deleteProperty(globals, "window");
});

test("a disposed model's text is dropped from the worker, once", () => {
	let version = 1;
	let subscriptions = 0;
	let dispose: (() => void) | undefined;
	const model = {
		uri: "inmemory://model/1",
		getValue: () => "x".repeat(FLOWSCRIPT_WORKER_MIN_TEXT_LENGTH),
		getVersionId: () => version,
		onWillDispose: (listener: () => void) => {
			subscriptions++;
			dispose = listener;
		},
	};

	void requestFlowScriptWorkerFolding(model, []);
	version = 2;
	void requestFlowScriptWorkerFolding(model, []);
	const folding = posted.filter((message) => message.kind === "folding");
	expect(folding).toHaveLength(2);
	expect(folding[1].doc?.text).toBeDefined();
	expect(subscriptions).toBe(1);

	dispose?.();
	dispose?.();
	expect(posted.filter((message) => message.kind === "forget-doc")).toEqual([
		{ kind: "forget-doc", uri: "inmemory://model/1" },
	]);
});
