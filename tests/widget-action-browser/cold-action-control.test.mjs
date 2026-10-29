import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { PassThrough } from "node:stream";
import test from "node:test";
import {
	assertColdActionConfiguration,
	checkColdActionBoundary,
	coldBoundaryRequests,
	createColdActionBoundary,
	waitForColdActionController,
	waitForColdRequests,
} from "./cold-action-control.mjs";

function controller(timeoutMs = 1000, workerIds = [0]) {
	const input = new PassThrough();
	const output = new PassThrough();
	const signals = new EventEmitter();
	let written = "";
	output.on("data", (chunk) => {
		written += chunk;
	});
	const waiting = waitForColdActionController({
		input,
		output,
		signals,
		timeoutMs,
		workerIds,
	});
	return { input, output, signals, waiting, written: () => written };
}

test("cold action requires one verified local action and an owned browser", () => {
	const config = {
		samples: 1,
		navigations: 1,
		concurrency: 1,
		warmup: 0,
		requireActionTrace: true,
		requireTerminalSuccess: true,
	};
	assert.doesNotThrow(() =>
		assertColdActionConfiguration(config, new URL("http://127.0.0.1"), {}),
	);
	for (const patch of [
		{ samples: 2 },
		{ navigations: 2 },
		{ concurrency: 2 },
		{ warmup: 1 },
		{ requireActionTrace: false },
		{ requireTerminalSuccess: false },
		{ cdpUrl: "http://127.0.0.1:9222" },
	]) {
		assert.throws(() =>
			assertColdActionConfiguration(
				{ ...config, ...patch },
				new URL("http://localhost"),
				{},
			),
		);
	}
	assert.throws(() =>
		assertColdActionConfiguration(
			config,
			new URL("https://app.example.test"),
			{},
		),
	);
	assert.throws(() =>
		assertColdActionConfiguration(config, new URL("http://localhost"), {
			isTTY: true,
		}),
	);
});

test("cold action accepts exactly one verified burst for eight workers", () => {
	const config = {
		samples: 8,
		navigations: 1,
		concurrency: 8,
		warmup: 0,
		requireActionTrace: true,
		requireTerminalSuccess: true,
	};
	assert.doesNotThrow(() =>
		assertColdActionConfiguration(config, new URL("http://localhost"), {}),
	);
	for (const patch of [
		{ samples: 7 },
		{ samples: 16 },
		{ concurrency: 0, samples: 0 },
		{ concurrency: 1.5, samples: 1.5 },
		{ warmup: 1 },
		{ navigations: 2 },
	])
		assert.throws(() =>
			assertColdActionConfiguration(
				{ ...config, ...patch },
				new URL("http://localhost"),
				{},
			),
		);
});

test("eight workers emit one coordinated marker and missing or duplicate IDs fail", async () => {
	const workers = Array.from({ length: 8 }, (_, index) => index);
	const channel = controller(1000, workers);
	assert.deepEqual(JSON.parse(channel.written()), {
		type: "benchmark_cold_action_ready",
		navigation: 0,
		workers,
	});
	channel.input.write('{"type":"benchmark_cold_action_continue"}\n');
	await channel.waiting;
	assert.equal(channel.written().trim().split("\n").length, 1);
	for (const workerIds of [[], [1], [0, 2], [0, 0], [1, 0]])
		assert.throws(() => controller(1000, workerIds), /each worker/);
});

function bootstraps(count = 8) {
	return Array.from({ length: count }, (_, worker) => ({
		worker,
		index: 0,
		status: "ok",
		executionSuccessVerified: true,
		browserTimeOriginMs: 1_000 + worker,
	}));
}

test("every worker needs distinct successful bootstrap and document evidence", () => {
	const ids = Array.from({ length: 8 }, (_, index) => index);
	const boundary = createColdActionBoundary(ids, bootstraps());
	assert.equal(boundary.workerCount, 8);
	assert.equal(boundary.workers.length, 8);
	for (const entries of [
		bootstraps().slice(0, 7),
		[...bootstraps().slice(0, 7), bootstraps()[0]],
		bootstraps().map((entry) =>
			entry.worker === 7
				? { ...entry, executionSuccessVerified: false }
				: entry,
		),
		bootstraps().map((entry) =>
			entry.worker === 7 ? { ...entry, browserTimeOriginMs: undefined } : entry,
		),
	])
		assert.throws(() => createColdActionBoundary(ids, entries), /Every worker/);
	const single = createColdActionBoundary([0], bootstraps(1));
	assert.equal(single.worker, 0);
	assert.equal(
		single.documentTimeOriginMs,
		single.workers[0].documentTimeOriginMs,
	);
});

test("boundary waits for tracked requests on every worker, including terminal streams", async () => {
	const workers = Array.from({ length: 8 }, (_, id) => ({
		id,
		pendingRequests: new Map(),
	}));
	workers[7].pendingRequests.set("terminal-stream", { terminalObserved: true });
	let time = 0;
	const result = await waitForColdRequests(workers, {
		timeoutMs: 100,
		now: () => time,
		pause: async (ms) => {
			time += ms;
			if (time === 10) {
				workers[7].pendingRequests.clear();
				workers[3].pendingRequests.set("late-profile", {});
			}
			if (time === 30) workers[3].pendingRequests.clear();
		},
	});
	assert.equal(time, 30);
	assert.equal(result.workerCount, 8);
	assert.equal(result.pendingApiRequests, 0);
	workers[1].pendingRequests.set("stuck", {});
	await assert.rejects(
		waitForColdRequests(workers, {
			timeoutMs: 20,
			now: () => time,
			pause: async (ms) => {
				time += ms;
			},
		}),
		/did not finish/,
	);
});

