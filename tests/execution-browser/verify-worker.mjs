import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { chromium } from "playwright-core";

const origin = process.env.EXECUTION_QA_ORIGIN || "http://127.0.0.1:4348";
const output =
	process.env.EXECUTION_QA_OUTPUT ||
	join(tmpdir(), "flow-like-execution-verification");
await mkdir(output, { recursive: true });
const browser = await chromium.launch({
	headless: true,
	executablePath:
		process.env.CHROME_PATH ||
		"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
	args: ["--no-sandbox", "--disable-gpu"],
});
const context = await browser.newContext({
	viewport: { width: 1280, height: 900 },
	reducedMotion: "reduce",
});
const errors = [];
const scope = `browser-${Date.now()}`;
const check = (name) => console.log(`PASS ${name}`);
const post = async (path, data) => {
	const response = await context.request.post(
		`${origin}/execution-fixture/${path}`,
		{ data },
	);
	assert.ok(response.ok(), `${path}: ${await response.text()}`);
	return response;
};
const emit = (taskId, event) => post("emit", { taskId, event });
const stats = async () =>
	(await context.request.get(`${origin}/execution-fixture/stats`)).json();
const load = async (targetScope = scope) => {
	const page = await context.newPage();
	page.on("pageerror", (error) => {
		errors.push(error.message);
		console.error("PAGE", error.message);
	});
	page.on("response", (response) => {
		if (response.status() >= 400)
			console.error("HTTP", response.status(), response.url());
	});
	page.on("console", (message) => {
		if (message.type() === "error") console.error("BROWSER", message.text());
	});
	await page.goto(`${origin}/?worker&scope=${targetScope}`, {
		waitUntil: "domcontentloaded",
	});
	await page.waitForFunction(() => window.executionQa?.client, undefined, {
		timeout: 60_000,
	});
	assert.equal(
		await page.evaluate(() => window.executionQa.client.mode),
		"shared-worker",
	);
	return page;
};
const waitTask = async (page, id, status) => {
	try {
		await page.waitForFunction(
			({ id, status }) =>
				window.executionQa.client.getTask(id)?.status === status,
			{ id, status },
		);
	} catch (error) {
		console.error(
			"TASK TIMEOUT",
			id,
			status,
			await page.evaluate(() =>
				window.executionQa.client
					.getTasks()
					.map(
						({
							taskId,
							status,
							error,
							runId,
							cancellationRequested,
							events,
						}) => ({
							taskId,
							status,
							error,
							runId,
							cancellationRequested,
							events: events.length,
						}),
					),
			),
			await stats(),
		);
		throw error;
	}
};
const complete = async (taskId, text) => {
	await emit(taskId, {
		event_type: "chat_out",
		payload: {
			message_id: `message-${taskId}`,
			response: {
				choices: [{ index: 0, message: { role: "assistant", content: text } }],
			},
		},
	});
	await emit(taskId, {
		event_type: "completed",
		payload: { status: "completed" },
	});
};

