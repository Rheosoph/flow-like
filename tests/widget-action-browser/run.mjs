import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { mkdir, writeFile } from "node:fs/promises";
import os from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import puppeteer from "puppeteer";
import { summarize } from "./measurement.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const options = {
	origin: "http://127.0.0.1:4342",
	output: join(root, "tmp/widget-action-benchmark/browser"),
	samples: 30,
	warmup: 3,
	variants: ["baseline", "overlap", "bundled", "optimized"],
	caches: ["warm", "cold"],
	timeout: 30_000,
	headed: false,
};
for (let index = 2; index < process.argv.length; index++) {
	const [key, inline] = process.argv[index].replace(/^--/, "").split("=", 2);
	if (key === "help") {
		console.log(
			"node tests/widget-action-browser/run.mjs [--samples 30] [--warmup 3] [--variants baseline,overlap,bundled,optimized] [--caches warm,cold] [--origin http://127.0.0.1:4342] [--output PATH] [--timeout 30000] [--headed]",
		);
		process.exit(0);
	}
	if (key === "headed") {
		options.headed = inline !== "false";
		continue;
	}
	const value = inline ?? process.argv[++index];
	assert.ok(
		value !== undefined && key in options,
		`Unknown or incomplete option: --${key}`,
	);
	if (["samples", "warmup", "timeout"].includes(key))
		options[key] = Number(value);
	else if (["variants", "caches"].includes(key))
		options[key] = value.split(",");
	else options[key] = value;
}
assert.ok(
	Number.isInteger(options.samples) && options.samples > 0,
	"samples must be a positive integer",
);
assert.ok(
	Number.isInteger(options.warmup) && options.warmup >= 0,
	"warmup must be a nonnegative integer",
);
assert.ok(
	Number.isInteger(options.timeout) && options.timeout > 0,
	"timeout must be a positive integer",
);
assert.ok(
	options.variants.length &&
		options.variants.every((v) =>
			["baseline", "overlap", "bundled", "optimized"].includes(v),
		),
	"Unknown variant",
);
assert.ok(
	options.caches.length &&
		options.caches.every((v) => ["warm", "cold"].includes(v)),
	"Unknown cache mode",
);
const origin = new URL(options.origin);
assert.ok(
	origin.protocol === "http:" &&
		["127.0.0.1", "localhost", "[::1]"].includes(origin.hostname),
	"This benchmark only targets a local HTTP fixture",
);
assert.ok(
	!origin.username && !origin.password,
	"The local benchmark does not use credentials",
);
const health = await fetch(new URL("/benchmark/health", origin), {
	signal: AbortSignal.timeout(options.timeout),
});
assert.ok(
	health.ok,
	`Local benchmark server is not ready (HTTP ${health.status})`,
);
await mkdir(options.output, { recursive: true });
const report = {
	schemaVersion: 1,
	status: "running",
	startedAt: new Date().toISOString(),
	options,
	server: await health.json(),
	machine: {
		platform: os.platform(),
		release: os.release(),
		arch: os.arch(),
		cpu: os.cpus()[0]?.model,
		logicalCpus: os.cpus().length,
		memoryBytes: os.totalmem(),
	},
	browser: null,
	viewport: { width: 1440, height: 1000, deviceScaleFactor: 1 },
	metricNotes: [
		"Click timings start in the real widget action's DOM click handler; synthetic browser input does not measure hardware input latency.",
		"History DOM readiness requires the requested person's rental marker and the expected visible row count after production A2UI updates.",
		"Frame opportunity is two requestAnimationFrame callbacks after matching DOM content. It does not prove physical pixels were presented.",
		"The fixture uses production action handling, surface updates, widget instances, and table rendering with a registry limited to its component types.",
		"The local backend adapter replaces hosted authentication, prerun, and API dispatch. Server timing fields describe the local executor and injected transport profile.",
		"Warmups remain in raw samples but do not contribute to summaries. Failed trials are counted and make the runner fail.",
	],
	errors: [],
	samples: [],
	summaries: [],
};
const save = () => {
	const measured = report.samples.filter((sample) => !sample.warmup);
	report.overall = {
		...summarize(measured),
		planned: options.samples * options.variants.length * options.caches.length,
	};
	report.warmups = summarize(report.samples.filter((sample) => sample.warmup));
	report.summaries = [
		...new Set(
			report.samples.map((sample) => `${sample.variant}/${sample.cache}`),
		),
	].map((key) => {
		const [variant, cache] = key.split("/");
		const samples = measured.filter(
			(sample) => sample.variant === variant && sample.cache === cache,
		);
		return {
			variant,
			cache,
			planned: options.samples,
			complete: samples.length === options.samples,
			...summarize(samples),
		};
	});
	return writeFile(
		join(options.output, "results.json"),
		`${JSON.stringify(report, null, 2)}\n`,
	);
};
const macChrome =
	"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const browser = await puppeteer.launch({
	headless: !options.headed,
	executablePath:
		process.env.CHROME_PATH || (existsSync(macChrome) ? macChrome : undefined),
	args: [
		"--disable-background-timer-throttling",
		"--disable-renderer-backgrounding",
		"--disable-backgrounding-occluded-windows",
	],
	defaultViewport: report.viewport,
});
try {
	report.browser = await browser.version();
	const page = await browser.newPage();
	page.on("pageerror", (error) => report.errors.push(error.message));
	await page.goto(origin.href, {
		waitUntil: "networkidle0",
		timeout: options.timeout,
	});
	await page.bringToFront();
	await page.waitForFunction(() => window.__widgetBenchmark?.ready, {
		timeout: options.timeout,
	});
	for (const cache of options.caches) {
		for (const variant of options.variants) {
			for (let index = 0; index < options.warmup + options.samples; index++) {
				const trial = {
					variant,
					cache,
					personId: 1 + (index % 2),
					timeoutMs: options.timeout,
				};
				const warmup = index < options.warmup;
				let sample;
				try {
					const before = await page.evaluate(async (config) => {
						await window.__widgetBenchmark.prepare(config);
						return window.__widgetBenchmark.results.length;
					}, trial);
					await page.click(`[data-benchmark-person="${trial.personId}"]`);
					await page.waitForFunction(
						(count) => window.__widgetBenchmark.results.length > count,
						{ timeout: options.timeout + 5000 },
						before,
					);
					sample = await page.evaluate(() =>
						window.__widgetBenchmark.results.at(-1),
					);
				} catch (error) {
					sample = { ...trial, status: "error", error: error.message };
				}
				report.samples.push({ ...sample, warmup });
				await save();
				if (sample.status !== "ok")
					throw new Error(`${variant}/${cache}: ${sample.error}`);
			}
			const summary = report.summaries.find(
				(entry) => entry.variant === variant && entry.cache === cache,
			);
			console.log(
				`${variant.padEnd(9)} ${cache.padEnd(4)} DOM p50=${summary.metrics.clickToHistoryDomMs.p50?.toFixed(1)}ms p95=${summary.metrics.clickToHistoryDomMs.p95?.toFixed(1)}ms; frame opportunity p95=${summary.metrics.clickToHistoryFrameOpportunityMs.p95?.toFixed(1)}ms; settled p95=${summary.metrics.clickToSettledMs.p95?.toFixed(1)}ms`,
			);
		}
	}
	assert.equal(
		report.errors.length,
		0,
		`Browser errors: ${report.errors.join("; ")}`,
	);
	report.status = "passed";
} finally {
	if (report.status === "running") report.status = "failed";
	report.finishedAt = new Date().toISOString();
	try {
		await save();
	} finally {
		await browser.close();
	}
}
console.log(`Results: ${join(options.output, "results.json")}`);
