import { createI18n } from "@flow-like/locales";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Window } from "happy-dom";
import { type ReactNode, act, createElement } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

interface StubProps {
	readonly children?: ReactNode;
	readonly onClick?: () => void;
	readonly disabled?: boolean;
	readonly title?: string;
}

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	listen: vi.fn(),
	toastError: vi.fn(),
	toastInfo: vi.fn(),
	toastSuccess: vi.fn(),
}));

vi.mock("@flow-like/flow-like-ui", async () => {
	const query = await import("@tanstack/react-query");
	const stub =
		(tag: string, name: string) =>
		({ children, onClick, disabled, title }: StubProps) =>
			createElement(
				tag,
				{ "data-stub": name, onClick, disabled, title },
				children,
			);
	return {
		Alert: stub("div", "alert"),
		AlertDescription: stub("div", "alert-description"),
		AlertTitle: stub("div", "alert-title"),
		AlertDialog: stub("div", "alert-dialog"),
		AlertDialogAction: stub("button", "alert-dialog-action"),
		AlertDialogCancel: stub("button", "alert-dialog-cancel"),
		AlertDialogContent: stub("div", "alert-dialog-content"),
		AlertDialogDescription: stub("p", "alert-dialog-description"),
		AlertDialogFooter: stub("div", "alert-dialog-footer"),
		AlertDialogHeader: stub("div", "alert-dialog-header"),
		AlertDialogTitle: stub("h2", "alert-dialog-title"),
		AlertDialogTrigger: stub("div", "alert-dialog-trigger"),
		Badge: stub("span", "badge"),
		Button: stub("button", "button"),
		Card: stub("section", "card"),
		CardContent: stub("div", "card-content"),
		CardDescription: stub("p", "card-description"),
		CardHeader: stub("div", "card-header"),
		CardTitle: stub("h2", "card-title"),
		Progress: () => null,
		Skeleton: () => null,
		cn: (...classes: unknown[]) => classes.filter(Boolean).join(" "),
		useQuery: query.useQuery,
		useQueryClient: query.useQueryClient,
	};
});

