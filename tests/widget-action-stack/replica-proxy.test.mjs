import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { createServer, request } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { pathCategory, startReplicaProxy } from "./replica-proxy.mjs";

const token = "private-test-control-token";
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function until(predicate) {
	const deadline = Date.now() + 3000;
	while (!predicate()) {
		if (Date.now() > deadline) assert.fail("Condition did not become true");
		await pause(5);
	}
}

function open(url, { method = "GET", headers = {}, body } = {}) {
	return new Promise((resolve, reject) => {
		const outgoing = request(
			url,
			{ method, headers, agent: false },
			(incoming) => {
				const result = {
					response: incoming,
					request: outgoing,
					status: incoming.statusCode,
					headers: incoming.headers,
					text: "",
				};
				result.done = new Promise((done, fail) => {
					incoming.on("data", (chunk) => {
						result.text += chunk.toString();
					});
					incoming.on("end", () => done(result));
					incoming.on("error", fail);
				});
				result.done.catch(() => {});
				resolve(result);
			},
		);
		outgoing.on("error", reject);
		outgoing.end(body);
	});
}

async function get(url, options) {
	return (await open(url, options)).done;
}

async function setup(t, handlers = {}, options = {}) {
	const directory = await mkdtemp(join(tmpdir(), "flow-replica-proxy-test-"));
	const backends = [];
	const pools = {};
	let proxy = null;
	t.after(async () => {
		await proxy?.close({ force: true });
		await Promise.all(
			backends.map(
				(server) =>
					new Promise((resolve) => {
						server.close(resolve);
						server.closeAllConnections();
					}),
			),
		);
		await rm(directory, { recursive: true, force: true });
	});
	for (const pool of ["api_frontend", "api_callback", "runtime"]) {
		const chosen =
			handlers[pool] ?? ((_request, response) => response.end("ok"));
		const targets = [];
		for (const [index, handler] of (Array.isArray(chosen)
			? chosen
			: [chosen]
		).entries()) {
			const server = createServer((request, response) => {
				request.on("error", () => {});
				request.resume();
				handler(request, response);
			});
			backends.push(server);
			await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
			targets.push({
				id: `${pool}-${index}`,
				url: `http://127.0.0.1:${server.address().port}`,
				generation: 0,
			});
		}
		pools[pool] = { port: 0, targets };
	}
	const logFile = join(directory, "requests.jsonl");
	const config = {
		pools,
		logFile,
		controlPort: 0,
		controlToken: token,
		...options,
	};
	proxy = await startReplicaProxy(config);
	return {
		proxy,
		config,
		url: proxy.addresses,
		logs: async () =>
			(await readFile(logFile, "utf8"))
				.trim()
				.split("\n")
				.filter(Boolean)
				.map(JSON.parse),
		control: (path, body, suppliedToken = token) =>
			get(`${proxy.addresses.control}${path}`, {
				method: body === undefined ? "GET" : "POST",
				headers: {
					authorization: `Bearer ${suppliedToken}`,
					"content-type": "application/json",
				},
				body: body === undefined ? undefined : JSON.stringify(body),
			}),
	};
}

test("holds a replica beyond SSE completed until upstream EOF", async (t) => {
	let firstResponse;
	let calls = 0;
	const harness = await setup(t, {
		runtime: (_request, response) => {
			calls++;
			if (calls === 1) {
				firstResponse = response;
				response.writeHead(200, { "content-type": "text/event-stream" });
				response.write('event: completed\ndata: {"done":true}\n\n');
			} else response.end("second");
		},
	});
	const first = await open(`${harness.url.runtime}/execute/sse`);
	await until(() => first.text.includes("completed"));
	const second = get(`${harness.url.runtime}/execute/sse`);
	await until(() => harness.proxy.status().pools.runtime.queued === 1);
	assert.equal(calls, 1);
	assert.equal(harness.proxy.status().pools.runtime.active, 1);
	firstResponse.end("event: accounting_finished\ndata: {}\n\n");
	await first.done;
	assert.equal((await second).text, "second");
	const logs = await harness.logs();
	assert.deepEqual(
		logs.map((row) => row.event),
		["admitted", "released", "admitted", "released"],
	);
	assert.ok(
		BigInt(logs[2].admittedAtMonotonicNs) >=
			BigInt(logs[1].releasedAtMonotonicNs),
	);
	assert.ok(
		logs
			.filter((row) => row.event === "released")
			.every((row) => row.upstreamComplete),
	);
});

