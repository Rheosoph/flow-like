import { existsSync, readFileSync, readdirSync } from "node:fs";
import { mkdir, rm } from "node:fs/promises";
import { homedir } from "node:os";
import {
	basename,
	dirname,
	join,
	normalize,
	relative,
	resolve,
} from "node:path";
import { parseArgs } from "node:util";
import puppeteer, { type BrowserContext, type Page } from "puppeteer";
import { PACKAGE_DIR, buildHarness } from "./build";
import {
	type EditorVisualApi,
	SURFACES,
	type Surface,
	THEMES,
	type Theme,
	VISUAL_CASES,
	type VisualCase,
} from "./cases";

const USAGE = `Renders the Plate corpus through the real editor components in Chrome and screenshots every case.

  bun scripts/editor-visual/run.ts --out <dir> [options]

  --out <dir>          run directory (bundle/, shots/<theme>/<surface>/<id>.png, manifest.json)
  --label <name>       name stored in the manifest (default: basename of --out)
  --filter <regex>     only case keys ("<surface>/<id>") matching the pattern
  --surfaces <list>    comma separated subset of ${SURFACES.join(", ")}
  --themes <list>      comma separated subset of ${THEMES.join(", ")}
  --mode <mode>        React build: development (default, keeps React warnings) or production
  --chrome <path>      browser binary (default: newest Chrome for Testing in ~/.cache/puppeteer)
  --bundle <dir>       reuse a bundle from an earlier run instead of building
  --workers <n>        pages rendering in parallel (default 4)
`;

const VIEWPORT = { width: 960, height: 900, deviceScaleFactor: 1 } as const;
const FIXED_NOW = Date.UTC(2026, 0, 15, 12, 0, 0);
const SETTLE_POLL_MS = 100;
const SETTLE_STABLE_POLLS = 4;
/**
 * Chrome's native media controls keep spinning inside their user-agent shadow
 * root (out of reach of the page CSS) until a loading animation iteration ends,
 * about 1.5 s after the element fails to load.
 */
const MEDIA_STABLE_POLLS = 25;
const SETTLE_TIMEOUT_MS = 20_000;
/**
 * A starved machine occasionally stalls a page for good (maplibre maps that
 * never fire `load` were seen at load averages above 200); such a case is
 * rendered again in a fresh tab and the attempts are recorded.
 */
const MAX_ATTEMPTS = 3;
const NAVIGATION_TIMEOUT_MS = 30_000;
const RETRY_NAVIGATION_TIMEOUT_MS = 120_000;
const MAX_VIEWPORT_GROWTH = 6;
const PARKED_MOUSE = { x: VIEWPORT.width - 1, y: 1 } as const;

type HarnessWindow = Window & { editorVisual: EditorVisualApi };

export type CaseRecord = {
	readonly key: string;
	readonly surface: Surface;
	readonly theme: Theme;
	readonly id: string;
	readonly file: string | null;
	readonly width: number;
	readonly height: number;
	readonly settled: boolean;
	readonly attempts: number;
	readonly durationMs: number;
	readonly renderError: string | null;
	readonly harnessError: string | null;
	readonly pageErrors: readonly string[];
	readonly consoleErrors: readonly string[];
	readonly consoleWarnings: readonly string[];
	readonly blockedRequests: readonly string[];
	readonly localMisses: readonly string[];
};

export type RunManifest = {
	readonly label: string;
	readonly createdAt: string;
	readonly gitHead: string | null;
	readonly versions: Record<string, string | null>;
	readonly browser: { readonly path: string; readonly version: string };
	readonly viewport: typeof VIEWPORT;
	readonly mode: string;
	readonly bundle: { readonly dir: string; readonly bytes: number | null };
	readonly durationMs: number;
	readonly cases: readonly CaseRecord[];
};

const REPORTED_PACKAGES = [
	"platejs",
	"@platejs/core",
	"@platejs/markdown",
	"@platejs/ai",
	"react",
	"react-dom",
	"katex",
	"tailwindcss",
	"puppeteer",
];

function installedVersion(name: string): string | null {
	for (const root of [PACKAGE_DIR, resolve(PACKAGE_DIR, "../..")]) {
		const manifest = join(root, "node_modules", name, "package.json");
		if (existsSync(manifest))
			return JSON.parse(readFileSync(manifest, "utf8")).version ?? null;
	}
	return null;
}

