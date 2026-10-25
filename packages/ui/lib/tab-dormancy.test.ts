import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import {
	holdTabDormancy,
	trackTabActivity,
	watchTabDormancy,
} from "./tab-dormancy";

class FakeDocument extends EventTarget {
	visibilityState: DocumentVisibilityState = "visible";
	show(state: DocumentVisibilityState) {
		this.visibilityState = state;
		this.dispatchEvent(new Event("visibilitychange"));
	}
}

const AFTER_MS = 150;
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

let page: FakeDocument;
let otherTab: BroadcastChannel;
const cleanups: (() => void)[] = [];

beforeEach(() => {
	page = new FakeDocument();
	Object.assign(globalThis, { document: page });
	otherTab = new BroadcastChannel("flow-like:tab-activity");
});

afterEach(() => {
	for (const cleanup of cleanups.splice(0)) cleanup();
	otherTab.close();
});

function watch(isBusy = () => false) {
	const changes: boolean[] = [];
	cleanups.push(
		watchTabDormancy({
			afterMs: AFTER_MS,
			isBusy,
			onChange: (dormant) => changes.push(dormant),
		}),
	);
	return changes;
}

describe("tab dormancy", () => {
	test("a tab left for another flow-like tab sleeps after the delay and wakes when shown", async () => {
		const changes = watch();
		page.show("hidden");
		otherTab.postMessage("visible");
		await sleep(AFTER_MS / 5);
		expect(changes).toEqual([]);
		await sleep(AFTER_MS * 2);
		expect(changes).toEqual([true]);

		page.show("visible");
		expect(changes).toEqual([true, false]);
	});

	test("a tab left for another site stays awake until a flow-like tab is shown", async () => {
		const changes = watch();
		page.show("hidden");
		await sleep(AFTER_MS * 2);
		expect(changes).toEqual([]);

		otherTab.postMessage("visible");
		await sleep(40);
		expect(changes).toEqual([true]);
	});

	test("held or busy tabs wait and sleep once released", async () => {
		let busy = true;
		const changes = watch(() => busy);
		const release = holdTabDormancy();
		page.show("hidden");
		otherTab.postMessage("visible");
		await sleep(AFTER_MS * 2);
		expect(changes).toEqual([]);

		release();
		busy = false;
		await sleep(AFTER_MS * 2);
		expect(changes).toEqual([true]);
	});

	test("a shown tab tells the other tabs", async () => {
		const received: unknown[] = [];
		otherTab.onmessage = (event) => received.push(event.data);
		page.visibilityState = "hidden";
		cleanups.push(trackTabActivity());
		page.show("visible");
		await sleep(40);
		expect(received).toEqual(["visible"]);
	});
});
