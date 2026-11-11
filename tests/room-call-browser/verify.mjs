import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash, generateKeyPairSync, randomUUID, sign } from "node:crypto";
import { mkdir } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import puppeteer from "puppeteer";

// Calls between people in a room, in real Chrome profiles over the real signaling server:
// each person is a separate profile, so presence and audio only arrive over the network.
// Needs bun (signaling) and Chrome; ROOM_CALL_TEST_VERBOSE=1 prints the pages' consoles.
const root = fileURLToPath(new URL("../..", import.meta.url));
const port = Number(process.env.ROOM_CALL_TEST_PORT ?? 4401);
const apiPort = port + 1;
const signalingPort = port + 2;
const origin = `http://127.0.0.1:${port}`;
const api = `http://127.0.0.1:${apiPort}`;
const output = resolve(
	process.env.ROOM_CALL_TEST_OUTPUT ??
		`${tmpdir()}/flow-like-room-call-browser`,
);
await mkdir(output, { recursive: true });

const ROOM = "team-room";
const PEOPLE = ["alice", "bob", "carol"];
const membersOf = (conversation) =>
	conversation === ROOM
		? PEOPLE
		: PEOPLE.filter((person) => conversation === `bot-chat-${person}`);

// The hub signs one room token per member and conversation, as `POST …/realtime` does.
const keys = generateKeyPairSync("ec", { namedCurve: "P-256" });
const base64url = (value) => Buffer.from(value).toString("base64url");
function roomToken(sub, conversation) {
	const now = Math.floor(Date.now() / 1000);
	const signed = `${base64url(
		JSON.stringify({ alg: "ES256", typ: "JWT", kid: "room-call-fixture" }),
	)}.${base64url(
		JSON.stringify({
			sub,
			name: sub,
			conversation_id: conversation,
			scope: "realtime.read",
			typ: "realtime",
			iss: "flow-like",
			aud: "y-webrtc",
			iat: now,
			nbf: now,
			exp: now + 3600,
			jti: randomUUID(),
		}),
	)}`;
	const signature = sign("sha256", Buffer.from(signed), {
		key: keys.privateKey,
		dsaEncoding: "ieee-p1363",
	});
	return `${signed}.${signature.toString("base64url")}`;
}

const inbox = {
	entries: [],
	notifications: [],
	sequence: 0,
	accessRequests: 0,
};
function sendEntry(conversation, sender, entry) {
	const repeat = inbox.entries.find(
		(sent) => sent.message_id === entry.message_id,
	);
	if (repeat) return repeat;
	const id = `entry-${++inbox.sequence}`;
	const sent = {
		id,
		message_id: entry.message_id,
		conversation_id: conversation,
		sender_id: sender,
		kind: entry.kind,
		envelope: entry.envelope,
		cursor: `${String(inbox.sequence).padStart(8, "0")}:${id}`,
		created_at: new Date().toISOString(),
	};
	inbox.entries.push(sent);
	if (entry.kind === "call" && entry.envelope?.body?.op === "started") {
		for (const recipient of membersOf(conversation)) {
			if (recipient === sender) continue;
			inbox.notifications.push({
				id: `pm-${recipient}-${entry.message_id}`,
				user_id: recipient,
				title: `${sender} in Team room`,
				description: "Started a call",
				link: `/bots?conversation=${encodeURIComponent(conversation)}&call=${encodeURIComponent(entry.message_id)}`,
				notification_type: "SYSTEM",
				read: false,
				source_run_id: `chat:${conversation}`,
				source_node_id: entry.message_id,
				created_at: sent.created_at,
			});
		}
	}
	return sent;
}

const hub = createServer(async (request, response) => {
	const url = new URL(request.url, api);
	const reply = (status, body) => {
		response.writeHead(status, {
			"content-type": "application/json",
			"access-control-allow-origin": "*",
			"access-control-allow-headers": "content-type",
			"cache-control": "no-store",
		});
		response.end(JSON.stringify(body));
	};
	if (request.method === "OPTIONS") return reply(204, null);
	let body = "";
	for await (const chunk of request) body += chunk;
	const json = body ? JSON.parse(body) : {};
	if (url.pathname === "/api/v1")
		return reply(200, { signaling: [`ws://127.0.0.1:${signalingPort}`] });
	if (url.pathname === "/fixture/realtime") {
		if (!membersOf(json.conversation).includes(json.sub))
			return reply(404, { error: "not a member" });
		inbox.accessRequests++;
		return reply(200, {
			jwt: roomToken(json.sub, json.conversation),
			encryption_key: createHash("sha256")
				.update(json.conversation)
				.digest("base64"),
			key_id: "2026-11-11.fixture",
			ice_servers: [],
		});
	}
	if (url.pathname === "/fixture/entries" && request.method === "POST")
		return reply(200, sendEntry(json.conversation, json.sender, json.entry));
	if (url.pathname === "/fixture/entries")
		return reply(200, {
			entries: inbox.entries
				.filter(
					(entry) =>
						entry.conversation_id === url.searchParams.get("conversation"),
				)
				.reverse(),
			next_before: null,
		});
	if (url.pathname === "/fixture/notifications")
		return reply(
			200,
			inbox.notifications.filter(
				(notification) => notification.user_id === url.searchParams.get("sub"),
			),
		);
	reply(404, { error: "unknown route" });
});
await new Promise((done) => hub.listen(apiPort, "127.0.0.1", done));

