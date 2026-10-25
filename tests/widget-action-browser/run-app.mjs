import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import os from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import puppeteer from "puppeteer";
import { installAppMeasurement, requestCategory } from "./app-measurement.mjs";
import {
	assertColdActionConfiguration,
	waitForColdActionController,
} from "./cold-action-control.mjs";
import { observeExecution } from "./execution-evidence.mjs";
import { requestRouteKey, summarize } from "./measurement.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const args = {};
for (let index = 2; index < process.argv.length; index++) {
	const [key, inline] = process.argv[index].replace(/^--/, "").split("=", 2);
	if (key === "help") {
		console.log(
			"node tests/widget-action-browser/run-app.mjs --config FILE [--app-url http://127.0.0.1:3001/use/...] [--cdp-url http://127.0.0.1:9222] [--output PATH] [--allow-remote] [--cold-action-control --cold-action-timeout-ms 180000]\nConfig: {appUrl, readySelector?, actions:[{selector,text?,historySelector,expectedText,expectedRows?,rowSelector?}], reset?:{selector,text?},samples:20,warmup:2,navigations:2,concurrency:1,timeoutMs:30000,headed:false,requireActionTrace:true,requireTerminalSuccess:true,storageState?}. samples is the total measured count per navigation across workers; warmup is per worker. Disabling requireTerminalSuccess permits unverified probe results. Use at least two read-only person actions with distinct history markers. Storage-state and selectors are never copied into reports. Cold-action control is a fixed stdin/stdout handshake for the local stack runner, with one action and no warmup.",
		);
		process.exit(0);
	}
	if (key === "allow-remote") {
		args.allowRemote = true;
		continue;
	}
	if (key === "cold-action-control") {
		args.coldActionControl = true;
		continue;
	}
	assert.ok(
		["config", "app-url", "cdp-url", "output", "cold-action-timeout-ms"].includes(key),
		`Unknown option --${key}`,
	);
	args[key] = inline ?? process.argv[++index];
	assert.ok(args[key], `Missing value for --${key}`);
}
assert.ok(args.config, "--config is required");
const configPath = resolve(args.config);
const config = {
	samples: 20,
	warmup: 2,
	navigations: 2,
	concurrency: 1,
	timeoutMs: 30_000,
	headed: false,
	requireActionTrace: true,
	requireTerminalSuccess: true,
	...JSON.parse(await readFile(configPath, "utf8")),
};
config.appUrl = args["app-url"] ?? config.appUrl;
config.cdpUrl = args["cdp-url"] ?? config.cdpUrl;
const app = new URL(config.appUrl);
const local = (url) =>
	["localhost", "127.0.0.1", "[::1]"].includes(url.hostname);
assert.ok(
	["http:", "https:"].includes(app.protocol) && !app.username && !app.password,
	"Use an HTTP app URL without embedded credentials",
);
assert.ok(
	local(app) || args.allowRemote,
	"Remote references require explicit --allow-remote; use a local app for optimization",
);
assert.ok(
	Array.isArray(config.actions) && config.actions.length >= 2,
	"Configure at least two distinct person actions",
);
for (const [index, action] of config.actions.entries()) {
	for (const key of ["selector", "historySelector", "expectedText"])
		assert.ok(
			typeof action[key] === "string" && action[key].length,
			`Action ${index} needs ${key}`,
		);
	assert.ok(
		action.expectedRows === undefined ||
			(Number.isInteger(action.expectedRows) && action.expectedRows >= 0),
		`Action ${index} expectedRows must be a nonnegative integer`,
	);
}
if (config.reset)
	assert.ok(
		typeof config.reset.selector === "string" && config.reset.selector.length,
		"reset needs a selector",
	);
assert.ok(
	new Set(config.actions.map((action) => action.expectedText)).size ===
		config.actions.length,
	"Each action needs a distinct history marker",
);
for (const key of ["samples", "navigations", "timeoutMs", "concurrency"])
	assert.ok(
		Number.isInteger(config[key]) && config[key] > 0,
		`${key} must be a positive integer`,
	);
assert.ok(
	Number.isInteger(config.warmup) && config.warmup >= 0,
	"warmup must be a nonnegative integer",
);
if (config.cdpUrl)
	assert.ok(
		local(new URL(config.cdpUrl)),
		"The browser debugging endpoint must be local",
	);
