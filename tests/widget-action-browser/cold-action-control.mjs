import assert from "node:assert/strict";
import { StringDecoder } from "node:string_decoder";

export function assertColdActionConfiguration(config, app, stdin) {
	assert.ok(
		["localhost", "127.0.0.1", "[::1]"].includes(app.hostname) &&
			!config.cdpUrl &&
			!stdin.isTTY,
		"Cold-action control requires a local app, an owned browser, and piped stdin",
	);
	assert.ok(
		Number.isInteger(config.concurrency) &&
			config.concurrency > 0 &&
			config.samples === config.concurrency &&
			config.navigations === 1 &&
			config.warmup === 0 &&
			config.requireActionTrace === true &&
			config.requireTerminalSuccess === true,
		"Cold-action control requires one verified action per worker after bootstrap, with no warmup",
	);
}

function assertWorkerIds(workerIds) {
	assert.ok(
		Array.isArray(workerIds) &&
			workerIds.length > 0 &&
			workerIds.every((worker, index) => worker === index),
		"Cold-action workers must contain each worker from zero exactly once in order",
	);
}

export async function waitForColdRequests(
	workers,
	{
		timeoutMs = 30_000,
		now = () => performance.now(),
		pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
	} = {},
) {
	assertWorkerIds(workers.map((worker) => worker.id));
	assert.ok(Number.isFinite(timeoutMs) && timeoutMs > 0);
	assert.ok(workers.every((worker) => worker.pendingRequests instanceof Map));
	const started = now();
	let polls = 0;
	while (true) {
		polls++;
		if (workers.every((worker) => worker.pendingRequests.size === 0))
			return {
				status: "confirmed",
				workerCount: workers.length,
				pendingApiRequests: 0,
				durationMs: now() - started,
				polls,
			};
		const remaining = timeoutMs - (now() - started);
		assert.ok(
			remaining > 0,
			"API requests did not finish before the cold-action boundary",
		);
		await pause(Math.min(10, remaining));
	}
}

export function createColdActionBoundary(
	workerIds,
	bootstraps,
	navigation = 0,
) {
	assertWorkerIds(workerIds);
	assert.equal(
		navigation,
		0,
		"Cold-action control requires the first navigation",
	);
	const selected = bootstraps.filter((entry) => entry.index === navigation);
	assert.equal(
		selected.length,
		workerIds.length,
		"Every worker needs one verified bootstrap",
	);
	const workers = workerIds.map((worker) => {
		const matches = selected.filter((entry) => entry.worker === worker);
		assert.equal(
			matches.length,
			1,
			"Every worker needs one verified bootstrap",
		);
		const bootstrap = matches[0];
		assert.ok(
			bootstrap.status === "ok" &&
				bootstrap.executionSuccessVerified === true &&
				Number.isFinite(bootstrap.browserTimeOriginMs),
			"Every worker needs successful bootstrap and document evidence",
		);
		return {
			worker,
			navigation,
			bootstrapExecutionSuccessVerified: true,
			documentTimeOriginMs: bootstrap.browserTimeOriginMs,
		};
	});
	const startedAtUnixMs = Date.now();
	return {
		mode: "cold-action",
		status: "waiting",
		navigation,
		workerCount: workers.length,
		workers,
		bootstrapExecutionSuccessVerified: true,
		startedAt: new Date(startedAtUnixMs).toISOString(),
		startedAtUnixMs,
		...(workers.length === 1
			? { worker: 0, documentTimeOriginMs: workers[0].documentTimeOriginMs }
			: {}),
	};
}

export function checkColdActionBoundary(
	boundary,
	network,
	observations,
	{ throughClick = false } = {},
) {
	assertWorkerIds(boundary.workers.map((worker) => worker.worker));
	assertWorkerIds(observations.map((worker) => worker.worker));
	assert.equal(
		observations.length,
		boundary.workers.length,
		"Every worker needs boundary evidence",
	);
	for (const [index, observation] of observations.entries()) {
		assert.ok(
			Number.isFinite(observation.throughUnixMs) &&
				observation.throughUnixMs >= boundary.startedAtUnixMs,
			"Every worker needs a valid boundary endpoint",
		);
		const worker = boundary.workers[index];
		Object.assign(
			worker,
			coldBoundaryRequests(
				network,
				worker.worker,
				boundary.startedAtUnixMs,
				observation.throughUnixMs,
			),
			{
				documentPreserved:
					observation.documentTimeOriginMs === worker.documentTimeOriginMs,
				requestsCheckedThroughClick: throughClick,
				checkedThroughUnixMs: observation.throughUnixMs,
			},
		);
	}
	boundary.documentPreserved = boundary.workers.every(
		(worker) => worker.documentPreserved,
	);
	boundary.requestsCheckedThroughClick = boundary.workers.every(
		(worker) => worker.requestsCheckedThroughClick,
	);
	boundary.unexpectedApiRequests = boundary.workers.reduce(
		(sum, worker) => sum + worker.unexpectedApiRequests,
		0,
	);
	boundary.unexpectedExecutionRequests = boundary.workers.reduce(
		(sum, worker) => sum + worker.unexpectedExecutionRequests,
		0,
	);
	boundary.networkRequestCategories = {};
	for (const worker of boundary.workers)
		for (const [category, count] of Object.entries(
			worker.networkRequestCategories,
		))
			boundary.networkRequestCategories[category] =
				(boundary.networkRequestCategories[category] ?? 0) + count;
	assert.ok(
		boundary.documentPreserved,
		"A browser navigated during the cold-action boundary",
	);
	assert.equal(
		boundary.unexpectedApiRequests,
		0,
		"The app issued an API request before its cold-action click",
	);
}

