import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { jwtAuthenticator, nkeys, wsconnect } from "@nats-io/nats-core";
import { openRealtimeProvider } from "../../packages/ui/lib/realtime/providers";
import { RealtimeRuntime } from "../../packages/ui/lib/realtime/runtime";
import type {
	RealtimePort,
	RealtimeTransport,
} from "../../packages/ui/lib/realtime/types";

const binary = process.env.NATS_SERVER_BIN;
assert.ok(binary, "Set NATS_SERVER_BIN to an existing nats-server executable.");
const root = resolve(import.meta.dir, "../..");
const bootstrap = spawnSync(
	"python3",
	[
		"-c",
		"import importlib.util,json; spec=importlib.util.spec_from_file_location('bootstrap','apps/backend/scripts/nats_bootstrap.py'); module=importlib.util.module_from_spec(spec); spec.loader.exec_module(module); print(json.dumps(module.generate()))",
	],
	{ cwd: root, encoding: "utf8" },
);
assert.equal(
	bootstrap.status,
	0,
	"NATS bootstrap failed; OpenSSL with Ed25519 is required.",
);
const trust = JSON.parse(bootstrap.stdout);

async function freePort(): Promise<number> {
	const socket = createServer();
	await new Promise<void>((resolve, reject) => {
		socket.once("error", reject);
		socket.listen(0, "127.0.0.1", resolve);
	});
	const address = socket.address();
	assert.ok(address && typeof address !== "string");
	await new Promise<void>((resolve) => socket.close(() => resolve()));
	return address.port;
}

function base32(data: Uint8Array): string {
	const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
	let bits = 0;
	let value = 0;
	let result = "";
	for (const byte of data) {
		value = (value << 8) | byte;
		bits += 8;
		while (bits >= 5) {
			result += alphabet[(value >>> (bits - 5)) & 31];
			bits -= 5;
		}
	}
	if (bits) result += alphabet[(value << (5 - bits)) & 31];
	return result;
}

const account = nkeys.fromSeed(new TextEncoder().encode(trust.account_seed));
const scope = "a".repeat(64);
const subject = `realtime.${scope}.wake`;
const foreign = `realtime.${"b".repeat(64)}.wake`;
const expiresAt = Math.floor(Date.now() / 1000) + 120;
function token(publisher: boolean): string {
	const claims = {
		iss: account.getPublicKey(),
		sub: nkeys.createUser().getPublicKey(),
		iat: Math.floor(Date.now() / 1000),
		exp: expiresAt,
		nats: {
			pub: publisher ? { allow: ["realtime.*.wake"] } : { deny: [">"] },
			sub: publisher ? { deny: [">"] } : { allow: [subject] },
			subs: 1,
			data: -1,
			payload: 4096,
			bearer_token: true,
			type: "user",
			version: 2,
		},
	};
	const jti = base32(
		createHash("sha256").update(JSON.stringify(claims)).digest(),
	);
	const encoded = `${Buffer.from(JSON.stringify({ typ: "JWT", alg: "ed25519-nkey" })).toString("base64url")}.${Buffer.from(JSON.stringify({ ...claims, jti })).toString("base64url")}`;
	return `${encoded}.${Buffer.from(account.sign(new TextEncoder().encode(encoded))).toString("base64url")}`;
}

const directory = await mkdtemp(join(tmpdir(), "flow-like-realtime-ws-"));
const [tcpPort, wsPort, monitorPort] = await Promise.all([
	freePort(),
	freePort(),
	freePort(),
]);
const config = join(directory, "nats.conf");
await writeFile(
	config,
	JSON.stringify({
		host: "127.0.0.1",
		port: tcpPort,
		http: `127.0.0.1:${monitorPort}`,
		max_payload: 1048576,
		operator: trust.operator_jwt,
		resolver: "MEMORY",
		resolver_preload: { [trust.account_public_key]: trust.account_jwt },
		websocket: {
			host: "127.0.0.1",
			port: wsPort,
			no_tls: true,
			compression: false,
		},
	}),
	{ mode: 0o600 },
);
const broker = spawn(binary, ["-c", config], {
	stdio: ["ignore", "ignore", "ignore"],
});
const descriptor: Extract<RealtimeTransport, { type: "nats" }> = {
	type: "nats",
	url: `ws://127.0.0.1:${wsPort}`,
	subject,
	token: token(false),
};
let runtime: RealtimeRuntime | undefined;
let publisher: Awaited<ReturnType<typeof wsconnect>> | undefined;