assert.ok(
	config.concurrency <= config.samples,
	"concurrency must not exceed the measured action count",
);
assert.ok(
	config.concurrency === 1 || (!config.cdpUrl && !config.headed),
	"Concurrent frame measurements require isolated headless browsers",
);
assert.ok(
	args["cold-action-timeout-ms"] === undefined || args.coldActionControl,
	"--cold-action-timeout-ms requires --cold-action-control",
);
const coldActionTimeoutMs = Number(args["cold-action-timeout-ms"] ?? 180_000);
if (args.coldActionControl) {
	assertColdActionConfiguration(config, app, process.stdin);
	assert.ok(
		Number.isInteger(coldActionTimeoutMs) &&
			coldActionTimeoutMs > 0 &&
			coldActionTimeoutMs <= 2_147_483_647,
		"Cold-action timeout must be a positive integer within the timer range",
	);
}
const output = resolve(
	args.output ?? config.output ?? join(root, "tmp/widget-action-benchmark/app"),
);
await mkdir(output, { recursive: true });
const report = {
	schemaVersion: 1,
	mode: "actual-app",
	status: "running",
	startedAt: new Date().toISOString(),
	app: { origin: app.origin, path: app.pathname },
	configuration: {
		samples: config.samples,
		warmup: config.warmup,
		navigations: config.navigations,
		actions: config.actions.length,
		resetBetweenActions: Boolean(config.reset),
		timeoutMs: config.timeoutMs,
		requireActionTrace: config.requireActionTrace,
		requireTerminalSuccess: config.requireTerminalSuccess,
		authenticatedBrowserAttached: Boolean(config.cdpUrl),
		storageStateSupplied: Boolean(config.storageState),
		concurrency: config.concurrency,
		loadMode: config.concurrency === 1 ? "isolated" : "synchronized-bursts",
		totalMeasuredActions: config.samples * config.navigations,
		totalWarmupActions: config.warmup * config.concurrency * config.navigations,
		...(args.coldActionControl
			? { coldActionControl: true, coldActionTimeoutMs }
			: {}),
	},
	machine: {
		platform: os.platform(),
		arch: os.arch(),
		cpu: os.cpus()[0]?.model,
		logicalCpus: os.cpus().length,
		memoryBytes: os.totalmem(),
	},
	browser: null,
	metricNotes: [
		"This driver uses the application's existing UI, bootstrap, prerun, dispatch, and streaming paths without response interception.",
		"Initial and repeat navigation labels describe documents. They do not claim that API, database, HTTP, IndexedDB, or executor caches were cold.",
		"Cards DOM readiness waits for every configured action to be visible, enabled, and free of pending Page-action guards. Before any action, bootstrap also waits for the page onLoad handler trace and independently verifies successful invocation completion; these checks do not move the cards timing boundary.",
		"Action readiness requires a distinct person marker and, when configured, an exact visible row count. DOM content and selectors are not exported.",
		"Two requestAnimationFrame callbacks after matching DOM indicate a frame opportunity, not physical paint. Click clocks start at event capture.",
		"Click-to-settled ends when the action handler trace is observed, including handlers that failed. This timing does not verify workflow terminal success. Handler-to-dispatch/patch/settled fields use the application's separate handler clock.",
		"handlerToPatchMs marks the first A2UI update received, which may precede history rows. Only the history DOM/frame fields verify the expected side panel content.",
		"Terminal success is independently checked in invoke responses observed through CDP. Only status, log level, error flags, and run IDs are retained. Missing evidence fails unless requireTerminalSuccess is explicitly false.",
		"Network records retain categories, statuses, sizes, and timing only. Tokens, headers, request bodies, query strings, and storage state are omitted.",
		"routeKey is SHA256(JSON.stringify([method, URL.pathname])). It groups exact method/path pairs without retaining paths; query variants can share a key, and distinct run IDs in paths remain distinct. Repeated keys alone do not establish redundant requests.",
		"Each concurrent worker has an isolated headless browser and completes normal application bootstrap and warmup before measured bursts. samples is a fixed total per navigation, distributed across workers.",
		"Mutation observation, 10 ms action polling, and browser automation add measurement overhead. Scheduled-to-click skew is recorded separately from click-to-history latency.",
		"When configured, a read-only reset control returns to the person cards between actions. Reset DOM/frame timing and successful terminal execution are recorded separately; every reset finishes before the next action burst is scheduled.",
		"CDP also copies invoke response bytes to inspect terminal metadata. The application consumes its original stream unchanged; terminal validation happens after the browser latency clocks have stopped.",
		...(args.coldActionControl
			? ["After successful bootstrap, cold-action control waits for the local runner to confirm its API/runtime restart before clicking in the same document. The runner records process identity and readiness evidence separately. Other infrastructure, browser state and storage artifacts remain warm."]
			: []),
	],
	navigations: [],
	samples: [],
	resets: [],
	network: [],
	summaries: [],
	browserErrorCount: 0,
};
const save = () => {
	const measured = report.samples.filter((sample) => !sample.warmup);
	report.overall = {
		...summarize(measured),
		planned: config.samples * config.navigations,
	};
	report.warmups = summarize(report.samples.filter((sample) => sample.warmup));
	report.preparation = summarize(report.resets, [
		"clickToCardsDomMs",
		"clickToCardsFrameOpportunityMs",
		"clickToSettledMs",
		"handlerToSettledMs",
	]);
	report.summaries = [
		...new Set(report.navigations.map((entry) => entry.index)),
	].map((navigation) => {
		const samples = measured.filter(
			(sample) => sample.navigation === navigation,
		);
		return {
			navigation,
			concurrency: config.concurrency,
			planned: config.samples,
			complete: samples.length === config.samples,
			...summarize(samples),
		};
	});
	return writeFile(
		join(output, "results.json"),
		`${JSON.stringify(report, null, 2)}\n`,
	);
};
const macChrome =
	"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const workers = [];