function gitHead(): string | null {
	const result = Bun.spawnSync(["git", "rev-parse", "HEAD"], {
		cwd: PACKAGE_DIR,
	});
	return result.success ? result.stdout.toString().trim() : null;
}

const CHROME_FOR_TESTING_BINARIES = [
	"chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
	"chrome-mac-x64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
	"chrome-linux64/chrome",
	"chrome-win64/chrome.exe",
];

const versionParts = (name: string) =>
	(name.split("-").pop() ?? "").split(".").map((part) => Number(part) || 0);

function compareVersionsDescending(left: string, right: string) {
	const a = versionParts(left);
	const b = versionParts(right);
	for (let index = 0; index < Math.max(a.length, b.length); index++) {
		const delta = (b[index] ?? 0) - (a[index] ?? 0);
		if (delta !== 0) return delta;
	}
	return 0;
}

function findChromeForTesting(): string {
	const cache = join(homedir(), ".cache", "puppeteer", "chrome");
	const installs = existsSync(cache)
		? readdirSync(cache).sort(compareVersionsDescending)
		: [];
	for (const install of installs) {
		for (const binary of CHROME_FOR_TESTING_BINARIES) {
			const candidate = join(cache, install, binary);
			if (existsSync(candidate)) return candidate;
		}
	}
	throw new Error(
		`editor-visual: no Chrome for Testing under ${cache}. Install the revision puppeteer pins with "node_modules/.bin/puppeteer browsers install chrome" from the repo root, or pass --chrome.`,
	);
}

/** Runs in the page before any bundle code: a fixed clock, a seedable Math.random and empty storage. */
function installDeterminism(fixedNow: number) {
	const RealDate = Date;
	const startedAt = RealDate.now();
	const now = () => fixedNow + (RealDate.now() - startedAt);
	class FixedDate extends RealDate {
		constructor(...args: unknown[]) {
			super(...((args.length > 0 ? args : [now()]) as [number]));
		}
		static now() {
			return now();
		}
	}
	globalThis.Date = FixedDate as DateConstructor;

	let state = 1;
	Math.random = () => {
		state = (state + 0x6d2b79f5) | 0;
		let mixed = Math.imul(state ^ (state >>> 15), 1 | state);
		mixed = (mixed + Math.imul(mixed ^ (mixed >>> 7), 61 | mixed)) ^ mixed;
		return ((mixed ^ (mixed >>> 14)) >>> 0) / 4294967296;
	};
	Object.assign(window, {
		__seedRandom: (seed: number) => {
			state = seed | 0 || 1;
		},
	});
	try {
		localStorage.clear();
		sessionStorage.clear();
	} catch {}
}

/** A hash of everything that can still change on screen; unchanged across consecutive polls means settled. */
async function pageSignature(): Promise<{
	key: string;
	media: number;
	pending: number;
}> {
	await new Promise<void>((resolveFrame) => {
		let done = false;
		const finish = () => {
			if (done) return;
			done = true;
			resolveFrame();
		};
		requestAnimationFrame(() => requestAnimationFrame(finish));
		setTimeout(finish, 250);
	});
	const markup = document.body.innerHTML;
	let hash = 0;
	for (let index = 0; index < markup.length; index++) {
		hash = (Math.imul(hash, 31) + markup.charCodeAt(index)) | 0;
	}
	const pendingImages = [...document.images].filter(
		(image) => !image.complete,
	).length;
	const media = [...document.querySelectorAll("video, audio")].map(
		(element) => {
			const player = element as HTMLMediaElement;
			return `${player.networkState}/${player.readyState}/${player.error?.code ?? 0}`;
		},
	);
	const pending = (
		window as unknown as HarnessWindow
	).editorVisual.pendingWork();
	const stage = document.getElementById("stage")?.getBoundingClientRect();
	const key = [
		markup.length,
		hash,
		pendingImages,
		document.fonts.status,
		stage ? `${stage.width}x${stage.height}` : "no-stage",
		document.documentElement.className,
		media.join(","),
		pending,
	].join(":");
	return { key, media: media.length, pending };
}