export function coldBoundaryRequests(
	network,
	worker,
	startedAtUnixMs,
	throughUnixMs,
) {
	const requests = network.filter(
		(record) =>
			record.worker === worker &&
			record.startedAtUnixMs >= startedAtUnixMs &&
			record.startedAtUnixMs < throughUnixMs,
	);
	return {
		networkRequestCategories: Object.fromEntries(
			[...new Set(requests.map((record) => record.category))].map(
				(category) => [
					category,
					requests.filter((record) => record.category === category).length,
				],
			),
		),
		unexpectedApiRequests: requests.length,
		unexpectedExecutionRequests: requests.filter((record) =>
			["bootstrap", "prerun", "invoke"].includes(record.category),
		).length,
	};
}

/** A fixed protocol lets the local stack runner restart its owned services. */
export function waitForColdActionController({
	input = process.stdin,
	output = process.stdout,
	signals = process,
	timeoutMs = 180_000,
	workerIds = [0],
} = {}) {
	assertWorkerIds(workerIds);
	const marker = {
		type: "benchmark_cold_action_ready",
		navigation: 0,
		...(workerIds.length === 1 ? { worker: 0 } : { workers: [...workerIds] }),
	};
	assert.ok(
		Number.isInteger(timeoutMs) && timeoutMs > 0 && timeoutMs <= 2_147_483_647,
		"Cold-action timeout must be a positive integer within the timer range",
	);
	return new Promise((resolve, reject) => {
		const decoder = new StringDecoder("utf8");
		let buffer = "";
		let settled = false;
		const finish = (error) => {
			if (settled) return;
			settled = true;
			clearTimeout(timer);
			input.off("data", receive);
			input.off("end", ended);
			input.off("close", ended);
			input.off("error", failed);
			output.off("error", failed);
			signals.off("SIGINT", interrupted);
			signals.off("SIGTERM", interrupted);
			input.pause();
			if (error) reject(error);
			else resolve();
		};
		const fail = (message) => finish(new Error(message));
		const ended = () =>
			fail("Cold-action controller closed before confirmation");
		const failed = () => fail("Cold-action controller pipe failed");
		const interrupted = () => fail("Cold-action control was interrupted");
		const receive = (chunk) => {
			buffer += decoder.write(
				Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk),
			);
			if (Buffer.byteLength(buffer) > 4096)
				return fail("Cold-action controller reply exceeded its size limit");
			const newline = buffer.indexOf("\n");
			if (newline < 0) return;
			if (buffer.slice(newline + 1).trim())
				return fail("Cold-action controller sent more than one reply");
			let message;
			try {
				message = JSON.parse(buffer.slice(0, newline));
			} catch {
				return fail("Cold-action controller sent an invalid reply");
			}
			if (message?.type === "benchmark_cold_action_abort")
				return fail(
					"Cold-action backend restart was aborted by the controller",
				);
			if (message?.type !== "benchmark_cold_action_continue")
				return fail("Cold-action controller sent an unexpected reply");
			finish();
		};
		input.on("data", receive);
		input.once("end", ended);
		input.once("close", ended);
		input.once("error", failed);
		output.once("error", failed);
		signals.once("SIGINT", interrupted);
		signals.once("SIGTERM", interrupted);
		const timer = setTimeout(
			() => fail("Cold-action backend restart confirmation timed out"),
			timeoutMs,
		);
		if (input.readableEnded || input.destroyed) return ended();
		input.resume();
		try {
			output.write(`${JSON.stringify(marker)}\n`);
		} catch {
			failed();
		}
	});
}