async function createWorker(id) {
	const browser = config.cdpUrl
		? await puppeteer.connect({ browserURL: config.cdpUrl })
		: await puppeteer.launch({
				headless: !config.headed,
				executablePath:
					process.env.CHROME_PATH ||
					(existsSync(macChrome) ? macChrome : undefined),
				args: [
					"--disable-background-timer-throttling",
					"--disable-renderer-backgrounding",
				],
			});
	const worker = { id, browser, page: null, sequence: 0, operation: 0 };
	workers.push(worker);
	report.browser = await browser.version();
	const page = await browser.newPage();
	worker.page = page;
	await page.setViewport({ width: 1440, height: 1000, deviceScaleFactor: 1 });
	page.on("pageerror", () => {
		report.browserErrorCount++;
	});
	if (config.storageState) {
		const statePath = resolve(dirname(configPath), config.storageState);
		const storage = JSON.parse(await readFile(statePath, "utf8"));
		const cookies = (storage.cookies ?? [])
			.filter((cookie) => {
				const domain = cookie.domain?.replace(/^\./, "");
				return (
					domain &&
					(app.hostname === domain || app.hostname.endsWith(`.${domain}`))
				);
			})
			.map(
				({
					name,
					value,
					domain,
					path,
					expires,
					httpOnly,
					secure,
					sameSite,
				}) => ({
					name,
					value,
					domain,
					path,
					...(expires > 0 ? { expires } : {}),
					httpOnly,
					secure,
					sameSite,
				}),
			);
		if (cookies.length) await page.setCookie(...cookies);
		const entries =
			storage.origins?.find((entry) => entry.origin === app.origin)
				?.localStorage ?? [];
		await page.evaluateOnNewDocument(
			(origin, entries) => {
				if (location.origin === origin)
					for (const entry of entries)
						localStorage.setItem(entry.name, entry.value);
			},
			app.origin,
			entries,
		);
	}

	await page.evaluateOnNewDocument(installAppMeasurement, {
		actions: config.actions,
		reset: config.reset,
		readySelector: config.readySelector,
		timeoutMs: config.timeoutMs,
		requireActionTrace: config.requireActionTrace,
	});
	const cdp = await page.createCDPSession();
	await cdp.send("Network.enable");
	worker.scope = { navigation: 0, phase: "bootstrap", sample: null };
	const requests = new Map();
	const executions = new Map();
	worker.executions = executions;
	cdp.on("Network.requestWillBeSent", (event) => {
		const category = requestCategory(event.request.url, event.request.method);
		if (!category) return;
		const record = {
			...worker.scope,
			worker: worker.id,
			category,
			routeKey: requestRouteKey(event.request.url, event.request.method),
			method: event.request.method,
			startedAtSeconds: event.timestamp,
			startedAtUnixMs: event.wallTime * 1000,
		};
		requests.set(event.requestId, record);
		if (category === "invoke")
			executions.set(record, observeExecution(cdp, event.requestId, record));
		report.network.push(record);
	});
	cdp.on("Network.responseReceived", (event) => {
		const record = requests.get(event.requestId);
		if (record)
			Object.assign(record, {
				responseHeadersMs: (event.timestamp - record.startedAtSeconds) * 1000,
				status: event.response.status,
				fromDiskCache: Boolean(event.response.fromDiskCache),
				fromServiceWorker: Boolean(event.response.fromServiceWorker),
				protocol: event.response.protocol,
			});
		executions.get(record)?.start();
	});
	cdp.on("Network.dataReceived", (event) => {
		executions.get(requests.get(event.requestId))?.data(event.data);
	});
	cdp.on("Network.loadingFinished", (event) => {
		const record = requests.get(event.requestId);
		if (record)
			Object.assign(record, {
				totalMs: (event.timestamp - record.startedAtSeconds) * 1000,
				encodedBytes: event.encodedDataLength,
			});
		requests.delete(event.requestId);
		void executions.get(record)?.finish();
	});
	cdp.on("Network.loadingFailed", (event) => {
		const record = requests.get(event.requestId);
		if (record)
			Object.assign(record, {
				totalMs: (event.timestamp - record.startedAtSeconds) * 1000,
				cancelled: Boolean(event.canceled),
				networkError: event.errorText,
			});
		requests.delete(event.requestId);
		void executions.get(record)?.finish();
	});

	return worker;
}
async function validateExecution(worker, result, invocationResponses) {
	const rejectedInvocation = invocationResponses.find(
		(record) => record.status >= 400,
	);
	await Promise.all(
		invocationResponses.map((record) =>
			worker.executions.get(record)?.wait(Math.min(config.timeoutMs, 5000)),
		),
	);
	result.execution = invocationResponses.map((record) => ({
		httpStatus: record.status,
		...record.execution,
	}));
	result.executionSuccessVerified =
		invocationResponses.length > 0 &&
		invocationResponses.every(
			(record) =>
				record.status >= 200 &&
				record.status < 300 &&
				record.execution?.success === true,
		);
	if (rejectedInvocation) {
		result.status = "error";
		result.error = `The invocation returned HTTP ${rejectedInvocation.status}`;
	} else if (
		config.requireTerminalSuccess &&
		!result.executionSuccessVerified
	) {
		result.status = "error";
		result.executionValidationError = invocationResponses.some(
			(record) =>
				record.execution?.terminalObserved && !record.execution?.success,
		)
			? "The invocation reported unsuccessful terminal execution"
			: "Successful terminal execution could not be verified";
		result.error ??= result.executionValidationError;
	}
}
async function navigate(worker, navigation) {
	const { page } = worker;
	worker.sequence = 0;
	worker.scope = { navigation, phase: "bootstrap", sample: null };
	const result = {
		worker: worker.id,
		index: navigation,
		kind: navigation === 0 ? "initial" : "repeat",
		status: "ok",
	};
	report.navigations.push(result);
	try {
		await page.goto(app.href, {
			waitUntil: "domcontentloaded",
			timeout: config.timeoutMs,
		});
		await page.bringToFront();
		await page.waitForFunction(
			() =>
				window.__flowLikeAppBenchmark?.navigation?.cardsFrameOpportunityMs !==
					undefined || window.__flowLikeAppBenchmark?.navigation?.error,
			{ timeout: config.timeoutMs },
		);
		Object.assign(
			result,
			await page.evaluate(() => {
				const navigation = performance.getEntriesByType("navigation")[0];
				return {
					...window.__flowLikeAppBenchmark.navigation,
					browserTimeOriginMs: performance.timeOrigin,
					pageLoadTimings: window.__flowLikeRunTimings?.findLast(
						(trace) => trace.label === "page onLoad",
					),
					document: navigation && {
						responseStartMs: navigation.responseStart,
						domContentLoadedMs: navigation.domContentLoadedEventEnd,
						loadEventMs: navigation.loadEventEnd,
						transferBytes: navigation.transferSize,
					},
				};
			}),
		);
		assert.ok(!result.error, result.error);
		if (config.requireActionTrace)
			await page.waitForFunction(
				() =>
					window.__flowLikeRunTimings?.some(
						(trace) => trace.label === "page onLoad",
					),
				{ timeout: config.timeoutMs },
			);
		Object.assign(
			result,
			await page.evaluate(() => ({
				pageLoadTimings: window.__flowLikeRunTimings?.findLast(
					(trace) => trace.label === "page onLoad",
				),
				pageOnLoadSettledObservedAtNavigationMs: performance.now(),
			})),
		);
		const invocationResponses = report.network.filter(
			(record) =>
				record.worker === worker.id &&
				record.navigation === navigation &&
				record.phase === "bootstrap" &&
				record.category === "invoke",
		);
		await validateExecution(worker, result, invocationResponses);
		assert.equal(
			result.status,
			"ok",
			"Bootstrap terminal success could not be verified",
		);
		result.bootstrapValidatedAtNavigationMs = await page.evaluate(() =>
			performance.now(),
		);
	} catch {
		result.status = "error";
		result.error ??=
			"Application bootstrap did not produce the configured ready actions";
		throw new Error(result.error);
	}
}
async function measure(
	worker,
	navigation,
	index,
	warmup,
	burst,
	scheduledAtUnixMs,
	reset = false,
) {
	const { page } = worker;
	const actionIndex = reset ? null : worker.sequence++ % config.actions.length;
	worker.scope = {
		navigation,
		phase: reset ? "reset" : warmup ? "warmup" : "action",
		operation: worker.operation++,
		sample: index,
		burst,
	};
	let sample;
	try {
		await page.evaluate(
			(action, reset) => window.__flowLikeAppBenchmark.arm(action, reset),
			actionIndex,
			reset,
		);
		const handle = await page.evaluateHandle(
			(index, reset) =>
				reset
					? window.__flowLikeAppBenchmark.resetElement()
					: window.__flowLikeAppBenchmark.actionElement(index),
			actionIndex,
			reset,
		);
		try {
			assert.ok(handle.asElement(), "The configured action disappeared");
			await handle.asElement().click();
		} finally {
			await handle.dispose();
		}
		await page.waitForFunction(() => window.__flowLikeAppBenchmark?.result, {
			timeout: config.timeoutMs + 5000,
		});
		sample = await page.evaluate(() => ({
			...window.__flowLikeAppBenchmark.result,
			browserTimeOriginMs: performance.timeOrigin,
		}));
	} catch {
		sample = {
			actionIndex,
			status: "error",
			error: reset
				? "The reset could not be clicked or the person cards did not return"
				: "The action could not be clicked or its expected history did not arrive",
		};
	}
	const invocationResponses = report.network.filter(
		(record) =>
			record.worker === worker.id &&
			record.navigation === navigation &&
			record.phase === worker.scope.phase &&
			record.operation === worker.scope.operation &&
			record.category === "invoke",
	);
	await validateExecution(worker, sample, invocationResponses);

	(reset ? report.resets : report.samples).push({
		navigation,
		index,
		worker: worker.id,
		warmup,
		burst,
		scheduledAtUnixMs,
		scheduledToClickMs:
			sample.clickAtNavigationMs === undefined
				? undefined
				: sample.browserTimeOriginMs +
					sample.clickAtNavigationMs -
					scheduledAtUnixMs,
		...sample,
	});
	return sample;
}
async function prepareRound(active, navigation, index, warmup, burst) {
	if (!config.reset) return;
	const preparations = await Promise.all(
		active
			.filter((worker) => worker.sequence > 0)
			.map((worker) =>
				measure(worker, navigation, index, warmup, burst, Date.now(), true),
			),
	);
	await save();
	assert.ok(
		preparations.every((sample) => sample.status === "ok"),
		"A reset action failed; partial results were retained",
	);
}
async function coldActionBoundary(worker, navigation) {
	const bootstrap = report.navigations.find(
		(entry) => entry.worker === worker.id && entry.index === navigation,
	);
	assert.equal(bootstrap?.executionSuccessVerified, true);
	const documentTimeOriginMs = await worker.page.evaluate(
		() => performance.timeOrigin,
	);
	const started = performance.now();
	const boundary = {
		mode: "cold-action",
		status: "waiting",
		worker: worker.id,
		navigation,
		bootstrapExecutionSuccessVerified: true,
		startedAt: new Date().toISOString(),
		documentTimeOriginMs,
	};
	report.coldBoundary = boundary;
	worker.scope = { navigation, phase: "cold-boundary", sample: null };
	await save();
	try {
		await waitForColdActionController({ timeoutMs: coldActionTimeoutMs });
		boundary.controllerRestartConfirmed = true;
		boundary.confirmedAt = new Date().toISOString();
		boundary.documentPreserved =
			(await worker.page.evaluate(() => performance.timeOrigin)) ===
			documentTimeOriginMs;
		assert.ok(boundary.documentPreserved, "The browser navigated during the cold-action boundary");
		const requests = report.network.filter(
			(record) => record.worker === worker.id && record.phase === "cold-boundary",
		);
		boundary.networkRequestCategories = Object.fromEntries(
			[...new Set(requests.map((record) => record.category))].map((category) => [
				category,
				requests.filter((record) => record.category === category).length,
			]),
		);
		boundary.unexpectedExecutionRequests = requests.filter((record) =>
			["bootstrap", "prerun", "invoke"].includes(record.category),
		).length;
		assert.equal(
			boundary.unexpectedExecutionRequests,
			0,
			"The app requested workflow preparation or execution during the cold-action boundary",
		);
		boundary.status = "confirmed";
	} catch (error) {
		boundary.status = "failed";
		boundary.error = error.message;
		throw error;
	} finally {
		boundary.elapsedMs = performance.now() - started;
		await save();
	}
}
try {
	const health = await fetch(`${app.origin}/__benchmark/health`, {
		signal: AbortSignal.timeout(config.timeoutMs),
	}).catch(() => null);
	if (
		health?.ok &&
		health.headers.get("content-type")?.includes("application/json")
	)
		report.web = await health.json();
	// Creating each browser sequentially avoids including browser startup contention in navigation timing.
	for (let id = 0; id < config.concurrency; id++) await createWorker(id);
	for (let navigation = 0; navigation < config.navigations; navigation++) {
		const bootstraps = await Promise.allSettled(
			workers.map((worker) => navigate(worker, navigation)),
		);
		await save();
		const failedBootstrap = bootstraps.find(
			(result) => result.status === "rejected",
		);
		if (failedBootstrap) throw failedBootstrap.reason;
		if (args.coldActionControl) await coldActionBoundary(workers[0], navigation);
		for (let round = 0; round < config.warmup; round++) {
			await prepareRound(workers, navigation, round, true, round);
			const scheduled = Date.now();
			const samples = await Promise.all(
				workers.map((worker) =>
					measure(worker, navigation, round, true, round, scheduled),
				),
			);
			await save();
			assert.ok(
				samples.every((sample) => sample.status === "ok"),
				"A warmup action failed; partial results were retained",
			);
		}
		for (let offset = 0; offset < config.samples; offset += workers.length) {
			const active = workers.slice(
				0,
				Math.min(workers.length, config.samples - offset),
			);
			await prepareRound(
				active,
				navigation,
				offset,
				false,
				Math.floor(offset / workers.length),
			);
			const scheduled = Date.now();
			const samples = await Promise.all(
				active.map((worker, index) =>
					measure(
						worker,
						navigation,
						offset + index,
						false,
						Math.floor(offset / workers.length),
						scheduled,
					),
				),
			);
			await save();
			assert.ok(
				samples.every((sample) => sample.status === "ok"),
				"A measured action failed; partial results were retained",
			);
		}
		const summary = report.summaries.find(
			(entry) => entry.navigation === navigation,
		);
		console.log(
			`Navigation ${navigation}, concurrency ${config.concurrency}: ${summary.attempts} measured actions; history DOM p50=${summary.metrics.clickToHistoryDomMs.p50?.toFixed(1)}ms p95=${summary.metrics.clickToHistoryDomMs.p95?.toFixed(1)}ms; frame opportunity p95=${summary.metrics.clickToHistoryFrameOpportunityMs.p95?.toFixed(1)}ms`,
		);
	}
	assert.equal(
		report.browserErrorCount,
		0,
		"The application raised browser errors; inspect it before trusting the benchmark",
	);
	report.status = "passed";
} finally {
	if (report.status === "running") report.status = "failed";
	report.finishedAt = new Date().toISOString();
	try {
		await save();
	} finally {
		const cleanup = await Promise.allSettled(
			workers.map(async ({ browser, page }) => {
				try {
					await page?.close();
				} finally {
					if (config.cdpUrl) browser.disconnect();
					else {
						const child = browser.process();
						try {
							await browser.close();
							assert.ok(
								!child || child.exitCode !== null || child.signalCode !== null,
								"Owned Chrome process did not exit",
							);
						} finally {
							// Concurrent Chrome launches can leave inherited stdio open after their processes exit.
							for (const stream of child?.stdio ?? []) stream?.destroy();
						}
					}
				}
			}),
		);
		assert.ok(
			cleanup.every((result) => result.status === "fulfilled"),
			"Owned browser cleanup failed",
		);
	}
}
console.log(`Results: ${join(output, "results.json")}`);

// All reports are flushed and owned browser processes have exited.
process.exit(0);
