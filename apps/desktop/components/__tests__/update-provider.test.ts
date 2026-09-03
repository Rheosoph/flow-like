import type { DownloadEvent } from "@tauri-apps/plugin-updater";
import { Window } from "happy-dom";
import { act, createElement } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

type EventHandler = (event: { payload?: unknown }) => void;

const mocks = vi.hoisted(() => ({
	addTelemetryBreadcrumb: vi.fn(),
	check: vi.fn(),
	confirm: vi.fn(),
	captureTelemetryError: vi.fn(),
	endTelemetrySpan: vi.fn(),
	getVersion: vi.fn(),
	invoke: vi.fn(),
	listen: vi.fn(),
	unlisten: vi.fn(),
	listeners: new Map<string, EventHandler>(),
	toastError: vi.fn(),
	toastDismiss: vi.fn(),
	toastInfo: vi.fn(),
	toastLoading: vi.fn(),
	toastSuccess: vi.fn(),
	windowLabel: "main",
}));

vi.mock("@tauri-apps/api/app", () => ({
	getVersion: mocks.getVersion,
}));

vi.mock("@flow-like/flow-like-ui/lib/telemetry/breadcrumbs", () => ({
	addTelemetryBreadcrumb: mocks.addTelemetryBreadcrumb,
}));

vi.mock("@flow-like/flow-like-ui/lib/telemetry/errors", () => ({
	captureTelemetryError: mocks.captureTelemetryError,
	normalizeError: (error: unknown) => ({
		kind: error instanceof Error ? error.name : "Error",
		value: error instanceof Error ? error.message : String(error),
		stack: error instanceof Error ? error.stack : undefined,
	}),
}));

vi.mock("@flow-like/flow-like-ui/lib/telemetry/tracing", () => ({
	startTelemetrySpan: vi.fn(() => ({
		traceId: "trace",
		spanId: "span",
		sampled: true,
		end: mocks.endTelemetrySpan,
	})),
}));

vi.mock("@tauri-apps/api/core", () => ({
	invoke: mocks.invoke,
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: mocks.listen,
}));

vi.mock("@tauri-apps/api/webviewWindow", () => ({
	getCurrentWebviewWindow: () => ({ label: mocks.windowLabel }),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
	confirm: mocks.confirm,
}));

vi.mock("@tauri-apps/plugin-updater", () => ({
	check: mocks.check,
}));

vi.mock("sonner", () => ({
	toast: {
		dismiss: mocks.toastDismiss,
		error: mocks.toastError,
		info: mocks.toastInfo,
		loading: mocks.toastLoading,
		success: mocks.toastSuccess,
	},
}));

vi.mock("../../lib/platform", () => ({
	isMobileDevice: () => false,
	isTauriRuntime: () => true,
}));

import { UpdateProvider } from "../update-provider";

const DISMISSED_VERSION_KEY = "updater:dismissed-version";

const browserGlobalKeys = [
	"window",
	"document",
	"HTMLElement",
	"Node",
	"navigator",
	"requestAnimationFrame",
	"cancelAnimationFrame",
	"localStorage",
	"sessionStorage",
	"IS_REACT_ACT_ENVIRONMENT",
] as const;

interface FakeUpdate {
	currentVersion: string;
	version: string;
	downloadAndInstall: ReturnType<typeof vi.fn>;
	close: ReturnType<typeof vi.fn>;
}

function fakeUpdate(
	version = "0.1.6",
	download?: (onEvent?: (event: DownloadEvent) => void) => Promise<void>,
): FakeUpdate {
	return {
		currentVersion: "0.1.4",
		version,
		downloadAndInstall: vi.fn(
			download ??
				(async (onEvent?: (event: DownloadEvent) => void) => {
					onEvent?.({ event: "Started", data: { contentLength: 100 } });
					onEvent?.({
						event: "Progress",
						data: { chunkLength: 50 },
					});
					onEvent?.({
						event: "Progress",
						data: { chunkLength: 50 },
					});
					onEvent?.({ event: "Finished" });
				}),
		),
		close: vi.fn().mockResolvedValue(undefined),
	};
}

