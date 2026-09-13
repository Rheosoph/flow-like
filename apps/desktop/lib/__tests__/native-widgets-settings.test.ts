// @vitest-environment happy-dom

import {
	type NativeCustomWidget,
	type NativeWidgetPageDefinition,
	nativeWidgetScope,
	readNativeWidgetDefinitions,
	saveNativeWidgetDefinitions,
} from "@flow-like/flow-like-ui/lib/native-widget";
import React, { act, createElement } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	profile: { id: "profile-a", hub: "api.test", apps: [{ app_id: "app" }] },
	auth: {
		isLoading: false,
		isAuthenticated: true,
		user: { profile: { sub: "viewer-a" } },
	},
	preview: vi.fn(),
	cache: vi.fn(),
	success: vi.fn(),
	error: vi.fn(),
	push: vi.fn(),
	backend: {
		userState: { getProfile: vi.fn() },
		appState: { getApps: vi.fn() },
		eventState: { getEvents: vi.fn() },
		pageState: { getPageBootstrap: vi.fn() },
	},
}));
vi.mock("@flow-like/flow-like-ui/state/backend-state", () => ({
	useBackend: () => mocks.backend,
	useBackendReady: () => true,
}));
vi.mock("@flow-like/flow-like-ui/hooks/use-invoke", () => ({
	useInvoke: () => ({ isSuccess: true, data: mocks.profile }),
}));
vi.mock("@tanstack/react-query", () => ({
	useQuery: () => ({ data: [{ id: "app", name: "Support" }] }),
}));
vi.mock("react-oidc-context", () => ({ useAuth: () => mocks.auth }));
vi.mock("next/navigation", () => ({ useRouter: () => ({ push: mocks.push }) }));
vi.mock("next/link", () => ({
	default: ({ children, ...props }: React.ComponentPropsWithoutRef<"a">) =>
		createElement("a", props, children),
}));
vi.mock("@flow-like/flow-like-ui/components/home/data-widget-settings", () => ({
	HomeDataWidgetSettings: () => createElement("div", null, "Chart settings"),
}));
vi.mock("../native-custom-widgets", () => ({
	previewNativeCustomWidget: mocks.preview,
	readNativeCustomWidgetCache: mocks.cache,
}));
vi.mock("../../components/native-widget-preview", () => ({
	NativeWidgetPreview: ({ widget }: { widget: NativeCustomWidget }) =>
		createElement(
			"div",
			{ "data-testid": "native-preview" },
			widget.page?.text ?? widget.title,
		),
}));
vi.mock("sonner", () => ({
	toast: { success: mocks.success, error: mocks.error, info: vi.fn() },
}));

import { NativeWidgetsSettings } from "../../components/native-widgets-settings";

let root: Root;
let host: HTMLDivElement;
const scope = () =>
	nativeWidgetScope(
		{ profile: mocks.profile as never },
		mocks.auth.user.profile.sub,
	);
const saved = (): NativeWidgetPageDefinition => ({
	id: "saved-page",
	title: "Account A page",
	kind: "page",
	appId: "app",
	path: "/orders",
	queryParams: [],
	accent: "orange",
	refreshMinutes: 30,
	updatedAt: "2026-09-13T10:00:00.000Z",
});
function button(text: string): HTMLButtonElement {
	const found = [...host.querySelectorAll<HTMLButtonElement>("button")].find(
		(element) =>
			element.textContent?.trim() === text ||
			element.getAttribute("aria-label") === text,
	);
	if (!found) throw new Error(`Missing button: ${text}`);
	return found;
}
function field(label: string): HTMLInputElement | HTMLSelectElement {
	const match = [...host.querySelectorAll("label")].find(
		(element) => element.textContent?.trim() === label,
	);
	const found = match?.htmlFor
		? document.getElementById(match.htmlFor)
		: undefined;
	if (
		!(found instanceof HTMLInputElement || found instanceof HTMLSelectElement)
	)
		throw new Error(`Missing field: ${label}`);
	return found;
}
async function click(text: string) {
	await act(async () => button(text).click());
}
async function change(label: string, value: string) {
	await act(async () => {
		const element = field(label);
		const prototype =
			element instanceof HTMLSelectElement
				? HTMLSelectElement.prototype
				: HTMLInputElement.prototype;
		Object.getOwnPropertyDescriptor(prototype, "value")?.set?.call(
			element,
			value,
		);
		element.dispatchEvent(
			new Event(element instanceof HTMLSelectElement ? "change" : "input", {
				bubbles: true,
			}),
		);
	});
}
async function mount() {
	await act(async () => root.render(createElement(NativeWidgetsSettings)));
}
async function createPage() {
	await mount();
	await click("App Page");
	await change("App", "app");
}