test("drains upstream after downstream disconnect before reusing the replica", async (t) => {
	let held;
	let calls = 0;
	const harness = await setup(t, {
		runtime: (_request, response) => {
			calls++;
			if (calls === 1) {
				held = response;
				response.write("running\n");
			} else response.end("next");
		},
	});
	const first = await open(`${harness.url.runtime}/execute/sse`);
	first.response.destroy();
	const next = get(`${harness.url.runtime}/execute/sse`);
	await until(() => harness.proxy.status().pools.runtime.queued === 1);
	await pause(20);
	assert.equal(calls, 1);
	assert.equal(harness.proxy.status().pools.runtime.active, 1);
	assert.equal(held.destroyed, false);
	held.end("final accounting\n");
	assert.equal((await next).text, "next");
	const release = (await harness.logs()).find(
		(row) => row.event === "released",
	);
	assert.equal(release.downstreamDisconnected, true);
	assert.equal(release.upstreamComplete, true);
	assert.equal(release.failureReason, null);
});

test("failed stream quarantines its replica and fails queued and future pool traffic", async (t) => {
	let held;
	const harness = await setup(t, {
		runtime: (request, response) => {
			if (request.url === "/broken") {
				held = response;
				response.write("partial");
			} else response.end("recovered");
		},
	});
	const broken = await open(`${harness.url.runtime}/broken`);
	const queued = get(`${harness.url.runtime}/next`);
	await until(() => harness.proxy.status().pools.runtime.queued === 1);
	held.destroy();
	await assert.rejects(broken.done);
	assert.equal((await queued).status, 503);
	assert.equal((await get(`${harness.url.runtime}/next`)).status, 503);
	assert.equal(harness.proxy.status().pools.runtime.quarantined, 1);
	const release = (await harness.logs()).find(
		(row) => row.event === "released",
	);
	assert.equal(release.upstreamComplete, false);
	assert.match(release.failureReason, /^upstream_/);
	assert.equal((await harness.control("/pause", {})).status, 200);
	assert.equal(
		(await harness.control("/generation", { generation: 1 })).status,
		200,
	);
	const recovered = await get(`${harness.url.runtime}/next`);
	assert.equal(recovered.text, "recovered");
	assert.equal(recovered.headers["x-flowlike-benchmark-generation"], "1");
});

test("callback pool keeps serving while the foreground API stream holds its slot", async (t) => {
	let held;
	const harness = await setup(t, {
		api_frontend: (_request, response) => {
			held = response;
			response.write("streaming");
		},
	});
	const foreground = await open(
		`${harness.url.api_frontend}/api/v1/apps/app/events/event/invoke`,
	);
	const callback = await get(
		`${harness.url.api_callback}/api/v1/execution/quota`,
		{ method: "POST", body: "{}" },
	);
	assert.equal(callback.status, 200);
	assert.equal(harness.proxy.status().pools.api_frontend.active, 1);
	assert.equal(harness.proxy.status().pools.api_callback.active, 0);
	held.end();
	await foreground.done;
	const admitted = (await harness.logs()).filter(
		(row) => row.event === "admitted",
	);
	assert.deepEqual(
		admitted.map((row) => [row.pool, row.pathCategory]),
		[
			["api_frontend", "frontend_invoke"],
			["api_callback", "execution_quota"],
		],
	);
});

test("FIFO queue uses any idle replica and never overlaps a replica lease", async (t) => {
	const held = new Map();
	const arrivals = [];
	const active = [0, 0];
	const maximum = [0, 0];
	const handlers = [0, 1].map((index) => (request, response) => {
		active[index]++;
		maximum[index] = Math.max(maximum[index], active[index]);
		const order = Number(request.headers["x-order"]);
		arrivals.push([order, index]);
		held.set(order, () => {
			active[index]--;
			response.end(String(order));
		});
		response.write("started:");
	});
	const harness = await setup(t, { runtime: handlers });
	const one = await open(`${harness.url.runtime}/execute/sse`, {
		headers: { "x-order": "1" },
	});
	const two = await open(`${harness.url.runtime}/execute/sse`, {
		headers: { "x-order": "2" },
	});
	const three = open(`${harness.url.runtime}/execute/sse`, {
		headers: { "x-order": "3" },
	});
	await until(() => harness.proxy.status().pools.runtime.queued === 1);
	const four = open(`${harness.url.runtime}/execute/sse`, {
		headers: { "x-order": "4" },
	});
	await until(() => harness.proxy.status().pools.runtime.queued === 2);
	held.get(2)();
	await three;
	await until(() => held.has(3));
	assert.deepEqual(arrivals, [
		[1, 0],
		[2, 1],
		[3, 1],
	]);
	held.get(3)();
	const fourth = await four;
	held.get(4)();
	held.get(1)();
	await Promise.all([one.done, two.done, (await three).done, fourth.done]);
	assert.deepEqual(arrivals, [
		[1, 0],
		[2, 1],
		[3, 1],
		[4, 1],
	]);
	assert.deepEqual(maximum, [1, 1]);
	const inFlight = new Set();
	for (const row of await harness.logs()) {
		if (row.event === "admitted") {
			assert.equal(inFlight.has(row.replica), false);
			inFlight.add(row.replica);
		} else {
			assert.equal(inFlight.has(row.replica), true);
			inFlight.delete(row.replica);
		}
	}
	assert.equal(inFlight.size, 0);
});

