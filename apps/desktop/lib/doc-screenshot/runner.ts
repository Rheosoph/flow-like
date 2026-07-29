import { createHash, randomBytes } from "node:crypto";
import { mkdir, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import puppeteer, {
	type Browser,
	type ElementHandle,
	type HTTPResponse,
	type KeyInput,
	type Page,
} from "puppeteer";
import sharp from "sharp";
import { outputFormatForCapture, safeCaptureOutputPath } from "./plan";
import {
	DOC_SCREENSHOT_RESULT_SCHEMA,
	type DocScreenshotArtifact,
	type DocScreenshotCaptureStep,
	type DocScreenshotFormat,
	type DocScreenshotPlan,
	type DocScreenshotQueryValue,
	type DocScreenshotResult,
	type DocScreenshotScenario,
	type DocScreenshotScenarioResult,
	type DocScreenshotStep,
	type DocScreenshotStepResult,
	type DocScreenshotTauriFixture,
	type DocScreenshotViewport,
} from "./types";

const LOCALE = "en-US";
const TIMEZONE = "UTC";
const SENSITIVE_QUERY_KEY =
	/(?:token|key|secret|password|passwd|auth|code|signature|credential)/i;

export interface RunDocScreenshotOptions {
	baseUrl: string;
	outputDir: string;
	tauriFixture?: DocScreenshotTauriFixture;
}

interface ScenarioDiagnostics {
	consoleErrors: number;
	pageErrors: number;
	requestFailures: number;
	warnings: number;
}

interface ScenarioRuntime {
	page: Page;
	baseUrl: URL;
	allowedOrigin: string;
	outputDir: string;
	defaults: DocScreenshotPlan["defaults"];
	viewport: DocScreenshotViewport;
	diagnostics: ScenarioDiagnostics;
	originViolation?: string;
	httpStatus?: number;
}

const delay = (ms: number): Promise<void> =>
	new Promise((resolveDelay) => setTimeout(resolveDelay, ms));

function errorMessage(error: unknown): string {
	const message = error instanceof Error ? error.message : String(error);
	return message.replace(
		/([?&](?:token|key|secret|password|passwd|auth|code|signature|credential)=)[^&\s]+/gi,
		"$1[REDACTED]",
	);
}

export function redactScreenshotUrl(value: string): string {
	try {
		const url = new URL(value);
		url.username = "";
		url.password = "";
		for (const key of [...url.searchParams.keys()]) {
			if (!SENSITIVE_QUERY_KEY.test(key)) continue;
			const values = url.searchParams.getAll(key);
			url.searchParams.delete(key);
			for (let index = 0; index < values.length; index += 1) {
				url.searchParams.append(key, "[REDACTED]");
			}
		}
		return url.toString();
	} catch {
		return value;
	}
}

export function buildScreenshotUrl(
	baseUrl: URL,
	path: string,
	query?: Record<string, DocScreenshotQueryValue>,
): URL {
	const url = new URL(path, baseUrl);
	if (url.origin !== baseUrl.origin) {
		throw new Error(`Navigation must stay on ${baseUrl.origin}.`);
	}
	if (query) {
		for (const [key, raw] of Object.entries(query)) {
			if (raw === undefined) continue;
			url.searchParams.delete(key);
			const values = Array.isArray(raw) ? raw : [raw];
			for (const value of values) {
				url.searchParams.append(key, value === null ? "" : String(value));
			}
		}
	}
	return url;
}

function assertAllowedPageUrl(value: string, origin: string): void {
	const url = new URL(value);
	if (url.protocol !== "http:" && url.protocol !== "https:") {
		throw new Error(
			`Top-level navigation used unsupported protocol ${url.protocol}.`,
		);
	}
	if (url.origin !== origin) {
		throw new Error(
			`Top-level navigation escaped the allowed origin: ${redactScreenshotUrl(value)}`,
		);
	}
}

async function injectTauriFixture(
	page: Page,
	fixture: DocScreenshotTauriFixture,
): Promise<void> {
	await page.evaluateOnNewDocument((fixtureValue) => {
		const callbacks = new Map<number, (...args: unknown[]) => unknown>();
		const eventListeners = new Map<string, number[]>();
		let callbackId = 0;
		const clone = <T>(value: T): T => {
			if (typeof structuredClone === "function") return structuredClone(value);
			return JSON.parse(JSON.stringify(value)) as T;
		};
		const transformCallback = (
			callback?: (...args: unknown[]) => unknown,
			once = false,
		): number => {
			callbackId += 1;
			const id = callbackId;
			if (callback) {
				callbacks.set(id, (...args: unknown[]) => {
					if (once) callbacks.delete(id);
					return callback(...args);
				});
			}
			return id;
		};
		const unregisterCallback = (id: number): void => {
			callbacks.delete(id);
		};
		const runCallback = (id: number, payload: unknown): void => {
			callbacks.get(id)?.(payload);
		};
		const invoke = async (
			command: string,
			args: Record<string, unknown> = {},
		): Promise<unknown> => {
			if (command === "plugin:event|listen") {
				const event = String(args.event ?? "");
				const handler = Number(args.handler);
				const handlers = eventListeners.get(event) ?? [];
				handlers.push(handler);
				eventListeners.set(event, handlers);
				return handler;
			}
			if (command === "plugin:event|unlisten") {
				const event = String(args.event ?? "");
				const id = Number(args.eventId ?? args.id);
				eventListeners.set(
					event,
					(eventListeners.get(event) ?? []).filter((item) => item !== id),
				);
				return null;
			}
			if (command === "plugin:event|emit") {
				const event = String(args.event ?? "");
				for (const id of eventListeners.get(event) ?? []) {
					runCallback(id, { event, payload: args.payload, id });
				}
				return null;
			}
			if (Object.hasOwn(fixtureValue.responses, command)) {
				const response = fixtureValue.responses[command];
				if (
					response &&
					typeof response === "object" &&
					!Array.isArray(response) &&
					"$error" in response
				) {
					throw new Error(String(response.$error));
				}
				return clone(response);
			}
			if (fixtureValue.strict) {
				throw new Error(
					`No screenshot fixture response for Tauri command: ${command}`,
				);
			}
			return null;
		};
		const internals = {
			invoke,
			transformCallback,
			unregisterCallback,
			runCallback,
			callbacks,
			convertFileSrc(filePath: string, protocol = "asset") {
				return `${protocol}://localhost/${encodeURIComponent(filePath)}`;
			},
			metadata: {
				currentWindow: { label: "main" },
				currentWebview: { windowLabel: "main", label: "main" },
			},
			plugins: {
				path: { sep: "/", delimiter: ":" },
			},
		};
		Object.defineProperty(window, "__TAURI_INTERNALS__", {
			configurable: true,
			value: internals,
		});
		Object.defineProperty(window, "__TAURI_EVENT_PLUGIN_INTERNALS__", {
			configurable: true,
			value: {
				unregisterListener(_event: string, id: number) {
					unregisterCallback(id);
				},
			},
		});
	}, fixture);
}

async function injectDeterministicPresentation(
	page: Page,
	scenario: DocScreenshotScenario,
	theme: "light" | "dark",
	disableAnimations: boolean,
	hideScrollbars: boolean,
): Promise<void> {
	await page.evaluateOnNewDocument(
		(settings) => {
			const applyStorage = (): void => {
				try {
					localStorage.clear();
					sessionStorage.clear();
					localStorage.setItem("theme", settings.theme);
					for (const [key, value] of Object.entries(settings.localStorage)) {
						localStorage.setItem(key, value);
					}
					for (const [key, value] of Object.entries(settings.sessionStorage)) {
						sessionStorage.setItem(key, value);
					}
				} catch {
					// Storage is unavailable on the initial about:blank document.
				}
			};
			const applyDocument = (): void => {
				document.documentElement.classList.remove("light", "dark");
				document.documentElement.classList.add(settings.theme);
				document.documentElement.style.colorScheme = settings.theme;
				if (document.getElementById("__doc_screenshot_determinism__")) return;
				const style = document.createElement("style");
				style.id = "__doc_screenshot_determinism__";
				style.textContent = [
					"nextjs-portal,#webpack-dev-server-client-overlay,[data-nextjs-toast]{display:none!important}",
					settings.disableAnimations
						? "*,*::before,*::after{animation-delay:0s!important;animation-duration:0s!important;animation-iteration-count:1!important;transition:none!important;caret-color:transparent!important;scroll-behavior:auto!important}"
						: "",
					settings.hideScrollbars
						? "html{scrollbar-width:none!important}::-webkit-scrollbar{display:none!important;width:0!important;height:0!important}"
						: "",
				].join("");
				(document.head ?? document.documentElement).appendChild(style);
			};
			applyStorage();
			applyDocument();
			document.addEventListener("DOMContentLoaded", applyDocument, {
				once: true,
			});
		},
		{
			theme,
			disableAnimations,
			hideScrollbars,
			localStorage: scenario.localStorage ?? {},
			sessionStorage: scenario.sessionStorage ?? {},
		},
	);
}

async function targetHandle(
	page: Page,
	selector: string,
	index: number,
	timeoutMs: number,
): Promise<ElementHandle<Element>> {
	await page.waitForSelector(selector, { visible: true, timeout: timeoutMs });
	const handles = await page.$$(selector);
	const handle = handles[index];
	if (!handle) {
		for (const item of handles) await item.dispose();
		throw new Error(
			`Selector "${selector}" matched ${handles.length} element(s); index ${index} is unavailable.`,
		);
	}
	for (const [itemIndex, item] of handles.entries()) {
		if (itemIndex !== index) await item.dispose();
	}
	return handle;
}

function resolveSecretValue(step: {
	value?: string;
	valueEnv?: string;
}): string {
	if (step.value !== undefined) return step.value;
	const envName = step.valueEnv;
	if (!envName) throw new Error("Input step has no value.");
	const value = process.env[envName];
	if (value === undefined) {
		throw new Error(`Required environment variable is not set: ${envName}`);
	}
	return value;
}

async function waitForReadyAssets(
	page: Page,
	timeoutMs: number,
): Promise<number> {
	return page.evaluate(
		async (assetTimeoutMs) => {
			if ("fonts" in document) {
				await Promise.race([
					document.fonts.ready,
					new Promise<void>((resolveWait) =>
						setTimeout(resolveWait, assetTimeoutMs),
					),
				]);
			}
			const images = [...document.images];
			for (const image of images) image.loading = "eager";
			const results = await Promise.all(
				images.map(
					(image) =>
						new Promise<boolean>((resolveImage) => {
							if (image.complete) {
								resolveImage(image.naturalWidth > 0);
								return;
							}
							const timer = setTimeout(
								() => resolveImage(false),
								assetTimeoutMs,
							);
							const finish = (loaded: boolean) => {
								clearTimeout(timer);
								resolveImage(loaded);
							};
							image.addEventListener("load", () => finish(true), {
								once: true,
							});
							image.addEventListener("error", () => finish(false), {
								once: true,
							});
						}),
				),
			);
			await Promise.all(
				images.map((image) => image.decode?.().catch(() => undefined)),
			);
			return results.filter((loaded) => !loaded).length;
		},
		Math.min(timeoutMs, 10_000),
	);
}

async function settlePage(runtime: ScenarioRuntime): Promise<void> {
	const failedImages = await waitForReadyAssets(
		runtime.page,
		runtime.defaults.timeoutMs,
	);
	if (failedImages > 0) runtime.diagnostics.warnings += failedImages;
	try {
		await runtime.page.waitForNetworkIdle({
			idleTime: 300,
			timeout: 1_500,
			concurrency: 2,
		});
	} catch {
		runtime.diagnostics.warnings += 1;
	}
	if (runtime.defaults.settleMs > 0) await delay(runtime.defaults.settleMs);
	await runtime.page.evaluate(
		() =>
			new Promise<void>((resolveFrames) =>
				requestAnimationFrame(() =>
					requestAnimationFrame(() => resolveFrames()),
				),
			),
	);
}

async function removeDevelopmentOverlays(page: Page): Promise<void> {
	await page.evaluate(() => {
		for (const selector of [
			"nextjs-portal",
			"#webpack-dev-server-client-overlay",
			"[data-nextjs-toast]",
		]) {
			for (const element of document.querySelectorAll(selector))
				element.remove();
		}
	});
}

async function encodeScreenshot(
	png: Buffer,
	format: DocScreenshotFormat,
	quality?: number,
): Promise<{ buffer: Buffer; mimeType: string }> {
	if (format === "png") {
		return { buffer: png, mimeType: "image/png" };
	}
	if (format === "webp") {
		return {
			buffer: await sharp(png).webp({ lossless: true, effort: 6 }).toBuffer(),
			mimeType: "image/webp",
		};
	}
	return {
		buffer: await sharp(png)
			.flatten({ background: "#ffffff" })
			.jpeg({
				quality: quality ?? 95,
				chromaSubsampling: "4:4:4",
				mozjpeg: true,
			})
			.toBuffer(),
		mimeType: "image/jpeg",
	};
}

async function captureScreenshot(
	runtime: ScenarioRuntime,
	step: DocScreenshotCaptureStep,
): Promise<DocScreenshotArtifact> {
	await settlePage(runtime);
	await removeDevelopmentOverlays(runtime.page);
	if (runtime.originViolation) throw new Error(runtime.originViolation);
	assertAllowedPageUrl(runtime.page.url(), runtime.allowedOrigin);
	const mode = step.mode ?? "viewport";
	let cssWidth = runtime.viewport.width;
	let cssHeight = runtime.viewport.height;
	let clip: { x: number; y: number; width: number; height: number } | undefined;
	if (mode === "fullPage") {
		const dimensions = await runtime.page.evaluate(() => ({
			width: Math.max(
				document.documentElement.scrollWidth,
				document.body?.scrollWidth ?? 0,
			),
			height: Math.max(
				document.documentElement.scrollHeight,
				document.body?.scrollHeight ?? 0,
			),
		}));
		cssWidth = dimensions.width;
		cssHeight = dimensions.height;
	} else if (mode === "element") {
		const handle = await targetHandle(
			runtime.page,
			step.selector ?? "",
			step.index ?? 0,
			runtime.defaults.timeoutMs,
		);
		try {
			const box = await handle.boundingBox();
			if (!box || box.width <= 0 || box.height <= 0) {
				throw new Error(`Element has no visible bounds: ${step.selector}`);
			}
			const padding = step.padding ?? 0;
			clip = {
				x: Math.max(0, box.x - padding),
				y: Math.max(0, box.y - padding),
				width: box.width + padding * 2,
				height: box.height + padding * 2,
			};
			cssWidth = clip.width;
			cssHeight = clip.height;
		} finally {
			await handle.dispose();
		}
	}
	let maskStyle: ElementHandle<HTMLStyleElement> | undefined;
	if (step.hideSelectors && step.hideSelectors.length > 0) {
		maskStyle = await runtime.page.addStyleTag({
			content: `${step.hideSelectors.join(",")}{visibility:hidden!important}`,
		});
	}
	try {
		const png = Buffer.from(
			await runtime.page.screenshot({
				type: "png",
				fullPage: mode === "fullPage",
				clip,
				captureBeyondViewport: true,
				omitBackground: false,
			}),
		);
		const format = outputFormatForCapture(step, runtime.defaults.format);
		const encoded = await encodeScreenshot(
			png,
			format,
			step.quality ?? runtime.defaults.quality,
		);
		const outputPath = safeCaptureOutputPath(runtime.outputDir, step, format);
		await mkdir(dirname(outputPath), { recursive: true });
		await writeFile(outputPath, encoded.buffer);
		const metadata = await sharp(encoded.buffer).metadata();
		const pixelWidth =
			metadata.width ??
			Math.round(cssWidth * runtime.viewport.deviceScaleFactor);
		const pixelHeight =
			metadata.height ??
			Math.round(cssHeight * runtime.viewport.deviceScaleFactor);
		return {
			id: step.name,
			path: outputPath,
			mimeType: encoded.mimeType,
			bytes: encoded.buffer.byteLength,
			sha256: createHash("sha256").update(encoded.buffer).digest("hex"),
			mode,
			selector: step.selector,
			capturedAt: new Date().toISOString(),
			css: {
				width: Number(cssWidth.toFixed(2)),
				height: Number(cssHeight.toFixed(2)),
			},
			pixels: {
				width: pixelWidth,
				height: pixelHeight,
			},
			effectiveScale: {
				x: Number((pixelWidth / cssWidth).toFixed(4)),
				y: Number((pixelHeight / cssHeight).toFixed(4)),
			},
		};
	} finally {
		if (maskStyle) {
			await maskStyle
				.evaluate((element) => element.remove())
				.catch(() => undefined);
			await maskStyle.dispose();
		}
	}
}

async function runStep(
	runtime: ScenarioRuntime,
	step: DocScreenshotStep,
): Promise<DocScreenshotArtifact | undefined> {
	const timeoutMs =
		step.type === "waitFor" && step.timeoutMs
			? step.timeoutMs
			: runtime.defaults.timeoutMs;
	switch (step.type) {
		case "goto": {
			const url = buildScreenshotUrl(runtime.baseUrl, step.path, step.query);
			const response = await runtime.page.goto(url.toString(), {
				waitUntil: "domcontentloaded",
				timeout: timeoutMs,
			});
			runtime.httpStatus = response?.status();
			assertAllowedPageUrl(runtime.page.url(), runtime.allowedOrigin);
			return;
		}
		case "click": {
			const handle = await targetHandle(
				runtime.page,
				step.selector,
				step.index ?? 0,
				timeoutMs,
			);
			try {
				await handle.click({
					button: step.button,
					clickCount: step.clickCount,
				});
			} finally {
				await handle.dispose();
			}
			break;
		}
		case "fill": {
			const handle = await targetHandle(
				runtime.page,
				step.selector,
				step.index ?? 0,
				timeoutMs,
			);
			try {
				await handle.focus();
				await runtime.page.keyboard.down("Control");
				await runtime.page.keyboard.press("A");
				await runtime.page.keyboard.up("Control");
				await runtime.page.keyboard.press("Backspace");
				const value = resolveSecretValue(step);
				if (value) await handle.type(value);
			} finally {
				await handle.dispose();
			}
			break;
		}
		case "type": {
			const handle = await targetHandle(
				runtime.page,
				step.selector,
				step.index ?? 0,
				timeoutMs,
			);
			try {
				await handle.type(resolveSecretValue(step), {
					delay: step.delayMs,
				});
			} finally {
				await handle.dispose();
			}
			break;
		}
		case "press":
			if (step.selector) {
				const handle = await targetHandle(
					runtime.page,
					step.selector,
					step.index ?? 0,
					timeoutMs,
				);
				try {
					await handle.focus();
				} finally {
					await handle.dispose();
				}
			}
			await runtime.page.keyboard.press(step.key as KeyInput);
			break;
		case "select": {
			const handle = await targetHandle(
				runtime.page,
				step.selector,
				step.index ?? 0,
				timeoutMs,
			);
			try {
				await handle.select(...step.values);
			} finally {
				await handle.dispose();
			}
			break;
		}
		case "check": {
			const handle = await targetHandle(
				runtime.page,
				step.selector,
				step.index ?? 0,
				timeoutMs,
			);
			try {
				const checked = await handle.evaluate((element) => {
					if (
						!(element instanceof HTMLInputElement) ||
						(element.type !== "checkbox" && element.type !== "radio")
					) {
						throw new Error("check target must be a checkbox or radio input.");
					}
					return element.checked;
				});
				if (checked !== (step.checked ?? true)) await handle.click();
			} finally {
				await handle.dispose();
			}
			break;
		}
		case "hover": {
			const handle = await targetHandle(
				runtime.page,
				step.selector,
				step.index ?? 0,
				timeoutMs,
			);
			try {
				await handle.hover();
			} finally {
				await handle.dispose();
			}
			break;
		}
		case "scroll":
			if (step.selector) {
				const handle = await targetHandle(
					runtime.page,
					step.selector,
					step.index ?? 0,
					timeoutMs,
				);
				try {
					await handle.evaluate(
						(element, offset) => {
							element.scrollIntoView({ block: "center", inline: "center" });
							if (offset.x || offset.y) {
								element.scrollBy(offset.x, offset.y);
							}
						},
						{ x: step.x ?? 0, y: step.y ?? 0 },
					);
				} finally {
					await handle.dispose();
				}
			} else {
				await runtime.page.evaluate(
					(offset) => window.scrollBy(offset.x, offset.y),
					{ x: step.x ?? 0, y: step.y ?? 0 },
				);
			}
			break;
		case "waitFor":
			if (step.selector) {
				await runtime.page.waitForSelector(step.selector, {
					timeout: timeoutMs,
					visible: step.state === "visible",
					hidden: step.state === "hidden" || step.state === "detached",
				});
			} else if (step.urlIncludes) {
				await runtime.page.waitForFunction(
					(fragment) => location.href.includes(fragment),
					{ timeout: timeoutMs },
					step.urlIncludes,
				);
			} else if (step.text) {
				await runtime.page.waitForFunction(
					(expected) => document.body?.innerText.includes(expected),
					{ timeout: timeoutMs },
					step.text,
				);
			}
			break;
		case "delay":
			await delay(step.ms);
			break;
		case "capture":
			return captureScreenshot(runtime, step);
	}
	if (runtime.originViolation) throw new Error(runtime.originViolation);
	if (runtime.page.url() !== "about:blank") {
		assertAllowedPageUrl(runtime.page.url(), runtime.allowedOrigin);
	}
}

async function runScenario(
	browser: Browser,
	plan: DocScreenshotPlan,
	scenario: DocScreenshotScenario,
	options: RunDocScreenshotOptions,
): Promise<DocScreenshotScenarioResult> {
	const started = Date.now();
	const baseUrl = new URL(options.baseUrl);
	const viewport: DocScreenshotViewport = {
		...plan.defaults.viewport,
		...scenario.viewport,
	};
	const theme = scenario.theme ?? plan.defaults.theme;
	const requestedUrl = buildScreenshotUrl(
		baseUrl,
		scenario.path,
		scenario.query,
	);
	const diagnostics: ScenarioDiagnostics = {
		consoleErrors: 0,
		pageErrors: 0,
		requestFailures: 0,
		warnings: 0,
	};
	const stepResults: DocScreenshotStepResult[] = [];
	const artifacts: DocScreenshotArtifact[] = [];
	let title = "";
	let finalUrl = requestedUrl.toString();
	let scenarioError: string | undefined;
	const context = await browser.createBrowserContext();
	const page = await context.newPage();
	const runtime: ScenarioRuntime = {
		page,
		baseUrl,
		allowedOrigin: baseUrl.origin,
		outputDir: options.outputDir,
		defaults: plan.defaults,
		viewport,
		diagnostics,
	};
	try {
		await page.setViewport(viewport);
		await page.emulateTimezone(TIMEZONE);
		await page.emulateMediaFeatures([
			{ name: "prefers-color-scheme", value: theme },
			{ name: "prefers-reduced-motion", value: "reduce" },
		]);
		await page.setExtraHTTPHeaders({ "Accept-Language": LOCALE });
		await injectDeterministicPresentation(
			page,
			scenario,
			theme,
			plan.defaults.disableAnimations,
			plan.defaults.hideScrollbars,
		);
		if (options.tauriFixture) {
			await injectTauriFixture(page, options.tauriFixture);
		}
		page.on("console", (message) => {
			if (message.type() === "error") diagnostics.consoleErrors += 1;
		});
		page.on("pageerror", () => {
			diagnostics.pageErrors += 1;
		});
		page.on("requestfailed", () => {
			diagnostics.requestFailures += 1;
		});
		page.on("popup", (popup) => {
			runtime.originViolation =
				"Popups are disabled in documentation captures.";
			if (popup) void popup.close();
		});
		page.on("framenavigated", (frame) => {
			if (frame !== page.mainFrame() || frame.url() === "about:blank") return;
			try {
				assertAllowedPageUrl(frame.url(), baseUrl.origin);
			} catch (error) {
				runtime.originViolation = errorMessage(error);
			}
		});
		const response: HTTPResponse | null = await page.goto(
			requestedUrl.toString(),
			{
				waitUntil: "domcontentloaded",
				timeout: plan.defaults.timeoutMs,
			},
		);
		runtime.httpStatus = response?.status();
		assertAllowedPageUrl(page.url(), baseUrl.origin);
		for (const [index, step] of scenario.steps.entries()) {
			const stepStarted = Date.now();
			try {
				const artifact = await runStep(runtime, step);
				if (artifact) artifacts.push(artifact);
				stepResults.push({
					index,
					type: step.type,
					status: "passed",
					durationMs: Date.now() - stepStarted,
					urlAfter: redactScreenshotUrl(page.url()),
				});
			} catch (error) {
				const message = errorMessage(error);
				stepResults.push({
					index,
					type: step.type,
					status: "failed",
					durationMs: Date.now() - stepStarted,
					urlAfter: redactScreenshotUrl(page.url()),
					error: message,
				});
				scenarioError = message;
				break;
			}
		}
		title = await page.title().catch(() => "");
		finalUrl = page.url();
	} catch (error) {
		scenarioError = errorMessage(error);
		title = await page.title().catch(() => "");
		finalUrl = page.url();
	} finally {
		await context.close();
	}
	return {
		name: scenario.name,
		passed: !scenarioError && artifacts.length > 0,
		requestedUrl: redactScreenshotUrl(requestedUrl.toString()),
		finalUrl: redactScreenshotUrl(finalUrl),
		title,
		httpStatus: runtime.httpStatus,
		durationMs: Date.now() - started,
		render: {
			viewport,
			theme,
			locale: LOCALE,
			timezone: TIMEZONE,
			reducedMotion: true,
			colorProfile: "srgb",
			disableAnimations: plan.defaults.disableAnimations,
			hideScrollbars: plan.defaults.hideScrollbars,
			settleMs: plan.defaults.settleMs,
		},
		steps: stepResults,
		artifacts,
		diagnostics,
		error: scenarioError,
	};
}

export async function runDocScreenshotPlan(
	plan: DocScreenshotPlan,
	options: RunDocScreenshotOptions,
): Promise<DocScreenshotResult> {
	const startedAt = new Date();
	const runId = `docs_${Date.now()}_${randomBytes(6).toString("hex")}`;
	const browser = await puppeteer.launch({
		headless: true,
		defaultViewport: null,
		args: [
			"--force-color-profile=srgb",
			"--disable-background-timer-throttling",
			"--disable-renderer-backgrounding",
			"--no-sandbox",
			"--disable-setuid-sandbox",
		],
	});
	const scenarios: DocScreenshotScenarioResult[] = [];
	let version = "";
	try {
		version = await browser.version();
		for (const scenario of plan.scenarios) {
			scenarios.push(await runScenario(browser, plan, scenario, options));
		}
	} finally {
		await browser.close();
	}
	const finishedAt = new Date();
	const screenshots = scenarios.reduce(
		(total, scenario) => total + scenario.artifacts.length,
		0,
	);
	const scenariosPassed = scenarios.filter(
		(scenario) => scenario.passed,
	).length;
	return {
		schema: DOC_SCREENSHOT_RESULT_SCHEMA,
		runId,
		passed: scenariosPassed === scenarios.length && screenshots > 0,
		startedAt: startedAt.toISOString(),
		finishedAt: finishedAt.toISOString(),
		durationMs: finishedAt.getTime() - startedAt.getTime(),
		baseUrl: redactScreenshotUrl(options.baseUrl),
		browser: {
			product: "Chromium",
			version,
			headless: true,
		},
		scenarios,
		summary: {
			scenarios: scenarios.length,
			scenariosPassed,
			screenshots,
		},
	};
}

export function resolveOutputDir(path: string): string {
	return resolve(path);
}
