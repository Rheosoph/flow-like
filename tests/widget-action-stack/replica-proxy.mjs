import { createHash, randomUUID, timingSafeEqual } from "node:crypto";
import { closeSync, openSync, readFileSync, writeSync } from "node:fs";
import { createServer, request as httpRequest } from "node:http";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const POOLS = new Set(["api_frontend", "api_callback", "runtime"]);
const HOP_HEADERS = new Set([
	"connection",
	"keep-alive",
	"proxy-authenticate",
	"proxy-authorization",
	"te",
	"trailer",
	"transfer-encoding",
	"upgrade",
	"proxy-connection",
]);
const stamp = () => ({
	unixMs: Date.now(),
	monotonicNs: process.hrtime.bigint().toString(),
});
const digest = (value) => createHash("sha256").update(value).digest();

function positiveInteger(value, fallback, name) {
	const result = value ?? fallback;
	if (!Number.isSafeInteger(result) || result < 1)
		throw new Error(`Invalid ${name}`);
	return result;
}

function validPort(port) {
	return Number.isInteger(port) && port >= 0 && port <= 65535;
}

function validGeneration(value) {
	return (
		(typeof value === "string" && /^[A-Za-z0-9_.-]{1,128}$/.test(value)) ||
		(typeof value === "number" && Number.isSafeInteger(value) && value >= 0)
	);
}

function cleanHeaders(headers) {
	const excluded = new Set(HOP_HEADERS);
	for (const token of String(headers.connection ?? "").split(",")) {
		excluded.add(token.trim().toLowerCase());
	}
	return Object.fromEntries(
		Object.entries(headers).filter(
			([name]) => !excluded.has(name.toLowerCase()),
		),
	);
}

export function pathCategory(pathname, method) {
	if (method === "GET" && pathname === "/api/v1/telemetry/config")
		return "frontend_telemetry_config";
	if (pathname === "/health" || pathname === "/healthz") return "health";
	if (pathname === "/execute/sse") return "executor_sse";
	if (pathname === "/execute/stream") return "executor_stream";
	if (pathname === "/execute") return "executor_execute";
	if (/\/execution\/quota$/.test(pathname)) return "execution_quota";
	if (/\/execution\/progress$/.test(pathname)) return "execution_progress";
	if (/\/execution\/events$/.test(pathname)) return "execution_events";
	if (/\/execution\/\.well-known\/jwks\.json$/.test(pathname))
		return "execution_jwks";
	if (/\/execution\/apps\/[^/]+\/(?:widgets|package-widgets)$/.test(pathname))
		return "execution_widgets";
	if (/^\/api\/v1\/channels(?:\/|$)/.test(pathname)) return "execution_channel";
	if (/\/apps\/[^/]+\/.*\/invoke(?:\/async)?$/.test(pathname))
		return "frontend_invoke";
	if (/\/apps\/[^/]+\/pages\/bootstrap$/.test(pathname))
		return "frontend_bootstrap";
	if (/\/apps\/[^/]+\/.*\/prerun$/.test(pathname)) return "frontend_prerun";
	return "other";
}

function reply(response, status, message) {
	if (response.destroyed || response.writableEnded) return;
	if (response.headersSent) {
		response.destroy();
		return;
	}
	response.writeHead(status, {
		"content-type": "application/json",
		"cache-control": "no-store",
	});
	response.end(JSON.stringify({ error: message }));
}

async function readControlBody(request) {
	let size = 0;
	const chunks = [];
	for await (const chunk of request) {
		size += chunk.length;
		if (size > 4096) throw new Error("Control body too large");
		chunks.push(chunk);
	}
	return JSON.parse(Buffer.concat(chunks).toString("utf8"));
}

