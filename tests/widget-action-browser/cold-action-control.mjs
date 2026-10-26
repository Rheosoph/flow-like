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
		config.samples === 1 &&
			config.navigations === 1 &&
			config.concurrency === 1 &&
			config.warmup === 0 &&
			config.requireActionTrace === true &&
			config.requireTerminalSuccess === true,
		"Cold-action control requires one verified action after bootstrap, with no warmup",
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
} = {}) {
	assert.ok(
		Number.isInteger(timeoutMs) && timeoutMs > 0 && timeoutMs <= 2_147_483_647,
		"Cold-action timeout must be a positive integer within the timer range",
	);
	return new Promise((resolve, reject) => {
		const decoder = new StringDecoder("utf8");
		let buffer = "";
		let settled = false;
		let timer;
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
		timer = setTimeout(
			() => fail("Cold-action backend restart confirmation timed out"),
			timeoutMs,
		);
		if (input.readableEnded || input.destroyed) return ended();
		input.resume();
		try {
			output.write(
				`${JSON.stringify({ type: "benchmark_cold_action_ready", worker: 0, navigation: 0 })}\n`,
			);
		} catch {
			failed();
		}
	});
}