const logs = { vite: "", signaling: "" };
function start(name, command, args, env = {}) {
	const child = spawn(command, args, {
		cwd: root,
		env: { ...process.env, ...env },
		stdio: ["ignore", "pipe", "pipe"],
	});
	child.stdout.on("data", (data) => {
		logs[name] += data;
	});
	child.stderr.on("data", (data) => {
		logs[name] += data;
	});
	return child;
}
const signaling = start(
	"signaling",
	process.env.BUN_PATH ?? "bun",
	["apps/backend/signaling/server.ts"],
	{
		PORT: String(signalingPort),
		NODE_ENV: "test",
		REALTIME_FANOUT_MODE: "local",
		REALTIME_ALLOWED_ORIGINS: origin,
		BACKEND_PUB: Buffer.from(
			keys.publicKey.export({ type: "spki", format: "pem" }),
		).toString("base64"),
	},
);
const vite = start("vite", process.execPath, [
	"node_modules/vite/bin/vite.js",
	"--config",
	"tests/room-call-browser/vite.config.ts",
	"--port",
	String(port),
]);

async function reachable(url, child, name) {
	const deadline = Date.now() + 30_000;
	while (true) {
		if (child.exitCode !== null)
			throw new Error(`${name} exited: ${logs[name]}`);
		try {
			if ((await fetch(url)).ok) return;
		} catch {}
		if (Date.now() > deadline)
			throw new Error(`${name} did not start: ${logs[name]}`);
		await new Promise((done) => setTimeout(done, 100));
	}
}

let browser;
const errors = [];
const pages = new Map();
const snapshot = (page) => page.evaluate(() => window.roomCallTest.snapshot());

/** Each person gets their own browser profile, so tabs cannot reach each other without the network. */
async function open(person, conversation, clockAheadMs = 0) {
	const context = await browser.createBrowserContext();
	const page = await context.newPage();
	await page.setViewport({ width: 900, height: 700 });
	await page.evaluateOnNewDocument((ahead) => {
		const now = Date.now.bind(Date);
		Date.now = () => now() + ahead;
	}, clockAheadMs);
	page.on("pageerror", (error) => errors.push(`${person}: ${error}`));
	page.on("console", (message) => {
		if (process.env.ROOM_CALL_TEST_VERBOSE)
			console.log(`[${person}] ${message.text()}`);
	});
	const search = new URLSearchParams({ me: person, api });
	if (conversation) search.set("conversation", conversation);
	await page.goto(`${origin}/?${search}`);
	await page.waitForFunction(() => Boolean(window.roomCallTest));
	pages.set(person, page);
	return page;
}

async function waitFor(page, check, what, argument, timeout = 30_000) {
	await page.bringToFront();
	try {
		await page.waitForFunction(check, { timeout, polling: 100 }, argument);
	} catch (cause) {
		throw new Error(
			`${what} never happened: ${JSON.stringify(await snapshot(page))}`,
			{ cause },
		);
	}
}
const peersIn = (page, conversation, count) =>
	waitFor(
		page,
		([id, wanted]) =>
			window.roomCallTest.snapshot().rooms[id]?.peers === wanted,
		`${count} peers in ${conversation}`,
		[conversation, count],
	);
const hearing = (page, speaker) =>
	waitFor(
		page,
		(name) =>
			document.querySelector(`li[aria-label="${name} is speaking"]`) !== null,
		`hearing ${speaker}`,
		speaker,
	);
const inCall = (page, count) =>
	waitFor(
		page,
		(wanted) =>
			document.querySelectorAll(
				'[data-slot="conversation-call-bar"] li[data-speaking]',
			).length === wanted,
		`${count} people in the call`,
		count,
	);
async function click(page, selector) {
	await page.bringToFront();
	await page.waitForSelector(selector, { timeout: 30_000 });
	await page.click(selector);
}
async function clickText(page, text) {
	await page.bringToFront();
	await waitFor(
		page,
		(value) =>
			[...document.querySelectorAll("button")].some(
				(button) => button.textContent?.trim() === value,
			),
		`a "${text}" button`,
		text,
	);
	await page.evaluate((value) => {
		[...document.querySelectorAll("button")]
			.find((button) => button.textContent?.trim() === value)
			?.click();
	}, text);
}
const policyCloses = async (page) =>
	(await snapshot(page)).sockets.closes.filter((close) => close.code === 1008);