/** One lease covers the entire upstream response, including accounting after SSE completed. */
export async function startReplicaProxy(config) {
	if (
		!config ||
		!validPort(config.controlPort) ||
		typeof config.controlToken !== "string" ||
		config.controlToken.length < 16
	) {
		throw new Error("Invalid control configuration");
	}
	if (typeof config.logFile !== "string" || !config.logFile)
		throw new Error("A logFile is required");
	const queueLimit = positiveInteger(config.queueLimit, 1024, "queueLimit");
	const queueTimeoutMs = positiveInteger(
		config.queueTimeoutMs,
		30_000,
		"queueTimeoutMs",
	);
	const requestTimeoutMs = positiveInteger(
		config.requestTimeoutMs,
		120_000,
		"requestTimeoutMs",
	);
	const usedTargets = new Set();
	const usedPorts = new Set(config.controlPort ? [config.controlPort] : []);
	const pools = new Map();
	for (const [name, entry] of Object.entries(config.pools ?? {})) {
		if (
			!POOLS.has(name) ||
			!validPort(entry.port) ||
			!Array.isArray(entry.targets) ||
			!entry.targets.length
		) {
			throw new Error("Invalid pool configuration");
		}
		if (entry.port && usedPorts.has(entry.port))
			throw new Error("Listener ports must be distinct");
		if (entry.port) usedPorts.add(entry.port);
		const ids = new Set();
		const targets = entry.targets.map((target) => {
			if (
				!target ||
				typeof target.id !== "string" ||
				!/^[A-Za-z0-9_.-]{1,128}$/.test(target.id) ||
				ids.has(target.id) ||
				!validGeneration(target.generation)
			) {
				throw new Error("Invalid replica identity");
			}
			ids.add(target.id);
			let url;
			try {
				url = new URL(target.url);
			} catch {
				throw new Error("Invalid target URL");
			}
			if (
				url.protocol !== "http:" ||
				url.hostname !== "127.0.0.1" ||
				!url.port ||
				url.username ||
				url.password ||
				url.pathname !== "/" ||
				url.search ||
				url.hash
			) {
				throw new Error("Targets must be HTTP origins on 127.0.0.1");
			}
			if (usedTargets.has(url.origin))
				throw new Error("A target cannot belong to multiple slots");
			usedTargets.add(url.origin);
			return {
				id: target.id,
				url,
				generation: target.generation,
				active: null,
				quarantined: false,
			};
		});
		pools.set(name, {
			name,
			port: entry.port,
			targets,
			queue: [],
			cursor: 0,
			failed: false,
			failures: 0,
			rejected: 0,
			admitted: 0,
			released: 0,
		});
	}
	if (pools.size !== POOLS.size)
		throw new Error("All three replica pools are required");
	for (const origin of usedTargets) {
		if (usedPorts.has(Number(new URL(origin).port)))
			throw new Error("Targets cannot point at proxy listeners");
	}

	const logFd = openSync(config.logFile, "a", 0o600);
	const controlHash = digest(config.controlToken);
	const servers = [];
	let closing = false;
	let closed = false;
	let paused = false;
	const log = (record) => writeSync(logFd, `${JSON.stringify(record)}\n`);
	const snapshot = () => ({
		closing,
		paused,
		pools: Object.fromEntries(
			[...pools].map(([name, pool]) => [
				name,
				{
					active: pool.targets.filter((target) => target.active).length,
					queued: pool.queue.length,
					quarantined: pool.targets.filter((target) => target.quarantined)
						.length,
					failed: pool.failed,
					failures: pool.failures,
					rejected: pool.rejected,
					admitted: pool.admitted,
					released: pool.released,
					targets: pool.targets.map((target) => ({
						id: target.id,
						generation: target.generation,
						state: target.quarantined
							? "quarantined"
							: target.active
								? "active"
								: "idle",
						requestId: target.active?.record.requestId ?? null,
					})),
				},
			]),
		),
	});

	function removeQueued(pool, item, status, reason) {
		const index = pool.queue.indexOf(item);
		if (index < 0) return;
		pool.queue.splice(index, 1);
		clearTimeout(item.queueTimer);
		item.queued = false;
		pool.rejected++;
		if (status) reply(item.response, status, reason);
		item.request.resume();
	}

	function dispatch(pool) {
		if (closing || paused || pool.failed) return;
		while (pool.queue.length) {
			let target;
			for (let offset = 0; offset < pool.targets.length; offset++) {
				const index = (pool.cursor + offset) % pool.targets.length;
				if (!pool.targets[index].active && !pool.targets[index].quarantined) {
					target = pool.targets[index];
					pool.cursor = (index + 1) % pool.targets.length;
					break;
				}
			}
			if (!target) return;
			const item = pool.queue.shift();
			clearTimeout(item.queueTimer);
			item.queued = false;
			if (item.response.destroyed || item.request.aborted) {
				pool.rejected++;
				continue;
			}
			admit(pool, target, item);
		}
	}

	function admit(pool, target, item) {
		const { request, response, record } = item;
		const now = stamp();
		Object.assign(record, {
			replica: target.id,
			generation: target.generation,
			admittedAtUnixMs: now.unixMs,
			admittedAtMonotonicNs: now.monotonicNs,
		});
		target.active = item;
		pool.admitted++;
		log({ event: "admitted", ...record });
		let upstream;
		let upstreamResponse;
		let done = false;
		let responseEnded = false;
		const timer = setTimeout(
			() => fail("upstream_timeout", 504),
			requestTimeoutMs,
		);
		timer.unref();

		function release(success, reason = null) {
			if (done) return;
			done = true;
			clearTimeout(timer);
			const end = stamp();
			Object.assign(record, {
				releasedAtUnixMs: end.unixMs,
				releasedAtMonotonicNs: end.monotonicNs,
				upstreamComplete: success,
				failureReason: reason,
			});
			target.active = null;
			pool.released++;
			log({ event: "released", ...record });
			dispatch(pool);
		}

		function fail(reason, status = 502) {
			if (done) return;
			target.quarantined = true;
			pool.failed = true;
			pool.failures++;
			for (const waiting of [...pool.queue])
				removeQueued(pool, waiting, 503, "Pool requires a new generation");
			reply(response, status, "Upstream did not finish; replica quarantined");
			release(false, reason);
			request.unpipe(upstream);
			request.resume();
			upstreamResponse?.destroy();
			upstream?.destroy();
		}
		item.fail = fail;
		item.disconnect = () => {
			if (done) return;
			record.downstreamDisconnected = true;
			// Keep consuming the worker response so final accounting finishes before reuse.
			upstreamResponse?.unpipe(response);
			upstreamResponse?.resume();
		};
		item.abortUpload = () => fail("downstream_upload_aborted");
		const maybeRelease = () => {
			if (responseEnded && upstream.writableFinished) release(true);
		};
		const headers = cleanHeaders(request.headers);
		headers.host = target.url.host;
		try {
			upstream = httpRequest(
				{
					protocol: "http:",
					hostname: "127.0.0.1",
					port: target.url.port,
					method: request.method,
					path: request.url,
					headers,
					agent: false,
				},
				(incoming) => {
					upstreamResponse = incoming;
					if (done) {
						incoming.destroy();
						return;
					}
					record.status = incoming.statusCode;
					incoming.on("error", () => fail("upstream_response_error"));
					incoming.on("aborted", () => fail("upstream_response_aborted"));
					incoming.on("end", () => {
						if (!incoming.complete) {
							fail("upstream_response_incomplete");
							return;
						}
						responseEnded = true;
						maybeRelease();
					});
					incoming.on("close", () => {
						if (!responseEnded) fail("upstream_response_closed");
					});
					if (record.downstreamDisconnected || response.destroyed) {
						incoming.resume();
					} else {
						response.writeHead(incoming.statusCode, {
							...cleanHeaders(incoming.headers),
							"x-flowlike-benchmark-replica": target.id,
							"x-flowlike-benchmark-generation": String(target.generation),
							"x-flowlike-benchmark-request-id": record.requestId,
						});
						incoming.pipe(response);
					}
				},
			);
			upstream.on("error", () => fail("upstream_request_error"));
			upstream.on("finish", maybeRelease);
			request.pipe(upstream);
		} catch {
			fail("upstream_request_error");
		}
	}

	function accept(pool, request, response) {
		if (!request.url?.startsWith("/") || request.url.startsWith("//")) {
			reply(response, 400, "Origin-form request required");
			request.resume();
			return;
		}
		let pathname;
		try {
			pathname = new URL(request.url, "http://127.0.0.1").pathname;
		} catch {
			reply(response, 400, "Invalid request target");
			request.resume();
			return;
		}
		if (paused) {
			pool.rejected++;
			const now = stamp();
			const category = pathCategory(pathname, request.method);
			if (category !== "health")
				log({
					event: "rejected",
					requestId: randomUUID(),
					pool: pool.name,
					replica: null,
					generation: null,
					method: request.method,
					routeKey: digest(`${request.method}\n${request.url}`).toString("hex"),
					pathCategory: category,
					queuedAtUnixMs: now.unixMs,
					queuedAtMonotonicNs: now.monotonicNs,
					admittedAtUnixMs: null,
					admittedAtMonotonicNs: null,
					releasedAtUnixMs: now.unixMs,
					releasedAtMonotonicNs: now.monotonicNs,
					status: 503,
					upstreamComplete: false,
					downstreamDisconnected: false,
					failureReason: "admission_paused",
				});
			reply(response, 503, "Admission paused");
			request.resume();
			return;
		}
		if (closing || pool.failed) {
			pool.rejected++;
			reply(response, 503, "Pool unavailable");
			request.resume();
			return;
		}
		if (pool.queue.length >= queueLimit) {
			pool.rejected++;
			reply(response, 503, "Admission queue full");
			request.resume();
			return;
		}
		const now = stamp();
		const item = {
			request,
			response,
			queued: true,
			record: {
				requestId: randomUUID(),
				pool: pool.name,
				replica: null,
				generation: null,
				method: request.method,
				routeKey: digest(`${request.method}\n${request.url}`).toString("hex"),
				pathCategory: pathCategory(pathname, request.method),
				queuedAtUnixMs: now.unixMs,
				queuedAtMonotonicNs: now.monotonicNs,
				admittedAtUnixMs: null,
				admittedAtMonotonicNs: null,
				releasedAtUnixMs: null,
				releasedAtMonotonicNs: null,
				status: null,
				upstreamComplete: false,
				downstreamDisconnected: false,
				failureReason: null,
			},
		};
		response.on("close", () => {
			if (response.writableFinished) return;
			if (item.queued) removeQueued(pool, item);
			else item.disconnect?.();
		});
		response.on("error", () => item.disconnect?.());
		request.on("aborted", () => {
			if (item.queued) removeQueued(pool, item);
			else item.abortUpload?.();
		});
		request.on("error", () => {
			if (item.queued) removeQueued(pool, item);
			else item.abortUpload?.();
		});
		item.queueTimer = setTimeout(
			() => removeQueued(pool, item, 504, "Admission queue timed out"),
			queueTimeoutMs,
		);
		item.queueTimer.unref();
		pool.queue.push(item);
		dispatch(pool);
	}

	async function control(request, response) {
		if (request.method === "GET" && request.url === "/health") {
			response.writeHead(closing ? 503 : 200, {
				"content-type": "application/json",
			});
			response.end(JSON.stringify({ ready: !closing }));
			return;
		}
		const authorization = request.headers.authorization ?? "";
		const supplied = authorization.startsWith("Bearer ")
			? authorization.slice(7)
			: "";
		if (!timingSafeEqual(controlHash, digest(supplied))) {
			reply(response, 401, "Unauthorized");
			request.resume();
			return;
		}
		if (request.method === "GET" && request.url === "/status") {
			response.writeHead(200, {
				"content-type": "application/json",
				"cache-control": "no-store",
			});
			response.end(JSON.stringify(snapshot()));
			return;
		}
		if (request.method === "POST" && request.url === "/pause") {
			if (
				closing ||
				[...pools.values()].some(
					(pool) =>
						pool.queue.length || pool.targets.some((target) => target.active),
				)
			) {
				reply(response, 409, "All pools must be idle before pausing");
				request.resume();
				return;
			}
			paused = true;
			response.writeHead(200, { "content-type": "application/json" });
			response.end(JSON.stringify(snapshot()));
			request.resume();
			return;
		}
		if (request.method === "POST" && request.url === "/generation") {
			let body;
			try {
				body = await readControlBody(request);
			} catch {
				reply(response, 400, "Invalid generation body");
				return;
			}
			if (
				!body ||
				Object.keys(body).length !== 1 ||
				!validGeneration(body.generation)
			) {
				reply(response, 400, "Only a generation value is accepted");
				return;
			}
			if (
				closing ||
				!paused ||
				[...pools.values()].some(
					(pool) =>
						pool.queue.length || pool.targets.some((target) => target.active),
				)
			) {
				reply(
					response,
					409,
					"All pools must be paused and idle before changing generation",
				);
				return;
			}
			if (
				[...pools.values()].some((pool) =>
					pool.targets.some((target) => target.generation === body.generation),
				)
			) {
				reply(response, 409, "Generation must change for every replica");
				return;
			}
			for (const pool of pools.values()) {
				pool.failed = false;
				for (const target of pool.targets) {
					target.generation = body.generation;
					target.quarantined = false;
				}
			}
			paused = false;
			response.writeHead(200, { "content-type": "application/json" });
			response.end(JSON.stringify(snapshot()));
			return;
		}
		reply(response, 404, "Unknown control endpoint");
		request.resume();
	}

	async function close({ force = false } = {}) {
		if (closed) return;
		closing = true;
		for (const pool of pools.values()) {
			for (const item of [...pool.queue])
				removeQueued(pool, item, 503, "Proxy closing");
			if (force)
				for (const target of pool.targets)
					target.active?.fail("proxy_shutdown", 503);
		}
		await Promise.all(
			servers.map(
				(server) =>
					new Promise((done) => {
						server.close(done);
						if (force) server.closeAllConnections();
					}),
			),
		);
		if (!closed) {
			closed = true;
			closeSync(logFd);
		}
	}

	async function listen(port, handler) {
		const server = createServer(handler);
		server.requestTimeout = 0;
		server.headersTimeout = 30_000;
		server.on("connect", (_request, socket) => socket.destroy());
		server.on("upgrade", (_request, socket) => socket.destroy());
		servers.push(server);
		await new Promise((ready, reject) => {
			server.once("error", reject);
			server.listen(port, "127.0.0.1", () => {
				server.off("error", reject);
				ready();
			});
		});
		return `http://127.0.0.1:${server.address().port}`;
	}

	try {
		const addresses = {};
		for (const [name, pool] of pools)
			addresses[name] = await listen(pool.port, (request, response) =>
				accept(pool, request, response),
			);
		addresses.control = await listen(
			config.controlPort,
			(request, response) => {
				control(request, response).catch(() =>
					reply(response, 500, "Control request failed"),
				);
			},
		);
		return { addresses, status: snapshot, close };
	} catch (error) {
		await close({ force: true });
		throw error;
	}
}

if (
	process.argv[1] &&
	resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
	if (process.argv.length !== 4 || process.argv[2] !== "--config") {
		console.error("Usage: node replica-proxy.mjs --config PRIVATE_CONFIG.json");
		process.exitCode = 1;
	} else {
		try {
			const proxy = await startReplicaProxy(
				JSON.parse(readFileSync(process.argv[3], "utf8")),
			);
			console.log(
				JSON.stringify({ event: "ready", addresses: proxy.addresses }),
			);
			for (const signal of ["SIGINT", "SIGTERM"])
				process.once(signal, () => {
					proxy.close().catch(() => {
						process.exitCode = 1;
					});
				});
		} catch {
			console.error(
				"Replica proxy could not start; check its private configuration and listener ports.",
			);
			process.exitCode = 1;
		}
	}
}