vi.mock("next/link", () => ({
	default: ({ children }: StubProps) => createElement("a", null, children),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));

vi.mock("sonner", () => ({
	toast: {
		error: mocks.toastError,
		info: mocks.toastInfo,
		success: mocks.toastSuccess,
	},
}));

import AutomationSettingsPage from "../page";

const CFT_VERSION = "140.0.7339.80";
const REMOVE_ERROR =
	"Removing Chrome for Testing from /cache/browsers failed: Access is denied. (os error 5)";

interface StatusOverrides {
	readonly installed?: readonly {
		kind: "chrome" | "edge";
		flavor: string;
		path: string;
		version: string | null;
	}[];
	readonly cft_version?: string | null;
	readonly cft_supported?: boolean;
}

function engineStatus(overrides: StatusOverrides = {}) {
	return {
		installed: [],
		cft_version: null,
		cft_path: null,
		cft_supported: true,
		cft_pinned: "131.0.6778.204",
		cache_dir: "/cache/browsers",
		download_size_mb: 200,
		source: "storage.googleapis.com (Chrome for Testing)",
		...overrides,
		...(overrides.cft_version
			? { cft_path: `/cache/browsers/mac-arm64-${overrides.cft_version}` }
			: {}),
	};
}

const SNAP_CHROMIUM = {
	kind: "chrome",
	flavor: "snap_chromium",
	path: "/snap/bin/chromium",
	version: "139.0.7258.0",
} as const;

const browserGlobalKeys = [
	"window",
	"document",
	"HTMLElement",
	"Node",
	"navigator",
	"requestAnimationFrame",
	"cancelAnimationFrame",
	"IS_REACT_ACT_ENVIRONMENT",
] as const;

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
let root: Root | undefined;
let status: ReturnType<typeof engineStatus>;

function respond(overrides: Partial<Record<string, () => unknown>> = {}) {
	mocks.invoke.mockImplementation(async (command: string) => {
		const override = overrides[command];
		if (override) return override();
		if (command === "browser_engine_status") return status;
		if (command === "browser_engine_latest_cft") return CFT_VERSION;
		throw new Error(`Unexpected command ${command}`);
	});
}

async function settle(): Promise<void> {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
}

async function waitFor(check: () => boolean, what: string): Promise<void> {
	for (let attempt = 0; attempt < 50; attempt++) {
		if (check()) return;
		await settle();
	}
	throw new Error(`Timed out waiting for ${what}: ${container.textContent}`);
}

async function renderPage(): Promise<void> {
	const client = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	root = createRoot(container as unknown as Element);
	await act(async () => {
		root?.render(
			createElement(
				QueryClientProvider,
				{ client },
				createElement(AutomationSettingsPage),
			),
		);
	});
	await waitFor(() => cards().length === 2, "the status cards");
}

function cards() {
	return [...container.querySelectorAll('[data-stub="card"]')];
}

function detectedCard(): string {
	return cards()[0]?.textContent ?? "";
}

function badges(card: ReturnType<typeof cards>[number] | undefined): string[] {
	return [...(card?.querySelectorAll('[data-stub="badge"]') ?? [])].map(
		(badge) => badge.textContent ?? "",
	);
}

function cftCard() {
	const card = cards()[1];
	if (!card) throw new Error("The Chrome for Testing card is missing");
	return card;
}

function buttonsLabelled(label: string) {
	return [...cftCard().querySelectorAll("button")].filter(
		(button) => button.textContent?.trim() === label,
	);
}

async function click(element: { dispatchEvent: Window["dispatchEvent"] }) {
	await act(async () => {
		element.dispatchEvent(
			new browserWindow.MouseEvent("click", { bubbles: true }),
		);
	});
}

async function confirmRemove(): Promise<void> {
	const confirm = cftCard().querySelector('[data-stub="alert-dialog-action"]');
	if (!confirm) throw new Error("The Remove confirmation is missing");
	await click(confirm);
	await waitFor(
		() => buttonsLabelled("Remove").every((button) => !button.disabled),
		"the removal to finish",
	);
}

beforeEach(async () => {
	await createI18n({ language: "en" }).changeLanguage("en");
	browserWindow = new Window({ url: "http://localhost" });
	restoreGlobals = installBrowserGlobals(browserWindow);
	container = browserWindow.document.createElement("div");
	browserWindow.document.body.append(container);
	status = engineStatus();
	respond();
	mocks.listen.mockResolvedValue(() => {});
});

afterEach(async () => {
	if (root) await act(async () => root?.unmount());
	root = undefined;
	container.remove();
	restoreGlobals();
	await browserWindow.happyDOM.close();
	vi.clearAllMocks();
});

describe("removing Chrome for Testing", () => {
	test("a removal that fails after disabling the build shows the refreshed status", async () => {
		status = engineStatus({ cft_version: CFT_VERSION });
		await renderPage();
		expect(buttonsLabelled("Install")).toHaveLength(0);
		respond({
			browser_engine_remove_cft: () => {
				status = engineStatus();
				throw REMOVE_ERROR;
			},
		});

		await confirmRemove();

		expect(mocks.toastError).toHaveBeenCalledWith(
			"Could not remove Chrome for Testing",
			{ description: REMOVE_ERROR },
		);
		expect(cftCard().textContent).toContain("Not installed");
		expect(cftCard().textContent).not.toContain(CFT_VERSION);
		expect(buttonsLabelled("Install")).toHaveLength(1);
	});

	test("a successful removal reports it from the refreshed status", async () => {
		status = engineStatus({ cft_version: CFT_VERSION });
		await renderPage();
		respond({
			browser_engine_remove_cft: () => {
				status = engineStatus();
				return null;
			},
		});

		await confirmRemove();

		expect(mocks.toastSuccess).toHaveBeenCalledWith(
			"Chrome for Testing was removed",
		);
		expect(mocks.toastError).not.toHaveBeenCalled();
		expect(buttonsLabelled("Install")).toHaveLength(1);
	});

	test("a removal that keeps a build in use says so", async () => {
		status = engineStatus({ cft_version: CFT_VERSION });
		await renderPage();
		respond({ browser_engine_remove_cft: () => null });

		await confirmRemove();

		expect(mocks.toastInfo).toHaveBeenCalledWith(
			"Chrome for Testing is in use by an open browser and was kept. Close that browser and try again.",
		);
		expect(mocks.toastSuccess).not.toHaveBeenCalled();
		expect(cftCard().textContent).toContain(CFT_VERSION);
	});
});

describe("detected browsers text", () => {
	const DOWNLOAD_OFFER = /download Chrome for Testing/i;

	test("does not offer the download once Chrome for Testing is installed", async () => {
		status = engineStatus({ cft_version: CFT_VERSION });
		await renderPage();

		expect(detectedCard()).not.toMatch(DOWNLOAD_OFFER);
		expect(badges(cftCard())).toEqual(["Used by flows"]);
	});

	test("does not offer the download where Chrome for Testing has no build", async () => {
		status = engineStatus({ cft_supported: false });
		await renderPage();

		expect(detectedCard()).not.toMatch(DOWNLOAD_OFFER);
		expect(cftCard().textContent).toContain("Not available on this computer");
	});

	test("ranks Chrome for Testing before Snap Chromium, as the badge does", async () => {
		status = engineStatus({
			installed: [SNAP_CHROMIUM],
			cft_version: CFT_VERSION,
		});
		await renderPage();

		expect(badges(cards()[0])).toEqual([]);
		expect(badges(cftCard())).toEqual(["Used by flows"]);
		const descriptions = [
			...container.querySelectorAll('[data-stub="card-description"]'),
		].map((description) => description.textContent ?? "");
		expect(descriptions).toHaveLength(2);
		for (const description of descriptions) {
			expect(description).toContain("Chromium from Snap");
		}
	});
});