async function until(
	check: () => boolean | Promise<boolean>,
	timeout = 5_000,
): Promise<void> {
	const deadline = Date.now() + timeout;
	while (!(await check())) {
		assert.ok(
			Date.now() < deadline,
			"Timed out waiting for the realtime broker test.",
		);
		await new Promise((resolve) => setTimeout(resolve, 20));
	}
}

async function denied(
	action: "publish" | "subscribe",
	target: string,
): Promise<void> {
	const client = await wsconnect({
		servers: descriptor.url,
		authenticator: jwtAuthenticator(descriptor.token),
		reconnect: false,
		timeout: 2_000,
	});
	let rejected = false;
	const statuses = (async () => {
		for await (const status of client.status()) {
			if (status.type === "error" && /permission/i.test(status.error.message))
				rejected = true;
		}
	})();
	try {
		if (action === "publish")
			client.publish(target, new TextEncoder().encode("forbidden"));
		else client.subscribe(target, { callback() {} });
		await until(() => rejected);
	} finally {
		await client.close();
		await statuses;
	}
}

try {
	await until(async () => {
		try {
			return (await fetch(`http://127.0.0.1:${monitorPort}/healthz`)).ok;
		} catch {
			return false;
		}
	});
	await denied("subscribe", foreign);
	await denied("publish", subject);
	await denied("publish", foreign);
	publisher = await wsconnect({
		servers: descriptor.url,
		authenticator: jwtAuthenticator(token(true)),
		reconnect: false,
		timeout: 2_000,
	});
	let connected = false;
	let revision = "n1";
	const revisions: string[] = [];
	const pollTimeouts: number[] = [];
	runtime = new RealtimeRuntime({
		openProvider: async (...args) => {
			const connection = await openRealtimeProvider(...args);
			connected = true;
			return connection;
		},
		fetch: async (url, init) => {
			if (String(url).endsWith("/connect"))
				return Response.json({
					protocol_version: 1,
					expires_at: expiresAt,
					transport: descriptor,
				});
			const request = JSON.parse(String(init?.body));
			pollTimeouts.push(request.timeout_ms);
			return Response.json({
				protocol_version: 1,
				...(request.notifications?.revision !== revision
					? {
							notifications: {
								revision,
								overview: {
									invites_count: 0,
									notifications_count: 1,
									unread_count: 1,
								},
							},
						}
					: {}),
				runs: [],
				errors: [],
			});
		},
	});
	const channel: RealtimePort = {
		postMessage(message) {
			if (message.type !== "snapshot") return;
			if (message.snapshot.notifications)
				revisions.push(message.snapshot.notifications.revision);
			queueMicrotask(() =>
				runtime?.receive(channel, {
					type: "accepted",
					id: "tab",
					delivery_id: message.delivery_id,
				}),
			);
		},
	};
	runtime.receive(channel, {
		type: "connect",
		id: "tab",
		session: {
			scope: "account",
			url: "https://hub.test/api/v1/realtime/poll",
			headers: { Authorization: "Bearer fixture-account" },
		},
		activity: { online: true, visible: true },
	});
	runtime.receive(channel, {
		type: "subscribe",
		id: "tab",
		subscription: { notifications: true, runs: [] },
	});
	await until(
		() => connected && revisions.includes("n1") && pollTimeouts.at(-1) === 0,
	);
	await new Promise((resolve) => setTimeout(resolve, 100));
	revision = "n2";
	publisher.publish(
		subject,
		new TextEncoder().encode(
			JSON.stringify({
				protocol_version: 1,
				kind: "invalidate",
				nonce: "own-account",
			}),
		),
	);
	await publisher.flush();
	await until(() => revisions.includes("n2"));
	await new Promise((resolve) => setTimeout(resolve, 100));
	const count = pollTimeouts.length;
	publisher.publish(
		foreign,
		new TextEncoder().encode(
			JSON.stringify({
				protocol_version: 1,
				kind: "invalidate",
				nonce: "foreign-account",
			}),
		),
	);
	await publisher.flush();
	await new Promise((resolve) => setTimeout(resolve, 250));
	assert.equal(
		pollTimeouts.length,
		count,
		"Foreign account wakeups must not trigger replay.",
	);
	broker.kill();
	await until(() => pollTimeouts.at(-1) === 20_000);
	console.log(
		"NATS WebSocket realtime integration passed: receive-only permissions, account isolation, wakeup replay, broker disconnect HTTP fallback.",
	);
} finally {
	runtime?.dispose();
	await publisher?.close();
	if (broker.exitCode === null) broker.kill();
	account.clear();
	await rm(directory, { recursive: true, force: true });
}
