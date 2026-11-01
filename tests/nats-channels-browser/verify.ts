import assert from "node:assert/strict";
import { type ChildProcessWithoutNullStreams, spawn } from "node:child_process";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import puppeteer from "puppeteer";
import type { IChannelHandle } from "../../packages/ui/lib/schema/channel";
import type { channels } from "./main";

declare global {
	interface Window {
		natsChannels: typeof channels;
	}
}

interface Config {
	url: string;
	ws_url: string;
	account_seed: string;
	channel_id: string;
}
interface WorkerEvent {
	event: string;
	id?: string;
	handle?: IChannelHandle;
	value?: unknown;
	values?: unknown[];
	accepted?: boolean;
}
const configIndex = process.argv.indexOf("--config");
assert.ok(
	configIndex >= 0 && process.argv[configIndex + 1],
	"Pass --config with a JSON file containing url, ws_url, account_seed, channel_id.",
);
const config: Config = JSON.parse(
	await readFile(process.argv[configIndex + 1], "utf8"),
);
const workerBinary =
	process.env.NATS_WORKER_BIN ||
	resolve("target/debug/examples/nats-channel-worker");
const directory = await mkdtemp(join(tmpdir(), "flow-like-nats-browser-"));
const workers: ChildProcessWithoutNullStreams[] = [];

function startWorker(input: Config & { ttl_secs?: number }) {
	const child = spawn(workerBinary, [], { stdio: "pipe" });
	workers.push(child);
	const events: WorkerEvent[] = [];
	const consumed = new Set<WorkerEvent>();
	const pending = new Set<() => void>();
	let failure: Error | undefined;
	child.on("error", (error) => {
		failure = error;
		for (const check of pending) check();
	});
	child.on("exit", (code) => {
		if (code && code !== 0)
			failure = new Error(`Rust NATS fixture exited with code ${code}`);
		for (const check of pending) check();
	});
	child.stderr.on("data", () => {});
	createInterface({ input: child.stdout }).on("line", (line) => {
		try {
			events.push(JSON.parse(line));
		} catch {
			failure = new Error("Rust NATS fixture returned invalid JSON");
		}
		for (const check of pending) check();
	});
	function next(event: string, id?: string): Promise<WorkerEvent> {
		return new Promise((resolve, reject) => {
			const timeout = setTimeout(
				() => finish(new Error(`Timed out waiting for fixture event ${event}`)),
				15_000,
			);
			const finish = (result: WorkerEvent | Error) => {
				clearTimeout(timeout);
				pending.delete(check);
				if (result instanceof Error) reject(result);
				else resolve(result);
			};
			const check = () => {
				const found = events.find(
					(value) =>
						!consumed.has(value) &&
						value.event === event &&
						(id === undefined || value.id === id),
				);
				if (found) {
					consumed.add(found);
					finish(found);
				} else if (failure) finish(failure);
			};
			pending.add(check);
			check();
		});
	}
	child.stdin.write(`${JSON.stringify(input)}\n`);
	return { child, events, next };
}

