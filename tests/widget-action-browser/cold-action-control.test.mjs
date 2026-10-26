import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { PassThrough } from "node:stream";
import test from "node:test";
import {
	assertColdActionConfiguration,
	coldBoundaryRequests,
	waitForColdActionController,
} from "./cold-action-control.mjs";

function controller(timeoutMs = 1000) {
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