try {
	await post("reset", {});
	const owner = await load();
	const observer = await load();
	await owner.evaluate(() =>
		window.executionQa.start("shared-run", {
			title: "Research from another tab",
		}),
	);
	await waitTask(owner, "shared-run", "running");
	await waitTask(observer, "shared-run", "running");
	assert.deepEqual((await stats()).starts, ["shared-run"]);
	check("two tabs share one real worker execution and one SSE request");
	await owner.close();
	await emit("shared-run", {
		event_type: "chat_stream_partial",
		payload: {
			message_id: "message-shared-run",
			chunk: {
				choices: [
					{
						index: 0,
						delta: {
							role: "assistant",
							content: "Still working after the initiating tab closed.",
						},
					},
				],
			},
		},
	});
	await observer.waitForFunction(() =>
		window.executionQa.client
			.getTask("shared-run")
			?.events.some((event) => event.event_type === "chat_stream_partial"),
	);
	assert.equal(
		(await stats()).runs.find((run) => run.taskId === "shared-run").closed,
		false,
	);
	check("closing the initiating tab leaves the stream alive in the other tab");

	const channel = {
		channel_id: "fixture-channel",
		request_id: "fixture-question",
		expires_at: Math.floor(Date.now() / 1000) + 900,
		transport: {
			type: "http",
			push_url: `${origin}/execution-fixture/push`,
			token: "fixture-token",
		},
	};
	await emit("shared-run", {
		event_type: "interaction_request",
		payload: {
			id: "fixture-question",
			name: "Which report should I prepare?",
			description:
				"The source data is ready. Choose the detail level for the final report.",
			status: "pending",
			ttl_seconds: 900,
			expires_at: channel.expires_at,
			channel,
			interaction_type: {
				type: "single_choice",
				options: [
					{ id: "summary", label: "Executive summary" },
					{ id: "full", label: "Detailed report" },
				],
			},
		},
	});
	await waitTask(observer, "shared-run", "waiting");
	const peer = await load();
	await waitTask(peer, "shared-run", "waiting");
	await observer.getByRole("button", { name: /^Activity,/ }).click();
	await observer.getByRole("button", { name: "Respond", exact: true }).click();
	await observer
		.getByRole("radio", { name: "Executive summary", exact: true })
		.waitFor();
	await observer.screenshot({
		path: join(output, "06-worker-feedback-other-tab.png"),
		animations: "disabled",
	});
	await Promise.all(
		[observer, peer].map((page) =>
			page.evaluate(() =>
				window.executionQa.engine.replyToTask(
					"shared-run",
					"fixture-question",
					{ selected_id: "summary" },
				),
			),
		),
	);
	await waitTask(observer, "shared-run", "running");
	await waitTask(peer, "shared-run", "running");
	assert.equal((await stats()).replies.length, 1);
	assert.equal(await observer.getByRole("radio").count(), 0);
	check(
		"feedback reaches another tab and racing answers send exactly one HTTP reply",
	);
	await complete(
		"shared-run",
		"The research is finished, including the answer supplied from another tab.",
	);
	await waitTask(peer, "shared-run", "completed");
	await peer.waitForFunction(
		async () =>
			(await window.executionQa.history("shared-run"))[0]?.inner.content ===
			"The research is finished, including the answer supplied from another tab.",
	);
	assert.equal(
		await peer.evaluate(
			async () => (await window.executionQa.history("shared-run")).length,
		),
		1,
	);
	check(
		"worker completion writes one final chat reply visible to the surviving tabs",
	);

	await observer.reload({ waitUntil: "networkidle" });
	await observer.waitForFunction(() => window.executionQa?.client);
	await waitTask(observer, "shared-run", "completed");
	assert.equal(
		await observer.evaluate(
			async () => (await window.executionQa.history("shared-run")).length,
		),
		1,
	);
	const isolated = await load(`${scope}-other-profile`);
	assert.equal(
		await isolated.evaluate(() => window.executionQa.client.getTasks().length),
		0,
	);
	await isolated.close();
	check(
		"reload replays existing tasks without duplicating messages and scopes stay isolated",
	);

	await observer.evaluate(() =>
		window.executionQa.start("cancel-run", { title: "Cancelable export" }),
	);
	await waitTask(peer, "cancel-run", "running");
	await peer.getByRole("button", { name: /^Activity,/ }).click();
	await peer
		.getByRole("button", { name: "Stop Cancelable export", exact: true })
		.click();
	await waitTask(observer, "cancel-run", "cancelled");
	assert.equal(
		(await stats()).cancellations.filter((id) => id === "run-cancel-run")
			.length,
		1,
	);
	await peer
		.getByRole("button", { name: "Dismiss Cancelable export", exact: true })
		.click();
	await observer.waitForFunction(
		() => !window.executionQa.client.getTask("cancel-run"),
	);
	check(
		"another tab can cancel the server execution and dismissal synchronizes",
	);

	await observer.evaluate(() =>
		window.executionQa.start("early-cancel", {
			title: "Waiting for run ID",
			delayRunId: true,
		}),
	);
	await waitTask(peer, "early-cancel", "running");
	await peer.evaluate(() =>
		window.executionQa.engine.cancelTask("early-cancel"),
	);
	await emit("early-cancel", {
		event_type: "run_initiated",
		payload: { run_id: "run-early-cancel" },
	});
	await waitTask(observer, "early-cancel", "cancelled");
	assert.equal(
		(await stats()).cancellations.filter((id) => id === "run-early-cancel")
			.length,
		1,
	);
	check(
		"cancellation before the run ID arrives still reaches the backend once",
	);

	const fallbackContext = await browser.newContext();
	await fallbackContext.addInitScript(() => {
		Object.defineProperty(window, "SharedWorker", {
			value: undefined,
			configurable: true,
		});
	});
	const fallback = await fallbackContext.newPage();
	fallback.on("pageerror", (error) => errors.push(error.message));
	await fallback.goto(`${origin}/?worker&scope=${scope}-fallback`, {
		waitUntil: "networkidle",
	});
	await fallback.waitForFunction(() => window.executionQa?.client);
	assert.equal(
		await fallback.evaluate(() => window.executionQa.client.mode),
		"in-tab",
	);
	await fallback.evaluate(() => window.executionQa.start("fallback-run"));
	await waitTask(fallback, "fallback-run", "running");
	await complete(
		"fallback-run",
		"The fallback runtime completed this response.",
	);
	await waitTask(fallback, "fallback-run", "completed");
	assert.equal(
		await fallback.evaluate(
			async () =>
				(await window.executionQa.history("fallback-run"))[0]?.inner.content,
		),
		"The fallback runtime completed this response.",
	);
	await fallbackContext.close();
	check(
		"browsers without SharedWorker execute and persist through the in-tab fallback",
	);
	assert.deepEqual(errors, [], "Browser errors");
} finally {
	await browser.close();
}