test("bounded queue rejects overflow and times out without poisoning a healthy slot", async (t) => {
	let held;
	let count = 0;
	const harness = await setup(
		t,
		{
			runtime: (_request, response) => {
				if (++count === 1) {
					held = response;
					response.write("held");
				} else response.end("ok");
			},
		},
		{ queueLimit: 1, queueTimeoutMs: 80 },
	);
	const first = await open(`${harness.url.runtime}/execute/sse`);
	const second = get(`${harness.url.runtime}/execute/sse`);
	await until(() => harness.proxy.status().pools.runtime.queued === 1);
	assert.equal((await get(`${harness.url.runtime}/execute/sse`)).status, 503);
	assert.equal((await second).status, 504);
	assert.equal(count, 1);
	assert.equal(harness.proxy.status().pools.runtime.quarantined, 0);
	held.end();
	await first.done;
	assert.equal((await get(`${harness.url.runtime}/execute/sse`)).status, 200);
});

test("total upstream timeout quarantines a request that never reaches EOF", async (t) => {
	const harness = await setup(
		t,
		{ runtime: (_request, response) => response.write("held") },
		{ requestTimeoutMs: 60 },
	);
	const first = await open(`${harness.url.runtime}/execute/sse`);
	await assert.rejects(first.done);
	await until(() => harness.proxy.status().pools.runtime.quarantined === 1);
	assert.equal((await harness.logs()).at(-1).failureReason, "upstream_timeout");
});

test("control requires authentication and paused idle pools before generation changes", async (t) => {
	let held;
	const harness = await setup(t, {
		runtime: (_request, response) => {
			held = response;
			response.write("held");
		},
	});
	assert.equal((await get(`${harness.url.control}/health`)).status, 200);
	assert.equal((await get(`${harness.url.control}/status`)).status, 401);
	assert.equal((await harness.control("/pause", {}, "wrong")).status, 401);
	assert.equal(
		(await harness.control("/generation", { generation: 1 })).status,
		409,
	);
	const first = await open(`${harness.url.runtime}/execute/sse`);
	assert.equal((await harness.control("/pause", {})).status, 409);
	held.end();
	await first.done;
	assert.equal((await harness.control("/pause", {})).status, 200);
	assert.equal(
		JSON.parse((await harness.control("/status")).text).paused,
		true,
	);
	assert.equal((await get(`${harness.url.runtime}/execute/sse`)).status, 503);
	assert.equal((await harness.logs()).at(-1).failureReason, "admission_paused");
	assert.equal(
		(await harness.control("/generation", { generation: 0 })).status,
		409,
	);
	assert.equal(
		(await harness.control("/generation", { generation: 1, targets: [] }))
			.status,
		400,
	);
	assert.equal(
		(await harness.control("/generation", { generation: "round-1" })).status,
		200,
	);
	const status = JSON.parse((await harness.control("/status")).text);
	assert.equal(status.paused, false);
	assert.ok(
		Object.values(status.pools).every((pool) =>
			pool.targets.every((target) => target.generation === "round-1"),
		),
	);
});

test("forwards bodies and authorization, strips hop headers, and keeps logs free of request secrets", async (t) => {
	let seen;
	const harness = await setup(t, {
		runtime: (request, response) => {
			seen = { headers: request.headers, path: request.url, body: "" };
			request.on("data", (chunk) => {
				seen.body += chunk;
			});
			request.on("end", () => {
				response.writeHead(200, {
					connection: "x-response-secret",
					"x-response-secret": "private-hop",
					"set-cookie": ["a=1", "b=2"],
				});
				response.end("ok");
			});
		},
	});
	const response = await get(
		`${harness.url.runtime}/execute/sse?secret=private-query`,
		{
			method: "POST",
			body: "private-payload",
			headers: {
				authorization: "Bearer private-auth",
				connection: "x-request-secret",
				"x-request-secret": "private-hop",
			},
		},
	);
	assert.equal(seen.headers.authorization, "Bearer private-auth");
	assert.equal(seen.headers["x-request-secret"], undefined);
	assert.equal(seen.path, "/execute/sse?secret=private-query");
	assert.equal(seen.body, "private-payload");
	assert.equal(response.headers["x-response-secret"], undefined);
	assert.deepEqual(response.headers["set-cookie"], ["a=1", "b=2"]);
	assert.equal(response.headers["x-flowlike-benchmark-replica"], "runtime-0");
	const logs = await harness.logs();
	assert.equal(
		response.headers["x-flowlike-benchmark-request-id"],
		logs[0].requestId,
	);
	assert.match(logs[0].routeKey, /^[0-9a-f]{64}$/);
	assert.equal(logs[0].pathCategory, "executor_sse");
	assert.equal(JSON.stringify(logs).includes("private-"), false);
	assert.equal(JSON.stringify(logs).includes(token), false);
});