test("each worker is checked through its own click, preserving all document proofs", () => {
	const boundary = createColdActionBoundary([0, 1], bootstraps(2));
	boundary.startedAtUnixMs = 100;
	const observations = [
		{ worker: 0, documentTimeOriginMs: 1_000, throughUnixMs: 150 },
		{ worker: 1, documentTimeOriginMs: 1_001, throughUnixMs: 200 },
	];
	const network = [
		{ worker: 0, category: "invoke", startedAtUnixMs: 160 },
		{ worker: 1, category: "prerun", startedAtUnixMs: 200 },
	];
	checkColdActionBoundary(boundary, network, observations, {
		throughClick: true,
	});
	assert.equal(boundary.unexpectedApiRequests, 0);
	assert.equal(boundary.documentPreserved, true);
	assert.equal(boundary.requestsCheckedThroughClick, true);
	assert.ok(
		boundary.workers.every((worker) => worker.requestsCheckedThroughClick),
	);
	network.push({
		worker: 1,
		category: "other_api_preflight",
		startedAtUnixMs: 175,
	});
	assert.throws(
		() =>
			checkColdActionBoundary(boundary, network, observations, {
				throughClick: true,
			}),
		/API request/,
	);
	assert.equal(boundary.workers[1].unexpectedApiRequests, 1);
	assert.equal(boundary.unexpectedApiRequests, 1);
	assert.throws(
		() => checkColdActionBoundary(boundary, [], [observations[0]]),
		/Every worker/,
	);
	assert.throws(
		() =>
			checkColdActionBoundary(boundary, [], [observations[0], observations[0]]),
		/each worker/,
	);
	assert.throws(
		() =>
			checkColdActionBoundary(
				boundary,
				[],
				[observations[0], { ...observations[1], documentTimeOriginMs: 2_001 }],
			),
		/navigated/,
	);
	assert.equal(boundary.documentPreserved, false);
	assert.equal(boundary.workers[0].documentPreserved, true);
	assert.equal(boundary.workers[1].documentPreserved, false);
});

test("controller receives one readiness marker and acknowledges across pipe chunks", async () => {
	const channel = controller();
	assert.deepEqual(JSON.parse(channel.written()), {
		type: "benchmark_cold_action_ready",
		worker: 0,
		navigation: 0,
	});
	channel.input.write('{"type":"benchmark_cold_action_');
	channel.input.write('continue"}\n');
	await channel.waiting;
	assert.equal(channel.written().trim().split("\n").length, 1);
	assert.equal(channel.input.listenerCount("data"), 0);
	assert.equal(channel.signals.listenerCount("SIGTERM"), 0);
	assert.equal(channel.input.isPaused(), true);
});

test("abort, malformed, unknown and oversized replies fail without echoing contents", async () => {
	for (const reply of [
		'{"type":"benchmark_cold_action_abort","error":"private controller data"}\n',
		"not JSON private controller data\n",
		'{"type":"execute","command":"private controller data"}\n',
		"x".repeat(4097),
		'{"type":"benchmark_cold_action_continue"}\n{"type":"extra"}\n',
	]) {
		const channel = controller();
		channel.input.write(reply);
		await assert.rejects(
			channel.waiting,
			(error) => !error.message.includes("private controller data"),
		);
		assert.equal(channel.input.listenerCount("data"), 0);
		assert.equal(channel.signals.listenerCount("SIGINT"), 0);
	}
});

test("EOF, closed pipes, signals and timeout release the waiting browser driver", async () => {
	for (const close of [
		(channel) => channel.input.end(),
		(channel) => channel.input.destroy(),
		(channel) => channel.signals.emit("SIGINT"),
		(channel) => channel.signals.emit("SIGTERM"),
	]) {
		const channel = controller();
		close(channel);
		await assert.rejects(channel.waiting);
		assert.equal(channel.signals.listenerCount("SIGTERM"), 0);
	}
	const channel = controller(10);
	await assert.rejects(channel.waiting, /timed out/);
});

test("cold boundary rejects all new API requests before click, including preflights", () => {
	const network = [
		{ worker: 0, category: "invoke", startedAtUnixMs: 90 },
		{ worker: 0, category: "other_api_preflight", startedAtUnixMs: 105 },
		{ worker: 0, category: "other_api", startedAtUnixMs: 110 },
		{ worker: 0, category: "prerun", startedAtUnixMs: 120 },
		{ worker: 1, category: "bootstrap", startedAtUnixMs: 125 },
		{ worker: 0, category: "invoke", startedAtUnixMs: 150 },
	];
	assert.deepEqual(coldBoundaryRequests(network, 0, 100, 150), {
		networkRequestCategories: {
			other_api_preflight: 1,
			other_api: 1,
			prerun: 1,
		},
		unexpectedApiRequests: 3,
		unexpectedExecutionRequests: 1,
	});
	assert.equal(
		coldBoundaryRequests(network, 0, 130, 150).unexpectedApiRequests,
		0,
	);
});
