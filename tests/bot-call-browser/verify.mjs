import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import puppeteer from "puppeteer";

const root = fileURLToPath(new URL("../..", import.meta.url));
const port = Number(process.env.BOT_CALL_TEST_PORT ?? 4391);
const origin = `http://127.0.0.1:${port}`;
const output = resolve(
	process.env.BOT_CALL_TEST_OUTPUT ?? `${tmpdir()}/flow-like-bot-call-browser`,
);
await mkdir(output, { recursive: true });
const server = spawn(
	process.execPath,
	[
		"node_modules/vite/bin/vite.js",
		"--config",
		"tests/bot-call-browser/vite.config.ts",
		"--port",
		String(port),
	],
	{
		cwd: root,
		stdio: ["ignore", "pipe", "pipe"],
	},
);
let log = "";
server.stdout.on("data", (data) => {
	log += data;
});
server.stderr.on("data", (data) => {
	log += data;
});
let browser;
const results = [];
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
try {
	const deadline = Date.now() + 20_000;
	while (true) {
		if (server.exitCode !== null) throw new Error(log);
		try {
			if ((await fetch(origin)).ok) break;
		} catch {}
		if (Date.now() > deadline) throw new Error(`Preview did not start: ${log}`);
		await pause(100);
	}
	browser = await puppeteer.launch({
		headless: true,
		executablePath:
			process.env.CHROME_PATH ??
			"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
	});
	const page = await browser.newPage();
	await page.setViewport({ width: 1280, height: 800 });
	page.setDefaultTimeout(20_000);
	const errors = [];
	page.on("pageerror", (error) => errors.push(String(error)));
	const snapshot = () => page.evaluate(() => window.botCallTest.snapshot());
	async function start(kind, voice) {
		errors.length = 0;
		await page.goto(`${origin}/?kind=${kind}&voice=${voice}`);
		await page.waitForFunction(() => Boolean(window.botCallTest));
		await page.waitForSelector(
			'button[aria-label="Start a call"]:not([disabled])',
		);
		await page.click('button[aria-label="Start a call"]');
		await page.waitForSelector('button[aria-label="Leave call"]');
		await page.waitForSelector(
			'[role="switch"][aria-label="Let bots listen"]:not([disabled])',
		);
		if (kind === "group") {
			assert.equal(
				await page.$eval('[role="switch"]', (node) =>
					node.getAttribute("aria-checked"),
				),
				"false",
				"group microphone sharing requires consent",
			);
			await page.evaluate(() =>
				window.botCallTest.utterance("Scout, before listening consent"),
			);
			assert.equal((await snapshot()).transcriptions.length, 0);
			await page.click('[role="switch"][aria-label="Let bots listen"]');
		}
		await page.waitForSelector('[role="switch"][aria-checked="true"]');
		// Allow the production AudioWorklet module to attach before the fixture device speaks.
		await pause(250);
	}
	async function say(text) {
		await page.evaluate((value) => window.botCallTest.utterance(value), text);
		await page.waitForFunction(
			(value) =>
				[...document.querySelectorAll('[data-slot="spoken-lines"]')].some(
					(node) => node.textContent.includes(value),
				),
			{},
			text,
		);
	}
	async function reply(text) {
		await page.waitForFunction(
			(value) =>
				[...document.querySelectorAll('[data-testid="bot-reply"]')].some(
					(node) => node.textContent === value,
				),
			{},
			text,
		);
	}
	async function leave() {
		await page.click('button[aria-label="Leave call"]');
		await page.waitForFunction(
			() => !document.querySelector('[data-slot="conversation-call-bar"]'),
		);
		const state = await snapshot();
		assert.ok(
			state.microphones.flat().every((track) => track === "ended"),
			"leaving stops microphone capture",
		);
		assert.ok(
			state.contexts.every((state) => state === "closed"),
			"leaving closes capture/playback contexts",
		);
		assert.deepEqual(errors, []);
		await page.evaluate(() => window.botCallTest.dispose());
		return state;
	}

	await start("group", "default");
	await say("Scout, summarize this conversation");
	await reply("Scout heard: Scout, summarize this conversation");
	await page.waitForFunction(
		() => window.botCallTest.snapshot().playbacks === 1,
	);
	await page.evaluate(() =>
		window.botCallTest.utterance("Scout, echo from the speakers"),
	);
	let state = await snapshot();
	assert.equal(
		state.transcriptions.length,
		1,
		"bot audio must not feed back into transcription",
	);
	assert.ok(
		state.transcriptions[0].wav && state.transcriptions[0].bytes > 20_000,
		"real microphone frames are encoded as WAV",
	);
	assert.equal(state.transcriptions[0].bit_id, "fixture-stt");
	assert.deepEqual(
		state.sent.find((entry) => entry.kind === "transcript").envelope.body
			.mentions,
		["bot_scout"],
	);
	assert.equal(
		state.speech[0].bit_id,
		"fixture-voice",
		"an unpinned bot uses the profile voice",
	);
	assert.equal(state.speech[0].voice, "alloy");
	assert.equal(state.speech[0].format, "wav");
	assert.equal(
		state.localAudioConnections,
		1,
		"bot speech reaches the local speakers",
	);
	assert.ok(
		state.peakOutput > 0.23,
		"bot speech also reaches the outgoing call audio",
	);
	assert.equal(
		await page.$$eval('[data-testid="bot-reply"]', (nodes) => nodes.length),
		1,
		"only the named group bot responds",
	);
	await say("We can continue discussing the plan");
	await pause(1500);
	state = await snapshot();
	assert.deepEqual(
		state.sent.filter((entry) => entry.kind === "transcript").at(-1).envelope
			.body.mentions,
		[],
	);
	assert.equal(
		state.speech.length,
		1,
		"unaddressed group speech does not wake bots",
	);
	await page.click('button[aria-label="Mute microphone"]');
	await page.evaluate(() =>
		window.botCallTest.utterance("Scout, muted speech"),
	);
	assert.equal(
		(await snapshot()).transcriptions.length,
		2,
		"muting pauses transcription",
	);
	await page.click('button[aria-label="Unmute microphone"]');
	await say("Scribe, record the decision");
	await reply("Scribe heard: Scribe, record the decision");
	await page.waitForFunction(
		() => window.botCallTest.snapshot().playbacks === 2,
	);
	await pause(1250);
	await page.screenshot({
		path: resolve(output, "group-voice.png"),
		fullPage: true,
	});
	results.push({
		scenario: "group voice, selective wake, mute and resume",
		...(await leave()),
		screenshot: resolve(output, "group-voice.png"),
	});

	await start("direct", "default");
	await say("What should I do next?");
	await reply("Scout heard: What should I do next?");
	await page.waitForFunction(
		() => window.botCallTest.snapshot().playbacks === 1,
	);
	state = await snapshot();
	assert.deepEqual(
		state.sent.find((entry) => entry.kind === "transcript").envelope.body
			.mentions,
		["bot_scout"],
		"direct calls address their sole bot without its name",
	);
	await page.screenshot({
		path: resolve(output, "direct-bot-call.png"),
		fullPage: true,
	});
	results.push({
		scenario: "direct bot call with automatic listening and default voice",
		...(await leave()),
	});

	await start("direct", "none");
	await say("Answer in text please");
	await reply("Scout heard: Answer in text please");
	await pause(500);
	state = await snapshot();
	assert.equal(state.speech.length, 0);
	assert.equal(state.playbacks, 0);
	results.push({
		scenario: "text reply without a voice model",
		...(await leave()),
	});

	await start("direct", "pinned");
	await say("Use your pinned voice");
	await reply("Scout heard: Use your pinned voice");
	await page.waitForFunction(
		() => window.botCallTest.snapshot().playbacks === 1,
	);
	assert.equal((await snapshot()).speech[0].bit_id, "fixture-voice");
	results.push({ scenario: "pinned bot voice", ...(await leave()) });

	await start("direct", "failure");
	await say("Keep the answer if the voice fails");
	await reply("Scout heard: Keep the answer if the voice fails");
	await page.waitForFunction(() =>
		document.body.textContent.includes("The bot's voice could not play"),
	);
	state = await snapshot();
	assert.equal(state.speech.length, 1);
	assert.equal(state.playbacks, 0);
	results.push({
		scenario: "voice failure preserves text",
		...(await leave()),
	});
	console.log(
		JSON.stringify(
			results.map(
				({
					scenario,
					transcriptions,
					speech,
					playbacks,
					peakOutput,
					screenshot,
				}) => ({
					scenario,
					transcriptions: transcriptions.length,
					voiceReplies: speech.length,
					playbacks,
					peakOutput,
					screenshot,
				}),
			),
			null,
			2,
		),
	);
} catch (error) {
	console.error(log);
	if (browser) {
		for (const page of await browser.pages()) {
			console.error(
				await page
					.evaluate(() => document.body.innerText)
					.catch(() => "page unavailable"),
			);
			console.error(
				await page
					.evaluate(() => window.botCallTest?.snapshot())
					.catch(() => null),
			);
		}
	}
	throw error;
} finally {
	await browser?.close();
	server.kill();
}