test("rejects non-loopback, shared, and credential-bearing target configuration", async (t) => {
	const harness = await setup(t);
	for (const url of [
		"http://example.com:80",
		"https://127.0.0.1:1234",
		"http://user:password@127.0.0.1:1234",
		"http://127.0.0.1:1234/path",
	]) {
		const config = structuredClone(harness.config);
		config.pools.runtime.targets[0].url = url;
		await assert.rejects(startReplicaProxy(config));
	}
	const duplicate = structuredClone(harness.config);
	duplicate.pools.runtime.targets[0].url =
		duplicate.pools.api_callback.targets[0].url;
	await assert.rejects(startReplicaProxy(duplicate), /multiple slots/);
	assert.equal(
		pathCategory("/api/v1/execution/apps/private-app/widgets"),
		"execution_widgets",
	);
	assert.equal(
		pathCategory("/api/v1/channels/private-id/messages/private-request"),
		"execution_channel",
	);
});

test("classifies only the exact telemetry config path while preserving method and response status", async (t) => {
	const harness = await setup(t, {
		api_frontend: (_request, response) => {
			response.writeHead(404);
			response.end("missing");
		},
	});
	for (const method of ["GET", "POST"]) {
		const result = await get(
			`${harness.url.api_frontend}/api/v1/telemetry/config?sampling=private-query`,
			{ method },
		);
		assert.equal(result.status, 404);
	}
	const releases = (await harness.logs()).filter(
		(row) => row.event === "released",
	);
	assert.deepEqual(
		releases.map((row) => row.method),
		["GET", "POST"],
	);
	assert.deepEqual(
		releases.map((row) => row.pathCategory),
		["frontend_telemetry_config", "other"],
	);
	assert.ok(
		releases.every((row) => row.status === 404 && row.upstreamComplete),
	);
	assert.equal(pathCategory("/api/v1/telemetry/config"), "other");
	assert.equal(pathCategory("/api/v1/telemetry/config", "HEAD"), "other");
	assert.equal(pathCategory("/api/v1/telemetry/config/", "GET"), "other");
	assert.equal(pathCategory("/api/v1/telemetry/configuration", "GET"), "other");
	assert.equal(JSON.stringify(releases).includes("private-query"), false);
});

test("streams large request and response bodies with backpressure", async (t) => {
	const payload = Buffer.alloc(2 * 1024 * 1024, "x");
	let received = 0;
	let observedBackpressure = false;
	const harness = await setup(t, {
		runtime: async (incoming, response) => {
			incoming.on("data", (chunk) => {
				received += chunk.length;
			});
			incoming.on("end", async () => {
				for (let offset = 0; offset < payload.length; offset += 65536) {
					if (!response.write(payload.subarray(offset, offset + 65536))) {
						observedBackpressure = true;
						await new Promise((resolve) => response.once("drain", resolve));
					}
				}
				response.end();
			});
		},
	});
	const result = await get(`${harness.url.runtime}/execute`, {
		method: "POST",
		body: payload,
		headers: { "content-length": String(payload.length) },
	});
	assert.equal(received, payload.length);
	assert.equal(result.text, payload.toString());
	assert.equal(observedBackpressure, true);
	assert.equal((await harness.logs()).at(-1).upstreamComplete, true);
});

test("aborted upload quarantines instead of reusing an uncertain invocation", async (t) => {
	let received = false;
	const harness = await setup(t, {
		runtime: (incoming) => {
			incoming.on("data", () => {
				received = true;
			});
		},
	});
	const outgoing = request(`${harness.url.runtime}/execute`, {
		method: "POST",
		headers: { "content-length": "1000" },
		agent: false,
	});
	outgoing.on("error", () => {});
	outgoing.write("partial");
	await until(() => received);
	outgoing.destroy();
	await until(() => harness.proxy.status().pools.runtime.quarantined === 1);
	assert.equal(
		(await harness.logs()).at(-1).failureReason,
		"downstream_upload_aborted",
	);
	assert.equal((await get(`${harness.url.runtime}/execute`)).status, 503);
});