const build = await Bun.build({
	entrypoints: [join(import.meta.dir, "main.ts")],
	outdir: directory,
	target: "browser",
	format: "esm",
	splitting: true,
});
assert.ok(build.success, build.logs.join("\n"));
const worker = startWorker(config);
let fallbackCalls = 0;
const fallbackIds: string[] = [];
const server = Bun.serve({
	hostname: "127.0.0.1",
	port: Number(process.env.NATS_E2E_BROWSER_PORT || 0),
	async fetch(request) {
		const path = new URL(request.url).pathname;
		if (path === "/")
			return new Response(
				'<!doctype html><script type="module" src="/main.js"></script>',
				{ headers: { "content-type": "text/html" } },
			);
		if (path === "/push" && request.method === "POST") {
			assert.equal(
				request.headers.get("authorization"),
				"Bearer fixture-http-grant",
			);
			const id = request.headers.get("x-channel-delivery-id");
			assert.ok(id, "HTTP fallback must preserve its delivery ID");
			fallbackCalls++;
			fallbackIds.push(id);
			const result = worker.next("forwarded", id);
			worker.child.stdin.write(
				`${JSON.stringify({ command: "forward", id, push: await request.json() })}\n`,
			);
			const ack = await result;
			return Response.json(
				{ accepted: ack.accepted },
				{ status: ack.accepted ? 200 : 409 },
			);
		}
		if (!/^\/[\w.-]+\.js$/.test(path))
			return new Response("Missing", { status: 404 });
		const file = Bun.file(join(directory, path.slice(1)));
		return (await file.exists())
			? new Response(file)
			: new Response("Missing", { status: 404 });
	},
});
const browser = await puppeteer.launch({
	headless: true,
	...(process.env.CHROME_PATH
		? { executablePath: process.env.CHROME_PATH }
		: {}),
});
try {
	const handle = (await worker.next("ready")).handle;
	assert.ok(handle);
	const page = await browser.newPage();
	const browserErrors: string[] = [];
	page.on("pageerror", (error) => browserErrors.push(String(error)));
	await page.goto(server.url.origin);
	await page.waitForFunction(() => !!window.natsChannels);
	assert.equal(
		await page.evaluate(
			(handle) =>
				window.natsChannels.permissionRejected(
					handle,
					"subscribe",
					"channel.another-run.inbox",
				),
			handle,
		),
		true,
		"The broker must reject reading another run",
	);
	assert.equal(
		await page.evaluate(
			(handle) =>
				window.natsChannels.permissionRejected(
					handle,
					"publish",
					"channel.another-run.inbox",
				),
			handle,
		),
		true,
		"The broker must reject writing another run",
	);
	assert.equal(
		await page.evaluate(
			(handle) =>
				window.natsChannels.permissionRejected(
					handle,
					"subscribe",
					handle.transport.type === "nats" ? handle.transport.subject : "",
				),
			handle,
		),
		true,
		"The browser grant must not read the executor inbox",
	);
	await page.evaluate(
		(handle) =>
			window.natsChannels.push(
				handle,
				{
					request_id: handle.request_id,
					kind: "reply",
					value: { approved: true },
				},
				{ deliveryId: "browser-reply" },
			),
		handle,
	);
	assert.deepEqual((await worker.next("reply")).value, { approved: true });
	await page.evaluate(
		(handle) =>
			window.natsChannels.push(
				handle,
				{ kind: "inbound", value: "browser steering" },
				{ deliveryId: "browser-inbound" },
			),
		handle,
	);
	assert.deepEqual((await worker.next("inbound")).values, ["browser steering"]);
	await page.evaluate(
		(handle) =>
			window.natsChannels.push(
				handle,
				{ kind: "inbound", value: "browser steering" },
				{ deliveryId: "browser-inbound" },
			),
		handle,
	);
	const fallbackHandle = {
		...handle,
		fallback: {
			type: "http" as const,
			push_url: `${server.url.origin}/push`,
			token: "fixture-http-grant",
		},
	};
	assert.equal(fallbackHandle.transport.type, "nats");
	if (fallbackHandle.transport.type === "nats")
		fallbackHandle.transport = {
			...fallbackHandle.transport,
			url: `ws://127.0.0.1:${server.port}/unavailable`,
		};
	await page.evaluate(
		(handle) =>
			window.natsChannels.push(
				handle,
				{ kind: "inbound", value: "fallback steering" },
				{ deliveryId: "browser-fallback" },
			),
		fallbackHandle,
	);
	assert.deepEqual((await worker.next("inbound")).values, [
		"fallback steering",
	]);
	assert.equal(fallbackCalls, 1);
	assert.deepEqual(fallbackIds, ["browser-fallback"]);
	// A duplicate arriving through a different transport must not enqueue a second message.
	await page.evaluate(
		(handle) =>
			window.natsChannels.push(
				handle,
				{ kind: "inbound", value: "fallback steering" },
				{ deliveryId: "browser-fallback" },
			),
		handle,
	);
	await page.evaluate(
		(handle) =>
			window.natsChannels.push(
				handle,
				{ kind: "cancel", value: null },
				{ deliveryId: "browser-cancel" },
			),
		handle,
	);
	await worker.next("cancelled");
	assert.deepEqual(
		worker.events
			.filter((event) => event.event === "inbound")
			.flatMap((event) => event.values ?? []),
		["browser steering", "fallback steering"],
	);
	await page.evaluate(() => window.natsChannels.reset());
	const expiring = startWorker({
		...config,
		channel_id: `${config.channel_id}-expiry`,
		ttl_secs: 2,
	});
	const expiredHandle = (await expiring.next("ready")).handle;
	assert.ok(expiredHandle);
	await expiring.next("expired");
	// NATS checks JWT timestamps in whole seconds; move past the expiration second.
	await new Promise((resolve) => setTimeout(resolve, 1_100));
	assert.equal(
		await page.evaluate(
			(handle) => window.natsChannels.connectionRejected(handle),
			expiredHandle,
		),
		true,
		"The broker must reject an expired signed grant",
	);
	assert.deepEqual(browserErrors, []);
	console.log(
		"NATS browser E2E passed: scoped permissions, reply ACK, steering, retry deduplication, HTTP forwarder fallback, cancel, expired credentials.",
	);
} finally {
	await browser.close();
	server.stop(true);
	for (const child of workers) {
		child.stdin.end();
		if (child.exitCode === null) child.kill();
	}
	await rm(directory, { recursive: true, force: true });
}