/** The stage plus anything floating over it (toolbars, menus, popovers). */
function captureRect() {
	const stage = document.getElementById("stage") ?? document.body;
	const rects = [stage.getBoundingClientRect()];
	const overlays = document.querySelectorAll(
		'[data-radix-popper-content-wrapper], [role="toolbar"], [role="menu"], [role="listbox"], [role="dialog"]',
	);
	for (const overlay of overlays) {
		const rect = overlay.getBoundingClientRect();
		const style = getComputedStyle(overlay);
		if (rect.width === 0 || rect.height === 0) continue;
		if (style.visibility === "hidden" || style.display === "none") continue;
		rects.push(rect);
	}
	const left = Math.max(0, Math.floor(Math.min(...rects.map((r) => r.left))));
	const top = Math.max(0, Math.floor(Math.min(...rects.map((r) => r.top))));
	const right = Math.ceil(Math.max(...rects.map((r) => r.right)));
	const bottom = Math.ceil(Math.max(...rects.map((r) => r.bottom)));
	return { x: left, y: top, width: right - left, height: bottom - top };
}

function firstWordCenter() {
	const editor = document.querySelector('[data-slate-editor="true"]');
	if (!editor) return null;
	const blocks = [...editor.querySelectorAll(".slate-p"), editor];
	for (const block of blocks) {
		const walker = document.createTreeWalker(block, NodeFilter.SHOW_TEXT);
		for (let node = walker.nextNode(); node; node = walker.nextNode()) {
			if (!node.parentElement?.closest("[data-slate-string]")) continue;
			const match = /\p{L}{4,}/u.exec(node.textContent ?? "");
			if (!match) continue;
			const range = document.createRange();
			range.setStart(node, match.index);
			range.setEnd(node, match.index + match[0].length);
			const rect = range.getBoundingClientRect();
			return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
		}
	}
	return null;
}

function firstBlockStart() {
	const block = document.querySelector(
		'[data-slate-editor="true"] [data-slate-node="element"]',
	);
	if (!block) return null;
	const rect = block.getBoundingClientRect();
	return { x: rect.left + 8, y: rect.top + rect.height / 2 };
}

class CaseLog {
	readonly pageErrors: string[] = [];
	readonly consoleErrors: string[] = [];
	readonly consoleWarnings: string[] = [];
	readonly blocked = new Set<string>();
	readonly localMisses = new Set<string>();
	readonly inflight = new Set<unknown>();
}

type Tab = {
	readonly page: Page;
	log: CaseLog;
	viewportHeight: number;
};

type RenderWorker = {
	readonly context: BrowserContext;
	readonly origin: string;
	tab?: Tab;
};

const unique = (values: readonly string[]) => [...new Set(values)];
const firstLine = (text: string) =>
	(text.split("\n").find((line) => line.trim()) ?? "").trim().slice(0, 400);
/** Chrome prints GPU object addresses in some warnings; they differ per run. */
const POINTER = /0x[0-9a-f]{6,}/gi;
/**
 * Unresolvable external hosts (already in `blockedRequests`) and GPU driver
 * notices that Chrome prints once per browser, in whichever page reads pixels first.
 */
const BROWSER_NOISE =
	/^Failed to load resource: net::ERR_NAME_NOT_RESOLVED|GL Driver Message/;
const errorText = (error: unknown) =>
	firstLine(error instanceof Error ? error.message : String(error));

async function openTab(context: BrowserContext, origin: string): Promise<Tab> {
	const page = await context.newPage();
	const tab: Tab = {
		page,
		log: new CaseLog(),
		viewportHeight: VIEWPORT.height,
	};
	const clean = (text: string) =>
		firstLine(text).replaceAll(origin, "<harness>").replace(POINTER, "0x…");
	await page.setViewport(VIEWPORT);
	await page.emulateTimezone("UTC");
	const session = await page.createCDPSession();
	await session.send("Emulation.setFocusEmulationEnabled", { enabled: true });
	await page.evaluateOnNewDocument(installDeterminism, FIXED_NOW);
	page.on("dialog", (dialog) => void dialog.dismiss());
	page.on("request", (request) => {
		const url = request.url();
		if (url.startsWith(origin)) tab.log.inflight.add(request);
		else if (!url.startsWith("data:") && !url.startsWith("blob:"))
			tab.log.blocked.add(url.slice(0, 200));
	});
	page.on("requestfinished", (request) => tab.log.inflight.delete(request));
	page.on("requestfailed", (request) => tab.log.inflight.delete(request));
	page.on("response", (response) => {
		if (response.url().startsWith(origin) && response.status() >= 400)
			tab.log.localMisses.add(`${response.status()} ${clean(response.url())}`);
	});
	page.on("console", (message) => {
		const type = message.type();
		const text = clean(message.text());
		if (BROWSER_NOISE.test(text)) return;
		if (type === "error") tab.log.consoleErrors.push(text);
		else if (type === "warn") tab.log.consoleWarnings.push(text);
	});
	page.on("pageerror", (error) =>
		tab.log.pageErrors.push(clean(errorText(error))),
	);
	return tab;
}