function installBrowserGlobals(browserWindow: Window): () => void {
	const previous = new Map(
		browserGlobalKeys.map((key) => [
			key,
			Object.getOwnPropertyDescriptor(globalThis, key),
		]),
	);
	const values: Record<(typeof browserGlobalKeys)[number], unknown> = {
		window: browserWindow,
		document: browserWindow.document,
		HTMLElement: browserWindow.HTMLElement,
		Node: browserWindow.Node,
		navigator: browserWindow.navigator,
		requestAnimationFrame:
			browserWindow.requestAnimationFrame.bind(browserWindow),
		cancelAnimationFrame:
			browserWindow.cancelAnimationFrame.bind(browserWindow),
		localStorage: browserWindow.localStorage,
		sessionStorage: browserWindow.sessionStorage,
		IS_REACT_ACT_ENVIRONMENT: true,
	};

	for (const key of browserGlobalKeys) {
		Object.defineProperty(globalThis, key, {
			configurable: true,
			writable: true,
			value: values[key],
		});
	}

	return () => {
		for (const key of browserGlobalKeys) {
			const descriptor = previous.get(key);
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	};
}

let browserWindow: Window;
let container: ReturnType<Window["document"]["createElement"]>;
let restoreGlobals: () => void;
let originalNavigatorOnLine: PropertyDescriptor | undefined;
let root: Root | undefined;

async function mountProvider() {
	root = createRoot(container as unknown as Element);
	await act(async () => {
		root?.render(createElement(UpdateProvider));
	});
}

beforeEach(() => {
	vi.clearAllMocks();
	mocks.listeners.clear();
	mocks.windowLabel = "main";
	mocks.invoke.mockResolvedValue(undefined);
	mocks.getVersion.mockResolvedValue("0.1.4");
	mocks.confirm.mockResolvedValue(true);
	mocks.listen.mockImplementation(
		async (event: string, handler: EventHandler) => {
			mocks.listeners.set(event, handler);
			return mocks.unlisten;
		},
	);

	browserWindow = new Window({ url: "http://localhost" });
	restoreGlobals = installBrowserGlobals(browserWindow);
	originalNavigatorOnLine = Object.getOwnPropertyDescriptor(
		globalThis.navigator,
		"onLine",
	);
	container = browserWindow.document.createElement("div");
	browserWindow.document.body.append(container);
});

afterEach(async () => {
	if (root) {
		await act(async () => root?.unmount());
		root = undefined;
	}
	container.remove();
	browserWindow.close();
	if (originalNavigatorOnLine) {
		Object.defineProperty(
			globalThis.navigator,
			"onLine",
			originalNavigatorOnLine,
		);
	}
	restoreGlobals();
	vi.restoreAllMocks();
});

describe("UpdateProvider", () => {
	test("reports an interrupted native install when the old version starts again", async () => {
		localStorage.setItem(
			"updater:install-attempt",
			JSON.stringify({
				attempt_id: "attempt-1",
				current_version: "0.1.4",
				phase: "verifying_or_installing",
				target_version: "0.1.8",
				started_at_ms: Date.now() - 2_000,
				trigger: "manual",
			}),
		);
		mocks.check.mockResolvedValue(null);

		await mountProvider();

		await vi.waitFor(() => {
			expect(mocks.invoke).toHaveBeenCalledWith(
				"queue_updater_interruption",
				expect.objectContaining({
					attempt: expect.objectContaining({
						checkTrigger: "manual",
						currentVersion: "0.1.4",
						targetVersion: "0.1.8",
						attemptPhase: "verifying_or_installing",
					}),
				}),
			);
		});
		expect(localStorage.getItem("updater:install-attempt")).toBeNull();
		expect(mocks.captureTelemetryError).not.toHaveBeenCalled();
	});

	test("clears a native install marker after the target version launches", async () => {
		localStorage.setItem(
			"updater:install-attempt",
			JSON.stringify({
				attempt_id: "attempt-2",
				current_version: "0.1.4",
				phase: "restart_pending",
				target_version: "0.1.8",
				started_at_ms: Date.now() - 2_000,
				trigger: "automatic",
			}),
		);
		mocks.getVersion.mockResolvedValue("0.1.8");
		mocks.check.mockResolvedValue(null);

		await mountProvider();
		await vi.waitFor(() => expect(mocks.check).toHaveBeenCalledTimes(1));

		expect(localStorage.getItem("updater:install-attempt")).toBeNull();
		expect(mocks.captureTelemetryError).not.toHaveBeenCalled();
	});

	test("keeps an interrupted install marker until native telemetry persistence succeeds", async () => {
		vi.spyOn(console, "warn").mockImplementation(() => undefined);
		localStorage.setItem(
			"updater:install-attempt",
			JSON.stringify({
				attempt_id: "attempt-3",
				current_version: "0.1.4",
				phase: "downloading",
				target_version: "0.1.8",
				started_at_ms: Date.now() - 2_000,
				trigger: "automatic",
			}),
		);
		mocks.invoke.mockImplementation((command: string) =>
			command === "queue_updater_interruption"
				? Promise.reject(new Error("database unavailable"))
				: Promise.resolve(undefined),
		);
		mocks.check.mockResolvedValue(null);

		await mountProvider();
		await vi.waitFor(() => expect(mocks.check).toHaveBeenCalledTimes(1));

		expect(localStorage.getItem("updater:install-attempt")).not.toBeNull();
	});

	test("does not clear a new tray install while reconciling an older attempt", async () => {
		let finishPersisting: (() => void) | undefined;
		let finishInstall: (() => void) | undefined;
		localStorage.setItem(
			"updater:install-attempt",
			JSON.stringify({
				attempt_id: "old-attempt",
				current_version: "0.1.4",
				phase: "downloading",
				target_version: "0.1.8",
				started_at_ms: Date.now() - 2_000,
				trigger: "automatic",
			}),
		);
		mocks.invoke.mockImplementation((command: string) => {
			if (command === "queue_updater_interruption") {
				return new Promise<void>((resolve) => {
					finishPersisting = resolve;
				});
			}
			return Promise.resolve(undefined);
		});
		const update = fakeUpdate(
			"0.1.9",
			() =>
				new Promise<void>((resolve) => {
					finishInstall = resolve;
				}),
		);
		mocks.check.mockResolvedValue(update);

		await mountProvider();
		await vi.waitFor(() => {
			expect(mocks.listeners.has("tray:update-requested")).toBe(true);
			expect(finishPersisting).toBeDefined();
		});

		mocks.listeners.get("tray:update-requested")?.({});
		await vi.waitFor(() => {
			expect(update.downloadAndInstall).toHaveBeenCalledTimes(1);
		});
		const newAttempt = JSON.parse(
			localStorage.getItem("updater:install-attempt") ?? "{}",
		);
		expect(newAttempt).toMatchObject({
			current_version: "0.1.4",
			target_version: "0.1.9",
			phase: "starting",
		});
		expect(newAttempt.attempt_id).not.toBe("old-attempt");

		finishPersisting?.();
		await vi.waitFor(() => expect(mocks.check).toHaveBeenCalledTimes(1));
		expect(
			JSON.parse(localStorage.getItem("updater:install-attempt") ?? "{}"),
		).toMatchObject({ attempt_id: newAttempt.attempt_id });

		finishInstall?.();
		await vi.waitFor(() => {
			expect(mocks.invoke).toHaveBeenCalledWith("restart_app");
		});
	});

	test("durably reports an inconclusive updater outcome after an unexpected version launches", async () => {
		localStorage.setItem(
			"updater:install-attempt",
			JSON.stringify({
				attempt_id: "attempt-4",
				current_version: "0.1.4",
				phase: "restart_pending",
				target_version: "0.1.8",
				started_at_ms: Date.now() - 2_000,
				trigger: "automatic",
			}),
		);
		mocks.getVersion.mockResolvedValue("0.1.7");
		mocks.check.mockResolvedValue(null);

		await mountProvider();

		await vi.waitFor(() => {
			expect(mocks.invoke).toHaveBeenCalledWith(
				"queue_updater_interruption",
				expect.objectContaining({
					attempt: expect.objectContaining({
						currentVersion: "0.1.4",
						runningVersion: "0.1.7",
						targetVersion: "0.1.8",
						attemptPhase: "restart_pending",
					}),
				}),
			);
		});
		expect(localStorage.getItem("updater:install-attempt")).toBeNull();
	});

	test("installs the exact update returned by check and reports download and install progress", async () => {
		const update = fakeUpdate();
		mocks.check.mockResolvedValue(update);

		await mountProvider();

		await vi.waitFor(() => {
			expect(mocks.invoke).toHaveBeenCalledWith("restart_app");
		});

		expect(mocks.check).toHaveBeenCalledTimes(1);
		expect(mocks.check).toHaveBeenCalledWith({ timeout: 30_000 });
		expect(mocks.confirm).toHaveBeenCalledWith(
			"Flow Like 0.1.6 is available. Would you like to download and install it now?",
		);
		expect(update.downloadAndInstall).toHaveBeenCalledTimes(1);
		expect(update.close).toHaveBeenCalledTimes(1);
		expect(
			mocks.invoke.mock.calls.some(([command]) => command === "update"),
		).toBe(false);

		expect(mocks.toastLoading).toHaveBeenCalledWith(
			"Downloading Flow Like 0.1.6…",
			expect.objectContaining({
				id: "flow-like-update",
				description: "50% · 50 B of 100 B",
			}),
		);
		expect(mocks.toastLoading).toHaveBeenCalledWith(
			"Downloading Flow Like 0.1.6…",
			expect.objectContaining({ description: "100% · 100 B of 100 B" }),
		);
		expect(mocks.toastLoading).toHaveBeenCalledWith(
			"Installing Flow Like 0.1.6…",
			expect.objectContaining({
				description: "The application will restart when installation finishes.",
			}),
		);
		expect(mocks.toastSuccess).toHaveBeenCalledWith(
			"Flow Like was updated",
			expect.objectContaining({ description: "Restarting the application…" }),
		);
		expect(mocks.captureTelemetryError).not.toHaveBeenCalled();
		expect(mocks.endTelemetrySpan).toHaveBeenCalledWith(
			"ok",
			expect.objectContaining({ update_available: true }),
		);
		expect(mocks.endTelemetrySpan).toHaveBeenCalledWith(
			"ok",
			expect.objectContaining({ downloaded_bytes: 100 }),
		);
	});

	test("declining remembers only that version and closes its updater resource", async () => {
		const update = fakeUpdate("0.1.7");
		mocks.check.mockResolvedValue(update);
		mocks.confirm.mockResolvedValue(false);

		await mountProvider();

		await vi.waitFor(() => {
			expect(update.close).toHaveBeenCalledTimes(1);
		});

		expect(sessionStorage.getItem(DISMISSED_VERSION_KEY)).toBe("0.1.7");
		expect(update.downloadAndInstall).not.toHaveBeenCalled();
		expect(mocks.invoke).not.toHaveBeenCalledWith("restart_app");
		expect(mocks.toastInfo).toHaveBeenCalledWith(
			"Flow Like 0.1.7 is available",
			expect.objectContaining({
				description: "Install it later from the tray menu.",
				action: expect.objectContaining({ label: "Install" }),
			}),
		);
	});

	test("does not prompt again for a dismissed version and still closes the resource", async () => {
		const update = fakeUpdate("0.1.7");
		sessionStorage.setItem(DISMISSED_VERSION_KEY, update.version);
		mocks.check.mockResolvedValue(update);

		await mountProvider();

		await vi.waitFor(() => {
			expect(update.close).toHaveBeenCalledTimes(1);
		});

		expect(mocks.confirm).not.toHaveBeenCalled();
		expect(update.downloadAndInstall).not.toHaveBeenCalled();
	});

	test("does not let an older dismissal suppress a newer release", async () => {
		const update = fakeUpdate("0.1.8");
		sessionStorage.setItem(DISMISSED_VERSION_KEY, "0.1.7");
		mocks.check.mockResolvedValue(update);

		await mountProvider();

		await vi.waitFor(() => {
			expect(update.downloadAndInstall).toHaveBeenCalledTimes(1);
		});
		expect(mocks.confirm).toHaveBeenCalledTimes(1);
	});

	test("restarts only after installation has completed", async () => {
		let finishInstall: (() => void) | undefined;
		const update = fakeUpdate(
			"0.1.8",
			() =>
				new Promise<void>((resolve) => {
					finishInstall = resolve;
				}),
		);
		mocks.check.mockResolvedValue(update);

		await mountProvider();

		await vi.waitFor(() => {
			expect(update.downloadAndInstall).toHaveBeenCalledTimes(1);
		});
		expect(mocks.invoke).not.toHaveBeenCalledWith("restart_app");
		expect(
			JSON.parse(localStorage.getItem("updater:install-attempt") ?? "{}"),
		).toMatchObject({
			phase: "starting",
			current_version: "0.1.4",
			target_version: "0.1.8",
		});

		finishInstall?.();
		await vi.waitFor(() => {
			expect(mocks.invoke).toHaveBeenCalledWith("restart_app");
		});
		expect(
			JSON.parse(localStorage.getItem("updater:install-attempt") ?? "{}"),
		).toMatchObject({
			phase: "restart_pending",
		});
	});

	test("shows a persistent check error whose Retry action performs a fresh manual check", async () => {
		vi.spyOn(console, "error").mockImplementation(() => undefined);
		mocks.check
			.mockRejectedValueOnce(new Error("manifest unavailable"))
			.mockResolvedValueOnce(null);

		await mountProvider();

		await vi.waitFor(() => {
			expect(mocks.toastError).toHaveBeenCalledTimes(1);
		});
		expect(mocks.captureTelemetryError).toHaveBeenCalledTimes(1);
		expect(mocks.captureTelemetryError).toHaveBeenCalledWith(
			expect.objectContaining({ message: "manifest unavailable" }),
			expect.objectContaining({
				culprit: "desktop/updater/check",
				context: expect.objectContaining({
					subsystem: "updater",
					updater_stage: "check",
					check_source: "automatic",
					check_trigger: "automatic",
					network_online: true,
				}),
			}),
		);

		const options = mocks.toastError.mock.calls[0]?.[1] as {
			duration?: number;
			closeButton?: boolean;
			action?: { label: string; onClick: (event: unknown) => void };
		};
		expect(options.duration).toBe(Number.POSITIVE_INFINITY);
		expect(options.closeButton).toBe(true);
		expect(options.action?.label).toBe("Retry");

		options.action?.onClick({});

		await vi.waitFor(() => {
			expect(mocks.check).toHaveBeenCalledTimes(2);
			expect(mocks.toastSuccess).toHaveBeenCalledWith(
				"Flow Like is up to date",
				expect.objectContaining({ id: "flow-like-update" }),
			);
		});
		expect(mocks.captureTelemetryError).toHaveBeenCalledTimes(1);
	});

	test("reports download and install failures with safe progress context", async () => {
		vi.spyOn(console, "error").mockImplementation(() => undefined);
		const updateError = new Error("installer failed");
		const update = fakeUpdate("0.1.8", async (onEvent) => {
			onEvent?.({ event: "Started", data: { contentLength: 1_000 } });
			onEvent?.({ event: "Progress", data: { chunkLength: 250 } });
			throw updateError;
		});
		mocks.check.mockResolvedValue(update);

		await mountProvider();

		await vi.waitFor(() => {
			expect(mocks.captureTelemetryError).toHaveBeenCalledWith(
				updateError,
				expect.objectContaining({
					culprit: "desktop/updater/download_install",
					context: expect.objectContaining({
						updater_stage: "download_install",
						check_source: "automatic",
						check_trigger: "automatic",
						current_version: "0.1.4",
						target_version: "0.1.8",
						downloaded_bytes: 250,
						content_length_bytes: 1_000,
					}),
				}),
			);
		});
		expect(mocks.invoke).not.toHaveBeenCalledWith("restart_app");
		expect(localStorage.getItem("updater:install-attempt")).toBeNull();
		expect(mocks.endTelemetrySpan).toHaveBeenCalledWith(
			"error",
			expect.objectContaining({
				downloaded_bytes: 250,
				content_length_bytes: 1_000,
			}),
		);
	});

	test("reports update prompt failures with version and trigger context", async () => {
		vi.spyOn(console, "error").mockImplementation(() => undefined);
		const promptError = new Error("dialog unavailable");
		const update = fakeUpdate("0.1.8");
		mocks.check.mockResolvedValue(update);
		mocks.confirm.mockRejectedValue(promptError);

		await mountProvider();

		await vi.waitFor(() => {
			expect(mocks.captureTelemetryError).toHaveBeenCalledWith(
				promptError,
				expect.objectContaining({
					culprit: "desktop/updater/prompt",
					context: expect.objectContaining({
						updater_stage: "prompt",
						check_source: "automatic",
						check_trigger: "automatic",
						current_version: "0.1.4",
						target_version: "0.1.8",
					}),
				}),
			);
		});
		expect(update.downloadAndInstall).not.toHaveBeenCalled();
	});

	test("keeps checking after a failed prompt instead of latching the pending update", async () => {
		vi.spyOn(console, "error").mockImplementation(() => undefined);
		let intervalCallback: (() => void) | undefined;
		vi.spyOn(browserWindow, "setInterval").mockImplementation((handler) => {
			intervalCallback = () => handler();
			return {} as ReturnType<typeof browserWindow.setInterval>;
		});
		const update = fakeUpdate("0.1.8");
		mocks.check.mockResolvedValue(update);
		mocks.confirm.mockRejectedValue(new Error("dialog unavailable"));

		await mountProvider();

		await vi.waitFor(() => {
			expect(mocks.check).toHaveBeenCalledTimes(1);
			expect(intervalCallback).toBeDefined();
		});
		// The failed prompt must release the native resource, otherwise
		// `checkForUpdate` short-circuits on `pendingUpdate` for the whole session.
		await vi.waitFor(() => expect(update.close).toHaveBeenCalledTimes(1));

		intervalCallback?.();

		await vi.waitFor(() => expect(mocks.check).toHaveBeenCalledTimes(2));
	});

	test("reports a repeated automatic check failure only once per failure streak", async () => {
		vi.spyOn(console, "error").mockImplementation(() => undefined);
		vi.spyOn(console, "warn").mockImplementation(() => undefined);
		let intervalCallback: (() => void) | undefined;
		vi.spyOn(browserWindow, "setInterval").mockImplementation((handler) => {
			intervalCallback = () => handler();
			return {} as ReturnType<typeof browserWindow.setInterval>;
		});
		mocks.check.mockRejectedValue(new Error("manifest unavailable"));

		await mountProvider();
		await vi.waitFor(() => {
			expect(mocks.captureTelemetryError).toHaveBeenCalledTimes(1);
			expect(intervalCallback).toBeDefined();
		});

		intervalCallback?.();
		await vi.waitFor(() => expect(mocks.check).toHaveBeenCalledTimes(2));
		expect(mocks.captureTelemetryError).toHaveBeenCalledTimes(1);
		expect(mocks.addTelemetryBreadcrumb).toHaveBeenCalledWith(
			expect.objectContaining({
				category: "desktop.updater",
				message: "check_failure_repeated",
				level: "warning",
			}),
		);
	});

	test("keeps offline check failures visible without marking the session errored", async () => {
		vi.spyOn(console, "warn").mockImplementation(() => undefined);
		Object.defineProperty(globalThis.navigator, "onLine", {
			configurable: true,
			value: false,
		});
		mocks.check.mockRejectedValue(new Error("network unavailable"));

		await mountProvider();

		await vi.waitFor(() => {
			expect(mocks.toastError).toHaveBeenCalledTimes(1);
		});
		expect(mocks.captureTelemetryError).not.toHaveBeenCalled();
		expect(mocks.addTelemetryBreadcrumb).toHaveBeenCalledWith(
			expect.objectContaining({
				message: "check_failure_offline",
				level: "warning",
			}),
		);
	});

	test("reports restart failures after a successful installation", async () => {
		vi.spyOn(console, "error").mockImplementation(() => undefined);
		const restartError = new Error("restart denied");
		mocks.invoke.mockImplementation((command: string) =>
			command === "restart_app"
				? Promise.reject(restartError)
				: Promise.resolve(undefined),
		);
		mocks.check.mockResolvedValue(fakeUpdate("0.1.8"));

		await mountProvider();

		await vi.waitFor(() => {
			expect(mocks.captureTelemetryError).toHaveBeenCalledWith(
				restartError,
				expect.objectContaining({
					culprit: "desktop/updater/restart",
					context: expect.objectContaining({
						updater_stage: "restart",
						current_version: "0.1.4",
						target_version: "0.1.8",
					}),
				}),
			);
		});
	});

	test("reports tray listener registration failures", async () => {
		vi.spyOn(console, "error").mockImplementation(() => undefined);
		const listenerError = new Error("event listener unavailable");
		mocks.listen.mockRejectedValueOnce(listenerError);
		mocks.check.mockResolvedValue(null);

		await mountProvider();

		await vi.waitFor(() => {
			expect(mocks.captureTelemetryError).toHaveBeenCalledWith(
				listenerError,
				expect.objectContaining({
					culprit: "desktop/updater/tray_listener",
					context: expect.objectContaining({
						updater_stage: "tray_listener",
					}),
				}),
			);
		});
	});

	test("installs from the tray request without invoking the legacy update command", async () => {
		const update = fakeUpdate("0.1.8");
		mocks.check.mockResolvedValueOnce(null).mockResolvedValueOnce(update);

		await mountProvider();

		await vi.waitFor(() => {
			expect(mocks.listeners.has("tray:update-requested")).toBe(true);
			expect(mocks.check).toHaveBeenCalledTimes(1);
		});

		mocks.listeners.get("tray:update-requested")?.({});

		await vi.waitFor(() => {
			expect(update.downloadAndInstall).toHaveBeenCalledTimes(1);
			expect(mocks.invoke).toHaveBeenCalledWith("restart_app");
		});

		expect(mocks.check).toHaveBeenCalledTimes(2);
		expect(
			mocks.invoke.mock.calls.some(([command]) => command === "update"),
		).toBe(false);
	});

	test("preserves a tray install request while an automatic check is in flight", async () => {
		let intervalCallback: (() => void) | undefined;
		vi.spyOn(browserWindow, "setInterval").mockImplementation((handler) => {
			intervalCallback = () => handler();
			return {} as ReturnType<typeof browserWindow.setInterval>;
		});
		let resolveRefresh: ((update: FakeUpdate) => void) | undefined;
		const declinedUpdate = fakeUpdate("0.1.8");
		const refreshedUpdate = fakeUpdate("0.1.9");
		mocks.confirm.mockResolvedValueOnce(false);
		mocks.check.mockResolvedValueOnce(declinedUpdate).mockImplementationOnce(
			() =>
				new Promise<FakeUpdate>((resolve) => {
					resolveRefresh = resolve;
				}),
		);

		await mountProvider();

		await vi.waitFor(() => {
			expect(declinedUpdate.close).toHaveBeenCalledTimes(1);
			expect(intervalCallback).toBeDefined();
		});
		mocks.confirm.mockClear();
		intervalCallback?.();

		await vi.waitFor(() => {
			expect(mocks.check).toHaveBeenCalledTimes(2);
			expect(mocks.listeners.has("tray:update-requested")).toBe(true);
		});

		mocks.listeners.get("tray:update-requested")?.({});
		resolveRefresh?.(refreshedUpdate);

		await vi.waitFor(() => {
			expect(refreshedUpdate.downloadAndInstall).toHaveBeenCalledTimes(1);
		});
		expect(mocks.check).toHaveBeenCalledTimes(2);
		expect(mocks.confirm).not.toHaveBeenCalled();
	});

	test("does not start an updater owner in secondary webviews", async () => {
		mocks.windowLabel = "flow-window";
		mocks.check.mockResolvedValue(null);

		await mountProvider();

		expect(mocks.check).not.toHaveBeenCalled();
		expect(mocks.listen).not.toHaveBeenCalled();
		expect(mocks.invoke).not.toHaveBeenCalled();
	});
});