beforeEach(() => {
	vi.stubGlobal("React", React);
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	const values = new Map<string, string>();
	vi.stubGlobal("localStorage", {
		getItem: (key: string) => values.get(key) ?? null,
		setItem: (key: string, value: string) => values.set(key, value),
		removeItem: (key: string) => values.delete(key),
	});
	mocks.auth.user.profile.sub = "viewer-a";
	mocks.auth.isLoading = false;
	mocks.preview.mockReset();
	mocks.cache.mockReset().mockReturnValue([]);
	mocks.success.mockReset();
	mocks.error.mockReset();
	mocks.push.mockReset();
	mocks.backend.eventState.getEvents.mockReset().mockResolvedValue([]);
	mocks.backend.pageState.getPageBootstrap
		.mockReset()
		.mockResolvedValue({ page: { components: [] } });
	host = document.createElement("div");
	document.body.appendChild(host);
	root = createRoot(host);
});
afterEach(async () => {
	await act(async () => root?.unmount());
	host?.remove();
	vi.unstubAllGlobals();
});

describe("native widget settings", () => {
	it("saves a native page path and raw query values once within the current account", async () => {
		await createPage();
		await change("Widget name", "Support overview");
		await change("App path", "/orders?tag=a%2Bb");
		await click("Query parameter");
		await change("Parameter 1", "search");
		await change("Value", "A+B & 東京 50%");
		await click("Save widget");
		await click("Save widget");
		const definitions = readNativeWidgetDefinitions(scope());
		expect(definitions).toHaveLength(1);
		expect(definitions[0]).toMatchObject({
			kind: "page",
			appId: "app",
			title: "Support overview",
			path: "/orders",
			queryParams: [
				{ name: "tag", value: "a+b" },
				{ name: "search", value: "A+B & 東京 50%" },
			],
		});
		expect(mocks.success).toHaveBeenCalledTimes(2);
		expect(mocks.preview).not.toHaveBeenCalled();
	});

	it("shows route validation errors before saving or fetching a preview", async () => {
		await createPage();
		await change("App path", "https://outside.test/private");
		await click("Save widget");
		expect(host.querySelector('[role="alert"]')?.textContent).toContain(
			"internal path",
		);
		expect(readNativeWidgetDefinitions(scope())).toEqual([]);
		await click("Preview latest content");
		expect(mocks.preview).not.toHaveBeenCalled();
	});

	it("drops the previous account's editor and saved list when the account changes", async () => {
		saveNativeWidgetDefinitions(scope(), [saved()]);
		await mount();
		expect(host.textContent).toContain("Account A page");
		await click("App Page");
		await change("Widget name", "Unsaved private draft");
		mocks.auth.user.profile.sub = "viewer-b";
		await mount();
		expect(host.textContent).not.toContain("Account A page");
		expect(host.querySelector("input")?.value).not.toBe(
			"Unsaved private draft",
		);
		expect(readNativeWidgetDefinitions(scope())).toEqual([]);
	});

	it("cancels a stale preview when the page settings change", async () => {
		let complete!: (value: NativeCustomWidget) => void;
		mocks.preview.mockImplementation(
			() =>
				new Promise<NativeCustomWidget>((resolve) => {
					complete = resolve;
				}),
		);
		await createPage();
		await click("Preview latest content");
		const signal = mocks.preview.mock.calls[0][2].signal as AbortSignal;
		await change("App path", "/new-page");
		expect(signal.aborted).toBe(true);
		await act(async () =>
			complete({
				id: "old",
				title: "Outdated preview",
				kind: "page",
				appId: "app",
				updatedAt: new Date().toISOString(),
				staleAt: new Date().toISOString(),
				expiresAt: new Date().toISOString(),
				state: "ready",
				action: { kind: "open_app", appId: "app" },
				page: { id: "root", kind: "text", text: "Outdated private result" },
			}),
		);
		expect(host.textContent).not.toContain("Outdated private result");
		expect(host.querySelector('[data-testid="native-preview"]')).toBeNull();
	});
});