/**
 * Every case renders in a fresh document, so module state such as parse caches
 * and resolved plugin objects never carries over. `lng=en` selects the bundled
 * source locale, so i18next fetches no catalogs.
 */
async function loadHarness(
	tab: Tab,
	origin: string,
	theme: Theme,
	timeout: number,
) {
	const { page } = tab;
	tab.log = new CaseLog();
	if (tab.viewportHeight !== VIEWPORT.height) {
		tab.viewportHeight = VIEWPORT.height;
		await page.setViewport(VIEWPORT);
	}
	await page.emulateMediaFeatures([
		{ name: "prefers-reduced-motion", value: "reduce" },
		{ name: "prefers-color-scheme", value: theme },
	]);
	await page.goto(`${origin}/index.html?lng=en`, {
		waitUntil: "domcontentloaded",
		timeout,
	});
	await page.waitForFunction(() => "editorVisual" in window, { timeout });
	tab.log.inflight.clear();
	await page.mouse.move(PARKED_MOUSE.x, PARKED_MOUSE.y);
}

/**
 * Reloading one tab is ten times cheaper than opening one, but on a starved
 * machine a navigation can stall behind the previous document; that case gets
 * a new tab instead of a missing screenshot.
 */
async function prepareTab(
	worker: RenderWorker,
	theme: Theme,
	fresh: boolean,
): Promise<Tab> {
	if (fresh && worker.tab) {
		await worker.tab.page.close().catch(() => {});
		worker.tab = undefined;
	}
	worker.tab ??= await openTab(worker.context, worker.origin);
	try {
		await loadHarness(worker.tab, worker.origin, theme, NAVIGATION_TIMEOUT_MS);
		return worker.tab;
	} catch (error) {
		console.warn(
			`editor-visual: navigation stalled (${errorText(error)}), retrying in a new tab`,
		);
		await worker.tab.page.close().catch(() => {});
		worker.tab = await openTab(worker.context, worker.origin);
		await loadHarness(
			worker.tab,
			worker.origin,
			theme,
			RETRY_NAVIGATION_TIMEOUT_MS,
		);
		return worker.tab;
	}
}

async function waitForStable(tab: Tab): Promise<boolean> {
	const deadline = performance.now() + SETTLE_TIMEOUT_MS;
	let previous = "";
	let stablePolls = 0;
	while (performance.now() < deadline) {
		await Bun.sleep(SETTLE_POLL_MS);
		const signature = await tab.page.evaluate(pageSignature);
		const quiet = tab.log.inflight.size === 0 && signature.pending === 0;
		if (signature.key === previous && quiet) {
			stablePolls++;
			const required =
				signature.media > 0 ? MEDIA_STABLE_POLLS : SETTLE_STABLE_POLLS;
			if (stablePolls >= required) return true;
		} else {
			stablePolls = 0;
			previous = signature.key;
		}
	}
	return false;
}

async function interact(tab: Tab, surface: Surface) {
	const { page } = tab;
	if (surface === "editable-selection") {
		const point = await page.evaluate(firstWordCenter);
		if (!point) throw new Error("no word to select in the editable editor");
		await page.mouse.click(point.x, point.y, { count: 2 });
	} else if (surface === "editable-slash") {
		const point = await page.evaluate(firstBlockStart);
		if (!point) throw new Error("no block to type into in the editable editor");
		await page.mouse.click(point.x, point.y);
		await page.keyboard.type("/");
	} else {
		return;
	}
	await page.mouse.move(PARKED_MOUSE.x, PARKED_MOUSE.y);
}

async function fitViewport(tab: Tab): Promise<boolean> {
	let settled = true;
	for (let growth = 0; growth < MAX_VIEWPORT_GROWTH; growth++) {
		const rect = await tab.page.evaluate(captureRect);
		const bottom = rect.y + rect.height;
		if (bottom <= tab.viewportHeight) break;
		tab.viewportHeight = Math.ceil(bottom / 100) * 100;
		await tab.page.setViewport({ ...VIEWPORT, height: tab.viewportHeight });
		settled = await waitForStable(tab);
	}
	return settled;
}

