import { createHash, randomBytes } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, resolve, sep } from "node:path";
import puppeteer, {
	type BrowserContext,
	type ElementHandle,
	type HTTPRequest,
	type HTTPResponse,
	type KeyInput,
	type Page,
} from "puppeteer";
import sharp from "sharp";
import {
	outputFormatForCapture,
	safeCaptureOutputPath,
	screenshotScenarioFingerprint,
} from "./plan";
import {
	DOC_SCREENSHOT_RESULT_SCHEMA,
	type DocScreenshotArtifact,
	type DocScreenshotDiagnostic,
	type DocScreenshotDiagnosticAllowance,
	type DocScreenshotCaptureStep,
	type DocScreenshotFormat,
	type DocScreenshotHttpFixture,
	type DocScreenshotHttpFixtureResponse,
	type DocScreenshotKeyboardModifier,
	type DocScreenshotMouseButton,
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

type StaticAsset = { body: Uint8Array; headers: Record<string, string> };
const LOCALE = "en-US";
const TIMEZONE = "UTC";
const SENSITIVE_QUERY_KEY =
	/(?:token|key|secret|password|passwd|auth|code|signature|credential)/i;

export interface RunDocScreenshotOptions {
	baseUrl: string;
	outputDir: string;
	tauriFixture?: DocScreenshotTauriFixture;
	httpFixture?: DocScreenshotHttpFixture;
	provenance?: DocScreenshotResult["provenance"];
}

interface ScenarioDiagnostics {
	consoleErrors: number;
	pageErrors: number;
	requestFailures: number;
	warnings: number;
	entries: DocScreenshotDiagnostic[];
	unexpected: number;
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
	httpFixtureViolation?: string;
	httpStatus?: number;
	heldModifiers: Set<DocScreenshotKeyboardModifier>;
	heldMouseButton?: DocScreenshotMouseButton;
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

export function classifyScreenshotDiagnostic(
	entry: DocScreenshotDiagnostic,
	previous: DocScreenshotDiagnostic[],
	allowlist: DocScreenshotDiagnosticAllowance[],
): DocScreenshotDiagnostic {
	const allowance = allowlist.find(
		(candidate) =>
			candidate.kind === entry.kind && candidate.message === entry.message,
	);
	const count = previous.filter(
		(candidate) =>
			candidate.kind === entry.kind && candidate.message === entry.message,
	).length;
	return allowance && count < allowance.maxCount
		? { ...entry, allowance: allowance.reason }
		: entry;
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

export type DocScreenshotHttpFixtureResolution =
	| {
			action: "respond";
			response: DocScreenshotHttpFixtureResponse;
	  }
	| {
			action: "continue";
	  }
	| {
			action: "block";
	  }
	| {
			action: "abort";
			error: string;
	  };

export function resolveHttpFixtureRequest(
	fixture: DocScreenshotHttpFixture,
	allowedOrigin: string,
	request: {
		method: string;
		url: string;
		body?: string;
	},
): DocScreenshotHttpFixtureResolution {
	const route = fixture.routes.find(
		(candidate) =>
			candidate.request.method === request.method &&
			candidate.request.url === request.url &&
			(candidate.request.body === undefined ||
				candidate.request.body === request.body),
	);
	if (route) {
		return {
			action: "respond",
			response: route.response,
		};
	}
	const url = new URL(request.url);
	if (
		(url.protocol === "http:" || url.protocol === "https:") &&
		(fixture.blockedOrigins.includes(url.origin) ||
			fixture.blockedEndpoints.includes(`${url.origin}${url.pathname}`))
	) {
		return { action: "block" };
	}
	if (
		(url.protocol !== "http:" && url.protocol !== "https:") ||
		url.origin === allowedOrigin ||
		!fixture.strict
	) {
		return { action: "continue" };
	}
	return {
		action: "abort",
		error: `Unexpected cross-origin request not declared in the HTTP fixture: ${request.method} ${redactScreenshotUrl(request.url)}`,
	};
}

function httpFixtureResponseHeaders(
	response: DocScreenshotHttpFixtureResponse,
): Record<string, string> {
	const headers = { ...response.headers };
	if (
		response.json !== undefined &&
		!Object.keys(headers).some((name) => name.toLowerCase() === "content-type")
	) {
		headers["content-type"] = "application/json; charset=utf-8";
	}
	return headers;
}

async function installHttpFixture(
	page: Page,
	fixture: DocScreenshotHttpFixture,
	allowedOrigin: string,
	onViolation: (message: string) => void,
	intentionalBlocks: WeakSet<HTTPRequest>,
): Promise<void> {
	await page.setRequestInterception(true);
	page.on("request", async (request: HTTPRequest) => {
		if (request.isInterceptResolutionHandled()) return;
		try {
			const resolution = resolveHttpFixtureRequest(fixture, allowedOrigin, {
				method: request.method(),
				url: request.url(),
				body: request.hasPostData() ? await request.fetchPostData() : undefined,
			});
			switch (resolution.action) {
				case "respond":
					await request.respond({
						status: resolution.response.status,
						headers: httpFixtureResponseHeaders(resolution.response),
						body:
							resolution.response.json === undefined
								? resolution.response.body
								: JSON.stringify(resolution.response.json),
					});
					return;
				case "continue":
					await request.continue();
					return;
				case "block":
					intentionalBlocks.add(request);
					await request.abort("blockedbyclient");
					return;
				case "abort":
					onViolation(resolution.error);
					await request.abort("blockedbyclient");
					return;
			}
		} catch (error) {
			onViolation(
				`HTTP fixture interception failed for ${request.method()} ${redactScreenshotUrl(request.url())}: ${errorMessage(error)}`,
			);
			if (!request.isInterceptResolutionHandled()) {
				await request.abort("failed").catch(() => undefined);
			}
		}
	});
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

function assertScenarioIntegrity(runtime: ScenarioRuntime): void {
	if (runtime.originViolation) throw new Error(runtime.originViolation);
	if (runtime.httpFixtureViolation) {
		throw new Error(runtime.httpFixtureViolation);
	}
	if (runtime.page.url() !== "about:blank") {
		assertAllowedPageUrl(runtime.page.url(), runtime.allowedOrigin);
	}
}

async function injectTauriFixture(
	page: Page,
	fixture: DocScreenshotTauriFixture,
): Promise<void> {
	// Browser fixtures need the same SQL contract as the desktop IndexedDB adapter.
	// Each scenario owns disposable in-memory databases, never user data.
	const { Database } = await import("bun:sqlite");
	const databases = new Map<string, InstanceType<typeof Database>>();
	const connections = new Map<number, InstanceType<typeof Database>>();
	let connectionId = 0;
	await page.exposeFunction(
		"__docScreenshotSql",
		(command: string, args: Record<string, any>) => {
			if (command.endsWith("sql_open")) {
				const name = String(args.name);
				let database = databases.get(name);
				if (!database) {
					database = new Database(":memory:");
					databases.set(name, database);
				}
				connections.set(++connectionId, database);
				return connectionId;
			}
			if (command.endsWith("sql_close")) {
				connections.delete(args.connId);
				return null;
			}
			const database = connections.get(args.connId);
			if (!database) throw new Error("Unknown screenshot SQL connection");
			return args.queries.map((query: { sql: string; args: any[] }) => {
				try {
					const statement = database.query(query.sql);
					if (statement.columnNames.length > 0)
						return { rows: statement.all(...query.args), rows_affected: 0 };
					const result = statement.run(...query.args);
					return {
						rows: [],
						rows_affected: result.changes,
						insert_id: Number(result.lastInsertRowid),
					};
				} catch (error) {
					return { rows: [], rows_affected: 0, error: String(error) };
				}
			});
		},
	);
	page.once("close", () => {
		for (const database of databases.values()) database.close();
	});
	await page.evaluateOnNewDocument((fixtureValue) => {
		// A new fixture profile has no legacy browser databases to migrate.
		localStorage.setItem("__fl_idb_sqlite_migrated__", "1");
		const httpRequests = new Map<
			number,
			{ url: string; body: unknown; sent: boolean }
		>();
		let httpId = 100;
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
			if (command === "plugin:http|fetch") {
				const config = args.clientConfig as { url: string };
				const url = new URL(config.url);
				const declared = fixtureValue.responses.$http as
					| Record<string, unknown>
					| undefined;
				let body: unknown = declared?.[url.pathname];
				if (body === undefined && url.pathname.endsWith("/info/features"))
					body = {};
				if (body === undefined && url.pathname.endsWith("/auth/openid"))
					body = null;
				if (body === undefined) {
					const bytes = fixtureValue.responses["plugin:http|fetch_read_body"];
					try {
						body = Array.isArray(bytes)
							? JSON.parse(
									new TextDecoder().decode(
										new Uint8Array(bytes.slice(0, -1) as number[]),
									),
								)
							: null;
					} catch {
						body = null;
					}
				}
				httpRequests.set(++httpId, { url: config.url, body, sent: false });
				return httpId;
			}
			if (command === "plugin:http|fetch_send") {
				const request = httpRequests.get(Number(args.rid));
				return {
					status: 200,
					statusText: "OK",
					url: request?.url,
					headers: [["content-type", "application/json"]],
					rid: args.rid,
				};
			}
			if (command === "plugin:http|fetch_read_body") {
				const request = httpRequests.get(Number(args.rid));
				if (!request || request.sent) return [1];
				request.sent = true;
				return [...new TextEncoder().encode(JSON.stringify(request.body)), 0];
			}
			if (command.startsWith("plugin:flow-like-dexie-blob-offload|sql_")) {
				return (window as any).__docScreenshotSql(command, args);
			}
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
				if (
					response &&
					typeof response === "object" &&
					!Array.isArray(response) &&
					"$events" in response
				) {
					const descriptor = response as {
						$value?: unknown;
						$delayMs?: unknown;
						$events?: unknown;
					};
					const events = Array.isArray(descriptor.$events)
						? descriptor.$events.slice(0, 100)
						: [];
					const eventTasks = events.map(async (eventValue) => {
						if (
							!eventValue ||
							typeof eventValue !== "object" ||
							Array.isArray(eventValue)
						) {
							return;
						}
						const event = eventValue as Record<string, unknown>;
						const afterMs =
							typeof event.afterMs === "number" &&
							Number.isFinite(event.afterMs)
								? Math.min(120_000, Math.max(0, event.afterMs))
								: 0;
						await new Promise((resolveEvent) =>
							setTimeout(resolveEvent, afterMs),
						);
						const name = String(event.name ?? "");
						if (!name) return;
						for (const id of eventListeners.get(name) ?? []) {
							runCallback(id, {
								event: name,
								payload: clone(event.payload),
								id,
							});
						}
					});
					const delayMs =
						typeof descriptor.$delayMs === "number" &&
						Number.isFinite(descriptor.$delayMs)
							? Math.min(120_000, Math.max(0, descriptor.$delayMs))
							: 0;
					await Promise.all([
						...eventTasks,
						new Promise((resolveDelay) => setTimeout(resolveDelay, delayMs)),
					]);
					return clone(descriptor.$value ?? null);
				}
				if (
					response &&
					typeof response === "object" &&
					!Array.isArray(response) &&
					"$argument" in response
				) {
					const path = response.$argument;
					if (
						typeof path !== "string" ||
						!/^[a-zA-Z_][a-zA-Z0-9_]*(\.[a-zA-Z_][a-zA-Z0-9_]*)*$/.test(path)
					)
						throw new Error("Invalid screenshot fixture argument path");
					let value: unknown = args;
					for (const key of path.split(".")) {
						value =
							value && typeof value === "object" && Object.hasOwn(value, key)
								? (value as Record<string, unknown>)[key]
								: undefined;
					}
					if (value === undefined)
						throw new Error(`Missing screenshot fixture argument: ${path}`);
					return clone(value);
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
			// Keep the browser factory available when a desktop storage adapter
			// replaces the global after some client databases have already opened.
			(window as any).__docScreenshotBrowserIndexedDB = window.indexedDB;
			// A capture must stay on one compiled page. Next's development refresh
			// can otherwise race first hydration while another route is compiling.
			const NativeWebSocket = window.WebSocket;
			window.WebSocket = new Proxy(NativeWebSocket, {
				construct(Target, args) {
					const socket = Reflect.construct(Target, args) as WebSocket;
					if (/\/_next\/(?:webpack-hmr|hmr)(?:\?|$)/.test(String(args[0]))) {
						socket.addEventListener("message", (event) => {
							if (typeof event.data !== "string") return;
							try {
								const message = JSON.parse(event.data);
								if (
									[
										"serverComponentChanges",
										"reloadPage",
										"staticParamsChanged",
										"addedPage",
										"removedPage",
									].includes(message.type)
								)
									event.stopImmediatePropagation();
							} catch {
								/* Binary module messages are handled by Next. */
							}
						});
					}
					return socket;
				},
			});
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
				const root = document.documentElement;
				if (!root) return;
				root.classList.remove("light", "dark");
				root.classList.add(settings.theme);
				root.style.colorScheme = settings.theme;
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
				(document.head ?? root).appendChild(style);
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
	await page.waitForFunction(
		(target) => {
			const element = document.querySelectorAll(target.selector)[target.index];
			if (!element) return false;
			const style = getComputedStyle(element);
			const bounds = element.getBoundingClientRect();
			return (
				style.display !== "none" &&
				style.visibility !== "hidden" &&
				bounds.width > 0 &&
				bounds.height > 0
			);
		},
		{ timeout: timeoutMs },
		{ selector, index },
	);
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

async function isDragCenterHitTestable(
	handle: ElementHandle<Element>,
): Promise<boolean> {
	return handle.evaluate((element) => {
		const bounds = element.getBoundingClientRect();
		const x = bounds.x + bounds.width / 2;
		const y = bounds.y + bounds.height / 2;
		if (x < 0 || y < 0 || x >= window.innerWidth || y >= window.innerHeight) {
			return false;
		}
		const hit = document.elementFromPoint(x, y);
		return Boolean(
			hit &&
				(hit === element || element.contains(hit) || hit.contains(element)),
		);
	});
}

async function withKeyboardModifiers<T>(
	runtime: ScenarioRuntime,
	modifiers: DocScreenshotKeyboardModifier[],
	action: () => Promise<T>,
): Promise<T> {
	const pressed: DocScreenshotKeyboardModifier[] = [];
	let actionFailed = false;
	let actionError: unknown;
	let result: T | undefined;
	try {
		for (const modifier of modifiers) {
			pressed.push(modifier);
			runtime.heldModifiers.add(modifier);
			await runtime.page.keyboard.down(modifier as KeyInput);
		}
		result = await action();
	} catch (error) {
		actionFailed = true;
		actionError = error;
	}
	let cleanupError: unknown;
	for (const modifier of pressed.reverse()) {
		try {
			await runtime.page.keyboard.up(modifier as KeyInput);
			runtime.heldModifiers.delete(modifier);
		} catch (error) {
			cleanupError ??= error;
		}
	}
	if (actionFailed) throw actionError;
	if (cleanupError) throw cleanupError;
	return result as T;
}

async function releaseHeldMouse(runtime: ScenarioRuntime): Promise<void> {
	const button = runtime.heldMouseButton;
	if (!button) return;
	await runtime.page.mouse.up({ button });
	runtime.heldMouseButton = undefined;
}

async function releaseHeldInput(runtime: ScenarioRuntime): Promise<void> {
	for (const modifier of [...runtime.heldModifiers].reverse()) {
		try {
			await runtime.page.keyboard.up(modifier as KeyInput);
		} catch {
			// Closing the browser context is the final fallback for input cleanup.
		} finally {
			runtime.heldModifiers.delete(modifier);
		}
	}
	await releaseHeldMouse(runtime).catch(() => undefined);
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

export interface DocScreenshotElementCaptureTarget {
	scrollIntoView(): Promise<void>;
	boundingBox(): Promise<{
		x: number;
		y: number;
		width: number;
		height: number;
	} | null>;
}

export async function resolveElementCaptureClip(
	target: DocScreenshotElementCaptureTarget,
	selector: string,
	padding: number,
): Promise<{ x: number; y: number; width: number; height: number }> {
	await target.scrollIntoView();
	const box = await target.boundingBox();
	if (!box || box.width <= 0 || box.height <= 0) {
		throw new Error(`Element has no visible bounds: ${selector}`);
	}
	return {
		x: Math.max(0, box.x - padding),
		y: Math.max(0, box.y - padding),
		width: box.width + padding * 2,
		height: box.height + padding * 2,
	};
}

async function captureScreenshot(
	runtime: ScenarioRuntime,
	step: DocScreenshotCaptureStep,
): Promise<DocScreenshotArtifact> {
	await settlePage(runtime);
	await removeDevelopmentOverlays(runtime.page);
	assertScenarioIntegrity(runtime);
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
			const padding = step.padding ?? 0;
			clip = await resolveElementCaptureClip(
				handle,
				step.selector ?? "",
				padding,
			);
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
			url: redactScreenshotUrl(runtime.page.url()),
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
	assertScenarioIntegrity(runtime);
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
			assertScenarioIntegrity(runtime);
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
				await withKeyboardModifiers(runtime, step.modifiers ?? [], () =>
					handle.click({
						button: step.button,
						clickCount: step.clickCount,
					}),
				);
			} finally {
				await handle.dispose();
			}
			break;
		}
		case "drag": {
			await releaseHeldMouse(runtime);
			const sourceHandle = await targetHandle(
				runtime.page,
				step.selector,
				step.index ?? 0,
				timeoutMs,
			);
			let target: ElementHandle<Element> | undefined;
			try {
				target = await targetHandle(
					runtime.page,
					step.targetSelector,
					step.targetIndex ?? 0,
					timeoutMs,
				);
				const [sourceBox, targetBox] = await Promise.all([
					sourceHandle.boundingBox(),
					target.boundingBox(),
				]);
				if (!sourceBox || sourceBox.width <= 0 || sourceBox.height <= 0) {
					throw new Error(
						`Drag source has no visible bounds: ${step.selector}`,
					);
				}
				if (!targetBox || targetBox.width <= 0 || targetBox.height <= 0) {
					throw new Error(
						`Drag target has no visible bounds: ${step.targetSelector}`,
					);
				}
				if (!(await isDragCenterHitTestable(sourceHandle))) {
					throw new Error(
						`Drag source center is outside the viewport or obscured: ${step.selector}`,
					);
				}
				if (!(await isDragCenterHitTestable(target))) {
					throw new Error(
						`Drag target center is outside the viewport or obscured: ${step.targetSelector}`,
					);
				}
				const button = step.button ?? "left";
				await runtime.page.mouse.move(
					sourceBox.x + sourceBox.width / 2,
					sourceBox.y + sourceBox.height / 2,
				);
				runtime.heldMouseButton = button;
				await runtime.page.mouse.down({ button });
				// Give pointer-driven canvases (including XYFlow) one frame to enter
				// their drag state before the cursor starts moving.
				await delay(50);
				await runtime.page.mouse.move(
					targetBox.x + targetBox.width / 2,
					targetBox.y + targetBox.height / 2,
					{ steps: step.steps ?? 20 },
				);
				await delay(50);
				if (step.release ?? true) await releaseHeldMouse(runtime);
			} catch (error) {
				await releaseHeldMouse(runtime).catch(() => undefined);
				throw error;
			} finally {
				await target?.dispose();
				await sourceHandle.dispose();
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
				await handle.evaluate((element) => {
					if (
						element instanceof HTMLInputElement ||
						element instanceof HTMLTextAreaElement
					) {
						element.select();
					} else {
						const range = document.createRange();
						range.selectNodeContents(element);
						const selection = window.getSelection();
						selection?.removeAllRanges();
						selection?.addRange(range);
					}
				});
				await runtime.page.keyboard.press("Backspace");
				const value = resolveSecretValue(step);
				if (value) await handle.type(value);
				const filled = await handle.evaluate((element) =>
					element instanceof HTMLInputElement ||
					element instanceof HTMLTextAreaElement
						? element.value
						: null,
				);
				if (filled !== null && filled !== value)
					throw new Error(
						`Could not replace the value of screenshot field: ${step.selector}`,
					);
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
		case "seedIndexedDB":
			await runtime.page.waitForFunction(
				async (database) => {
					const factories = new Set<IDBFactory>([
						indexedDB,
						(window as any).__docScreenshotBrowserIndexedDB,
					]);
					for (const factory of factories) {
						if (
							(await factory.databases()).some(
								(entry) => entry.name === database,
							)
						)
							return true;
					}
					return false;
				},
				{ timeout: timeoutMs, polling: 250 },
				step.database,
			);
			await runtime.page.evaluate(async ({ database, stores }) => {
				let factory = indexedDB;
				if (
					!(await factory.databases()).some((entry) => entry.name === database)
				)
					factory = (window as any).__docScreenshotBrowserIndexedDB;
				const db = await new Promise<IDBDatabase>((resolve, reject) => {
					const request = factory.open(database);
					request.onerror = () => reject(request.error);
					request.onupgradeneeded = () => {
						request.transaction?.abort();
						reject(
							new Error(`Open the application before seeding ${database}.`),
						);
					};
					request.onsuccess = () => resolve(request.result);
				});
				try {
					await new Promise<void>((resolve, reject) => {
						const transaction = db.transaction(
							Object.keys(stores),
							"readwrite",
						);
						transaction.oncomplete = () => resolve();
						transaction.onerror = () => reject(transaction.error);
						transaction.onabort = () =>
							reject(transaction.error ?? new Error("Fixture seed aborted."));
						for (const [name, rows] of Object.entries(stores)) {
							const store = transaction.objectStore(name);
							for (const row of rows) store.put(row);
						}
					});
				} finally {
					db.close();
				}
			}, step);
			break;
		case "delay":
			await delay(step.ms);
			break;
		case "capture":
			try {
				return await captureScreenshot(runtime, step);
			} finally {
				await releaseHeldMouse(runtime);
			}
	}
	assertScenarioIntegrity(runtime);
}

async function runScenario(
	context: BrowserContext,
	plan: DocScreenshotPlan,
	scenario: DocScreenshotScenario,
	options: RunDocScreenshotOptions,
	staticAssets: Map<string, StaticAsset>,
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
	const intentionalBlocks = new WeakSet<HTTPRequest>();
	const diagnostics: ScenarioDiagnostics = {
		consoleErrors: 0,
		pageErrors: 0,
		requestFailures: 0,
		warnings: 0,
		entries: [],
		unexpected: 0,
	};
	const recordDiagnostic = (entry: DocScreenshotDiagnostic) => {
		const assessed = classifyScreenshotDiagnostic(
			entry,
			diagnostics.entries,
			scenario.diagnosticAllowlist ?? [],
		);
		diagnostics.entries.push(assessed);
		if (!assessed.allowance) diagnostics.unexpected += 1;
	};
	const stepResults: DocScreenshotStepResult[] = [];
	const artifacts: DocScreenshotArtifact[] = [];
	let title = "";
	let finalUrl = requestedUrl.toString();
	let scenarioError: string | undefined;
	const page = await context.newPage();
	const storageSession = await page.createCDPSession();
	await storageSession.send("Storage.clearDataForOrigin", {
		origin: baseUrl.origin,
		storageTypes:
			"cookies,local_storage,indexeddb,websql,service_workers,cache_storage",
	});
	await storageSession.detach();
	const runtime: ScenarioRuntime = {
		page,
		baseUrl,
		allowedOrigin: baseUrl.origin,
		outputDir: options.outputDir,
		defaults: plan.defaults,
		viewport,
		diagnostics,
		heldModifiers: new Set(),
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
		// Next dev sends no-store even for content-named module chunks. Keep one
		// compiled asset snapshot while resetting application state per scenario.
		await page.setRequestInterception(true);
		page.on("request", async (request) => {
			if (request.isInterceptResolutionHandled()) return;
			const requestUrl = new URL(request.url());
			const staticDirectory = process.env.DOC_SCREENSHOT_STATIC_DIR;
			if (
				staticDirectory &&
				requestUrl.origin === baseUrl.origin &&
				requestUrl.pathname.startsWith("/_next/static/") &&
				!staticAssets.has(request.url())
			) {
				const root = resolve(staticDirectory);
				const assetPath = resolve(
					root,
					decodeURIComponent(
						requestUrl.pathname.slice("/_next/static/".length),
					),
				);
				if (assetPath.startsWith(`${root}${sep}`)) {
					try {
						const body = await readFile(assetPath);
						const contentType = assetPath.endsWith(".js")
							? "application/javascript"
							: assetPath.endsWith(".css")
								? "text/css"
								: "application/octet-stream";
						staticAssets.set(request.url(), {
							body,
							headers: { "content-type": contentType },
						});
					} catch {
						/* Let Next serve assets it has not written yet. */
					}
				}
			}
			if (request.isInterceptResolutionHandled()) return;
			const cached = staticAssets.get(request.url());
			if (cached)
				await request.respond({
					status: 200,
					body: Buffer.from(cached.body),
					headers: cached.headers,
				});
			else if (!options.httpFixture) await request.continue();
		});
		page.on("response", async (response) => {
			const url = new URL(response.url());
			if (
				url.origin !== baseUrl.origin ||
				!url.pathname.startsWith("/_next/static/") ||
				response.status() !== 200 ||
				staticAssets.has(response.url())
			)
				return;
			try {
				const headers = response.headers();
				delete headers["content-encoding"];
				delete headers["content-length"];
				staticAssets.set(response.url(), {
					body: await response.buffer(),
					headers,
				});
			} catch {
				/* Navigation may cancel a speculative chunk response. */
			}
		});
		if (options.httpFixture) {
			await installHttpFixture(
				page,
				options.httpFixture,
				baseUrl.origin,
				(message) => {
					runtime.httpFixtureViolation ??= message;
				},
				intentionalBlocks,
			);
		}
		page.on("console", (message) => {
			if (process.env.DOC_SCREENSHOT_DEBUG === "1") {
				console.error(`[browser ${message.type()}] ${message.text()}`);
			}
			if (message.type() === "error") {
				diagnostics.consoleErrors += 1;
				recordDiagnostic({
					kind: "console",
					message: errorMessage(message.text()),
				});
			}
		});
		page.on("pageerror", (error) => {
			diagnostics.pageErrors += 1;
			recordDiagnostic({ kind: "page", message: errorMessage(error) });
			if (process.env.DOC_SCREENSHOT_DEBUG === "1") {
				console.error(
					`[browser pageerror] ${
						error instanceof Error
							? (error.stack ?? error.message)
							: String(error)
					}`,
				);
			}
		});
		page.on("requestfailed", (request) => {
			if (intentionalBlocks.has(request)) return;
			diagnostics.requestFailures += 1;
			const requestUrl = new URL(request.url());
			const location =
				requestUrl.origin === baseUrl.origin
					? `${requestUrl.pathname}${requestUrl.search}`
					: redactScreenshotUrl(request.url());
			recordDiagnostic({
				kind: "request",
				message: `${request.method()} ${errorMessage(location)}: ${request.failure()?.errorText ?? "unknown error"}`,
			});
			if (process.env.DOC_SCREENSHOT_DEBUG === "1") {
				console.error(
					`[browser requestfailed] ${request.method()} ${redactScreenshotUrl(request.url())}: ${request.failure()?.errorText ?? "unknown error"}`,
				);
			}
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
		assertScenarioIntegrity(runtime);
		for (const [index, step] of scenario.steps.entries()) {
			if (process.env.DOC_SCREENSHOT_DEBUG === "1") {
				console.error(
					`Step ${scenario.name} ${index + 1}/${scenario.steps.length}: ${step.type}${"selector" in step && step.selector ? ` ${step.selector}` : "text" in step && step.text ? ` ${step.text}` : ""}`,
				);
			}
			const stepStarted = Date.now();
			try {
				const artifact = await runStep(runtime, step);
				assertScenarioIntegrity(runtime);
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
		await releaseHeldInput(runtime);
		if (scenarioError && process.env.DOC_SCREENSHOT_DEBUG === "1") {
			await mkdir(options.outputDir, { recursive: true });
			await page
				.screenshot({
					path: resolve(options.outputDir, `${scenario.name}.failed.png`),
				})
				.catch(() => undefined);
			await writeFile(
				resolve(options.outputDir, `${scenario.name}.failed.html`),
				await page.content().catch(() => ""),
			).catch(() => undefined);
		}
		await page.close();
	}
	if (diagnostics.unexpected > 0) {
		const unexpected = diagnostics.entries.filter((entry) => !entry.allowance);
		scenarioError = [
			scenarioError,
			`Unexpected browser diagnostics (${diagnostics.unexpected}): ${unexpected.map((entry) => `${entry.kind}: ${entry.message}`).join("; ")}`,
		]
			.filter(Boolean)
			.join("\n");
	}
	return {
		name: scenario.name,
		sourceSha256: screenshotScenarioFingerprint(plan, scenario),
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
	const context = await browser.createBrowserContext();
	const staticAssets = new Map<string, StaticAsset>();
	const scenarios: DocScreenshotScenarioResult[] = [];
	let version = "";
	try {
		version = await browser.version();
		for (const scenario of plan.scenarios) {
			console.error(`Scenario: ${scenario.name}`);
			const result = await runScenario(
				context,
				plan,
				scenario,
				options,
				staticAssets,
			);
			scenarios.push(result);
			await mkdir(options.outputDir, { recursive: true });
			await writeFile(
				resolve(options.outputDir, `${scenario.name}.result.json`),
				JSON.stringify(
					{
						schema: DOC_SCREENSHOT_RESULT_SCHEMA,
						runId,
						passed: result.passed,
						startedAt: startedAt.toISOString(),
						finishedAt: new Date().toISOString(),
						durationMs: result.durationMs,
						baseUrl: redactScreenshotUrl(options.baseUrl),
						provenance: options.provenance,
						browser: { product: "Chromium", version, headless: true },
						scenarios: [result],
						summary: {
							scenarios: 1,
							scenariosPassed: result.passed ? 1 : 0,
							screenshots: result.artifacts.length,
						},
					},
					null,
					2,
				),
			);
			console.error(
				`${result.passed ? "PASS" : "FAIL"} ${scenario.name}: ${result.artifacts.length} capture(s), ${result.diagnostics.unexpected} unexpected diagnostic(s)${result.error ? `; ${result.error.slice(0, 300)}` : ""}`,
			);
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
		provenance: options.provenance,
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
