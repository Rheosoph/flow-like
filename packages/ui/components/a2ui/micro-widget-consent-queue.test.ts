import { describe, expect, test } from "bun:test";
import {
	MicroWidgetConsentQueue,
	microWidgetConsentQueueKey,
} from "./micro-widget-consent-queue";
import type { MicroWidgetConsentPrompt } from "./use-micro-widget-grant";

function snapshot(queue: MicroWidgetConsentQueue) {
	return queue.getSnapshot().map((entry) => [entry.key, ...entry.members]);
}

describe("micro widget consent queue", () => {
	test("keeps one entry per key in mount order and shares it between instances", () => {
		const queue = new MicroWidgetConsentQueue();
		queue.join("map", "a");
		queue.join("chart", "b");
		queue.join("map", "c");
		expect(snapshot(queue)).toEqual([
			["map", "a", "c"],
			["chart", "b"],
		]);
	});

	test("the next instance of an entry takes over when the first leaves", () => {
		const queue = new MicroWidgetConsentQueue();
		queue.join("map", "a");
		queue.join("map", "c");
		queue.join("chart", "b");
		queue.leave("a");
		expect(snapshot(queue)).toEqual([
			["map", "c"],
			["chart", "b"],
		]);
		queue.leave("c");
		expect(snapshot(queue)).toEqual([["chart", "b"]]);
		queue.leave("missing");
		expect(snapshot(queue)).toEqual([["chart", "b"]]);
	});

	test("review, banner and blocked card jump to the front", () => {
		const queue = new MicroWidgetConsentQueue();
		queue.join("map", "a");
		queue.join("chart", "b");
		queue.requestFront("c");
		queue.join("table", "c");
		expect(snapshot(queue).map(([key]) => key)).toEqual([
			"table",
			"map",
			"chart",
		]);
		queue.requestFront("b");
		expect(snapshot(queue).map(([key]) => key)).toEqual([
			"chart",
			"table",
			"map",
		]);
	});

	test("a front request is spent by the next join only", () => {
		const queue = new MicroWidgetConsentQueue();
		queue.join("map", "a");
		queue.requestFront("b");
		queue.join("chart", "b");
		queue.join("table", "b");
		queue.join("list", "d");
		queue.join("chart2", "b");
		expect(snapshot(queue).map(([key]) => key)).toEqual([
			"chart2",
			"map",
			"list",
		]);
	});

	test("a newer frozen prompt keeps the instance's place in line", () => {
		const queue = new MicroWidgetConsentQueue();
		queue.join("map@1", "a");
		queue.join("chart", "b");
		queue.join("map@2", "a");
		expect(snapshot(queue)).toEqual([
			["map@2", "a"],
			["chart", "b"],
		]);
	});

	test("a member leaving a shared entry for a newer prompt goes right before it", () => {
		const queue = new MicroWidgetConsentQueue();
		queue.join("map@1", "a");
		queue.join("map@1", "c");
		queue.join("map@2", "a");
		expect(snapshot(queue)).toEqual([
			["map@2", "a"],
			["map@1", "c"],
		]);
	});

	test("notifies subscribers and keeps the snapshot stable between changes", () => {
		const queue = new MicroWidgetConsentQueue();
		let notified = 0;
		const unsubscribe = queue.subscribe(() => notified++);
		const empty = queue.getSnapshot();
		expect(queue.getSnapshot()).toBe(empty);
		queue.join("map", "a");
		queue.join("map", "a");
		expect(notified).toBe(1);
		unsubscribe();
		queue.leave("a");
		expect(notified).toBe(1);
	});
});

function prompt(
	overrides: Partial<MicroWidgetConsentPrompt> & {
		digest?: string;
	} = {},
): MicroWidgetConsentPrompt {
	const { digest = `sha256:${"a".repeat(64)}`, ...rest } = overrides;
	return {
		key: digest,
		mode: "mount",
		descriptor: {
			source: "hub",
			packageId: "com.example.maps",
			bundleHash: "b",
			widgetId: "map",
			preview: false,
			status: "ok",
			policy: { workers: true },
			policyDigest: digest,
			networkInputs: [],
		},
		declaredDescriptor: null,
		subject: {
			source: "hub",
			packageId: "com.example.maps",
			widgetId: "map",
			policy: { workers: true },
		},
		declared: { policy: { workers: true }, levels: {}, covered: false },
		runtime: { pending: [], allowed: [], level: null, slots: [] },
		newSources: [],
		raisedSources: [],
		newCapabilities: ["workers"],
		hasRuntimeCheckbox: false,
		includeRuntimeDefault: true,
		includeRuntime: true,
		newerAvailable: false,
		canAllowForProject: true,
		canStopAsking: false,
		level: null,
		...rest,
	};
}

describe("micro widget consent queue keys", () => {
	test("the same target and descriptor share a key", () => {
		expect(microWidgetConsentQueueKey(prompt(), "app")).toBe(
			microWidgetConsentQueueKey(prompt({ includeRuntime: false }), "app"),
		);
	});

	test("a different digest, app or legacy policy is a different entry", () => {
		const base = microWidgetConsentQueueKey(prompt(), "app");
		expect(
			microWidgetConsentQueueKey(
				prompt({ digest: `sha256:${"b".repeat(64)}` }),
				"app",
			),
		).not.toBe(base);
		expect(microWidgetConsentQueueKey(prompt(), "other")).not.toBe(base);
		const legacy = (policy: MicroWidgetConsentPrompt["subject"]["policy"]) =>
			microWidgetConsentQueueKey(
				prompt({
					descriptor: null,
					subject: {
						source: "registry:*",
						packageId: "com.example.maps",
						widgetId: "map",
						policy,
					},
				}),
				null,
			);
		expect(legacy({ workers: true })).not.toBe(legacy({ wasm: true }));
		expect(legacy({ workers: true, wasm: true })).toBe(
			legacy({ wasm: true, workers: true }),
		);
	});
});