async function screenshotCase(
	tab: Tab,
	visualCase: VisualCase,
	theme: Theme,
	file: string,
) {
	const { page } = tab;
	await page.evaluate(
		(key, caseTheme) =>
			(window as unknown as HarnessWindow).editorVisual.render(key, caseTheme),
		visualCase.key,
		theme,
	);
	let settled = await waitForStable(tab);
	await interact(tab, visualCase.surface);
	if (visualCase.surface.startsWith("editable-"))
		settled = (await waitForStable(tab)) && settled;
	settled = (await fitViewport(tab)) && settled;
	const renderError = await page.evaluate(() =>
		(window as unknown as HarnessWindow).editorVisual.renderError(),
	);
	const clip = await page.evaluate(captureRect);
	await mkdir(dirname(file), { recursive: true });
	await page.screenshot({
		path: file as `${string}.png`,
		clip,
		captureBeyondViewport: false,
	});
	return { settled, renderError, width: clip.width, height: clip.height };
}

async function captureCase(
	worker: RenderWorker,
	visualCase: VisualCase,
	theme: Theme,
	shotsDir: string,
	runDir: string,
): Promise<CaseRecord> {
	const started = performance.now();
	const file = join(
		shotsDir,
		theme,
		visualCase.surface,
		`${visualCase.id}.png`,
	);
	let result = { settled: false, renderError: null as string | null };
	let size = { width: 0, height: 0 };
	let harnessError: string | null = null;
	let attempts = 0;
	while (attempts < MAX_ATTEMPTS && !result.settled) {
		attempts++;
		harnessError = null;
		try {
			const tab = await prepareTab(worker, theme, attempts > 1);
			const shot = await screenshotCase(tab, visualCase, theme, file);
			result = shot;
			size = { width: shot.width, height: shot.height };
		} catch (error) {
			harnessError = errorText(error);
		}
	}
	const log = worker.tab?.log ?? new CaseLog();

	return {
		key: `${theme}/${visualCase.surface}/${visualCase.id}`,
		surface: visualCase.surface,
		theme,
		id: visualCase.id,
		file: harnessError ? null : relative(runDir, file),
		...size,
		settled: result.settled,
		attempts,
		durationMs: Math.round(performance.now() - started),
		renderError: result.renderError,
		harnessError,
		pageErrors: unique(log.pageErrors),
		consoleErrors: unique(log.consoleErrors),
		consoleWarnings: unique(log.consoleWarnings),
		blockedRequests: [...log.blocked].sort(),
		localMisses: [...log.localMisses].sort(),
	};
}

/** Lets Chrome reuse its compiled-code cache across the per-case reloads. */
const IMMUTABLE = { "Cache-Control": "public, max-age=86400, immutable" };

function serveBundle(dir: string) {
	const root = normalize(dir);
	return Bun.serve({
		hostname: "127.0.0.1",
		port: 0,
		async fetch(request) {
			const path = decodeURIComponent(new URL(request.url).pathname);
			const target = normalize(join(root, path === "/" ? "index.html" : path));
			if (relative(root, target).startsWith(".."))
				return new Response("forbidden", { status: 403 });
			const file = Bun.file(target);
			if (!(await file.exists()))
				return new Response("missing", { status: 404 });
			return target.endsWith(".html")
				? new Response(file)
				: new Response(file, { headers: IMMUTABLE });
		},
	});
}

const listOption = <T extends string>(
	value: string | undefined,
	allowed: readonly T[],
	flag: string,
): readonly T[] => {
	if (!value) return allowed;
	const picked = value.split(",").map((entry) => entry.trim());
	const unknown = picked.filter((entry) => !allowed.includes(entry as T));
	if (unknown.length > 0)
		throw new Error(`editor-visual: unknown ${flag} ${unknown.join(", ")}`);
	return picked as T[];
};