try {
	await Promise.all([
		reachable(origin, vite, "vite"),
		reachable(
			`http://127.0.0.1:${signalingPort}/health`,
			signaling,
			"signaling",
		),
	]);
	browser = await puppeteer.launch({
		headless: true,
		executablePath:
			process.env.CHROME_PATH ??
			"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
		args: [
			"--use-fake-ui-for-media-stream",
			"--autoplay-policy=no-user-gesture-required",
			"--disable-background-timer-throttling",
			"--disable-renderer-backgrounding",
			"--disable-backgrounding-occluded-windows",
			"--disable-features=WebRtcHideLocalIpsWithMdns",
		],
	});
	const report = {};

	// Everyone has the room open: one starts, the others see it and join.
	const alice = await open("alice", ROOM);
	const bob = await open("bob", ROOM);
	await peersIn(alice, ROOM, 1);
	await peersIn(bob, ROOM, 1);
	await click(alice, 'button[aria-label="Start a call"]');
	await inCall(alice, 1);
	await inCall(bob, 1);
	assert.equal(
		(await snapshot(bob)).calls[ROOM].joined,
		false,
		"seeing a call is not joining it",
	);
	await clickText(bob, "Join");
	await inCall(alice, 2);
	await inCall(bob, 2);
	await hearing(alice, "bob");
	await hearing(bob, "alice");
	await alice.screenshot({ path: resolve(output, "two-in-call.png") });
	report.openRoom = "bob saw alice's call, joined, and both hear each other";

	const carol = await open("carol", ROOM);
	await peersIn(carol, ROOM, 2);
	await inCall(carol, 2);
	await clickText(carol, "Join");
	for (const page of [alice, bob, carol]) await inCall(page, 3);
	await hearing(carol, "alice");
	await hearing(carol, "bob");
	await hearing(alice, "carol");
	await hearing(bob, "carol");
	await carol.screenshot({ path: resolve(output, "three-in-call.png") });
	report.mesh = "three people hear each other";

	for (const page of [carol, bob, alice])
		await click(page, 'button[aria-label="Leave call"]');
	await waitFor(
		alice,
		() => !document.querySelector('[data-slot="conversation-call-bar"]'),
		"the call ending",
	);
	assert.equal(
		inbox.entries.filter((entry) => entry.envelope.body.op === "ended").length,
		1,
		"the last person to leave ends the call line once",
	);

	// A call must ring people who are elsewhere: in their bot's chat, or in no chat at all.
	// Their clocks differ from the caller's, as clocks on two machines do.
	await bob.close();
	await carol.close();
	const bobElsewhere = await open("bob", "bot-chat-bob", 3_000);
	const carolElsewhere = await open("carol", undefined, -3_000);
	await waitFor(
		bobElsewhere,
		() => window.roomCallTest.snapshot().rooms["bot-chat-bob"]?.up === true,
		"bob's bot chat room",
	);
	await click(alice, 'button[aria-label="Start a call"]');
	await inCall(alice, 1);
	for (const [person, page] of [
		["bob", bobElsewhere],
		["carol", carolElsewhere],
	]) {
		await waitFor(
			page,
			() => document.querySelector('[role="alertdialog"]') !== null,
			`${person}'s incoming call`,
			undefined,
			45_000,
		);
		await page.screenshot({
			path: resolve(output, `${person}-incoming-call.png`),
		});
		await clickText(page, "Join");
		await waitFor(
			page,
			(room) => window.roomCallTest.snapshot().calls[room]?.joined === true,
			`${person} joining from the incoming call`,
			ROOM,
		);
	}
	for (const page of [alice, bobElsewhere, carolElsewhere])
		await inCall(page, 3);
	await hearing(alice, "bob");
	await hearing(alice, "carol");
	await hearing(bobElsewhere, "alice");
	await hearing(carolElsewhere, "bob");
	report.ringing =
		"bob (in his bot's chat) and carol (no chat open) were rung and joined";

	for (const [person, page] of [
		["alice", alice],
		["bob", bobElsewhere],
		["carol", carolElsewhere],
	]) {
		assert.deepEqual(
			await policyCloses(page),
			[],
			`${person}'s signaling sockets were closed for policy`,
		);
	}
	report.accessRequests = inbox.accessRequests;
	assert.ok(
		inbox.accessRequests < 20,
		`room access was requested ${inbox.accessRequests} times`,
	);
	assert.deepEqual(errors, []);
	console.log(JSON.stringify({ ...report, screenshots: output }, null, 2));
} catch (error) {
	console.error(logs.signaling.slice(-4000));
	console.error(errors);
	for (const [person, page] of pages) {
		if (page.isClosed()) continue;
		console.error(
			person,
			JSON.stringify(await snapshot(page).catch(() => null)),
		);
		await page
			.screenshot({ path: resolve(output, `failure-${person}.png`) })
			.catch(() => {});
	}
	throw error;
} finally {
	await browser?.close();
	vite.kill();
	signaling.kill();
	hub.close();
	// Chrome's helpers can outlive the browser and hold the output pipes open.
	setTimeout(() => process.exit(process.exitCode ?? 0), 500).unref();
}