async function main() {
	const { values } = parseArgs({
		options: {
			out: { type: "string" },
			label: { type: "string" },
			filter: { type: "string" },
			surfaces: { type: "string" },
			themes: { type: "string" },
			mode: { type: "string", default: "development" },
			chrome: { type: "string" },
			bundle: { type: "string" },
			workers: { type: "string", default: "4" },
			help: { type: "boolean", default: false },
		},
	});
	if (values.help || !values.out) {
		console.log(USAGE);
		process.exit(values.help ? 0 : 1);
	}
	const mode = values.mode === "production" ? "production" : "development";
	const runDir = resolve(values.out);
	const shotsDir = join(runDir, "shots");
	const surfaces = listOption(values.surfaces, SURFACES, "surface");
	const themes = listOption(values.themes, THEMES, "theme");
	const filter = values.filter ? new RegExp(values.filter) : null;
	const workerCount = Math.max(1, Number(values.workers) || 1);
	const cases = VISUAL_CASES.filter(
		(visualCase) =>
			surfaces.includes(visualCase.surface) &&
			(!filter || filter.test(visualCase.key)),
	);
	if (cases.length === 0) throw new Error("editor-visual: no case matches");

	const started = performance.now();
	await rm(shotsDir, { recursive: true, force: true });
	let bundleDir = values.bundle
		? resolve(values.bundle)
		: join(runDir, "bundle");
	let bundleBytes: number | null = null;
	if (!values.bundle) {
		const build = await buildHarness(bundleDir, mode);
		bundleDir = build.dir;
		bundleBytes = build.bundleBytes;
		console.log(
			`built harness in ${build.durationMs} ms (${Math.round(build.bundleBytes / 1024 / 1024)} MB js, ${Math.round(build.cssBytes / 1024)} KB tailwind)`,
		);
	}

	const chromePath =
		values.chrome ?? process.env.EDITOR_VISUAL_CHROME ?? findChromeForTesting();
	const server = serveBundle(bundleDir);
	const origin = `http://127.0.0.1:${server.port}`;
	const browser = await puppeteer.launch({
		headless: true,
		executablePath: chromePath,
		args: [
			"--no-sandbox",
			"--enable-unsafe-swiftshader",
			"--use-angle=swiftshader",
			"--hide-scrollbars",
			"--font-render-hinting=none",
			"--force-color-profile=srgb",
			"--disable-lcd-text",
			"--lang=en-US",
			"--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1",
		],
		env: { ...process.env, TZ: "UTC", LANG: "en_US.UTF-8" },
	});
	const version = await browser.version();

	const queue = cases.flatMap((visualCase) =>
		themes.map((theme) => ({ visualCase, theme })),
	);
	let next = 0;
	const records: CaseRecord[] = [];
	try {
		await Promise.all(
			Array.from({ length: Math.min(workerCount, queue.length) }, async () => {
				// A context per worker: parallel tabs never share storage.
				const worker: RenderWorker = {
					context: await browser.createBrowserContext(),
					origin,
				};
				while (next < queue.length) {
					const { visualCase, theme } = queue[next++];
					const record = await captureCase(
						worker,
						visualCase,
						theme,
						shotsDir,
						runDir,
					);
					records.push(record);
					const flags = [
						record.settled ? "" : "unsettled",
						record.attempts > 1 ? `${record.attempts} attempts` : "",
						record.renderError ? "render-error" : "",
						record.harnessError ? `harness-error: ${record.harnessError}` : "",
						record.pageErrors.length
							? `${record.pageErrors.length} page errors`
							: "",
					].filter(Boolean);
					console.log(
						`[${records.length}/${queue.length}] ${record.key} ${record.durationMs} ms${flags.length ? ` (${flags.join(", ")})` : ""}`,
					);
				}
				await worker.context.close();
			}),
		);
	} finally {
		await browser.close().catch(() => {});
		server.stop(true);
	}

	records.sort((left, right) => left.key.localeCompare(right.key));
	const manifest: RunManifest = {
		label: values.label ?? basename(runDir),
		createdAt: new Date().toISOString(),
		gitHead: gitHead(),
		versions: Object.fromEntries(
			REPORTED_PACKAGES.map((name) => [name, installedVersion(name)]),
		),
		browser: { path: chromePath, version },
		viewport: VIEWPORT,
		mode,
		bundle: { dir: bundleDir, bytes: bundleBytes },
		durationMs: Math.round(performance.now() - started),
		cases: records,
	};
	await Bun.write(
		join(runDir, "manifest.json"),
		`${JSON.stringify(manifest, null, "\t")}\n`,
	);

	const count = (predicate: (record: CaseRecord) => boolean) =>
		records.filter(predicate).length;
	console.log(
		[
			`${records.length} screenshots in ${Math.round(manifest.durationMs / 1000)} s -> ${shotsDir}`,
			`unsettled: ${count((record) => !record.settled)}`,
			`render errors: ${count((record) => record.renderError !== null)}`,
			`harness errors: ${count((record) => record.harnessError !== null)}`,
			`cases with page errors: ${count((record) => record.pageErrors.length > 0)}`,
			`cases with console errors: ${count((record) => record.consoleErrors.length > 0)}`,
		].join("\n"),
	);
	if (records.some((record) => record.harnessError)) process.exitCode = 1;
}

await main();
