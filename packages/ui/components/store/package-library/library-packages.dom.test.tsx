import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import type { Root } from "react-dom/client";
import {
	type InstalledPackage,
	PackageStatus,
	type PackageSummary,
	type PackageUpdate,
} from "../../../lib/schema/wasm";
import type { LibraryMachine } from "./library-packages";
import type { LibraryAuth } from "./use-library-packages";

/** The first dynamic import transpiles the backend-state graph, which can take seconds on a cold cache. */
const COLD_IMPORT_TIMEOUT_MS = 30_000;

let window: Window;
let root: Root;
let host: HTMLElement;
let restoreGlobals: () => void;

beforeEach(async () => {
	window = new Window({
		url: "https://app.flow-like.com/store/packages?tab=library",
	});
	const globals = {
		window,
		document: window.document,
		navigator: window.navigator,
		localStorage: window.localStorage,
		HTMLElement: window.HTMLElement,
		HTMLButtonElement: window.HTMLButtonElement,
		HTMLInputElement: window.HTMLInputElement,
		HTMLAnchorElement: window.HTMLAnchorElement,
		Element: window.Element,
		Text: window.Text,
		DocumentFragment: window.DocumentFragment,
		Node: window.Node,
		MutationObserver: window.MutationObserver,
		IntersectionObserver: window.IntersectionObserver,
		Event: window.Event,
		InputEvent: window.InputEvent,
		CustomEvent: window.CustomEvent,
		KeyboardEvent: window.KeyboardEvent,
		FocusEvent: window.FocusEvent,
		MouseEvent: window.MouseEvent,
		PointerEvent: window.PointerEvent,
		NodeFilter: window.NodeFilter,
		ResizeObserver: window.ResizeObserver,
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		getComputedStyle: window.getComputedStyle.bind(window),
		matchMedia: window.matchMedia.bind(window),
		IS_REACT_ACT_ENVIRONMENT: true,
	};
	const descriptors = Object.keys(globals).map(
		(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
	);
	Object.assign(globalThis, globals);
	Object.assign(window, { SyntaxError, TypeError });
	restoreGlobals = () => {
		for (const [key, descriptor] of descriptors) {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	};

	const { resetMicroWidgetConsentForTests } = await import(
		"../../a2ui/micro-widget-capability-consent"
	);
	resetMicroWidgetConsentForTests();
	const { createRoot } = await import("react-dom/client");
	host = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(host as never);
	root = createRoot(host);
});

afterEach(async () => {
	const { act } = await import("react");
	await act(() => root.unmount());
	await window.happyDOM.abort();
	restoreGlobals();
});

type RegistryMock = Record<string, (...args: never[]) => Promise<unknown>>;

interface MountOptions {
	signedIn: boolean | undefined;
	auth?: { isLoading?: boolean; isAuthenticated?: boolean };
	registry: RegistryMock;
	machine?: LibraryMachine;
}

function summary(
	id: string,
	overrides: Partial<PackageSummary> = {},
): PackageSummary {
	return {
		id,
		name: id,
		description: `${id} description`,
		latestVersion: "1.1.0",
		downloadCount: 3,
		status: PackageStatus.Active,
		keywords: [],
		verified: false,
		price: 0,
		visibility: "public",
		...overrides,
	};
}

function installed(
	id: string,
	manifest: Record<string, unknown> = {},
	source: "remote" | "local" = "remote",
): InstalledPackage {
	return {
		id,
		version: "1.0.0",
		source: { type: source },
		installedAt: "",
		wasmPath: "",
		manifest: {
			id,
			name: id,
			description: "",
			keywords: [],
			...manifest,
		} as unknown as InstalledPackage["manifest"],
	};
}

const HTTP = { permissions: { network: { http_enabled: true } } };
const FREE_UPDATE: PackageUpdate = {
	packageId: "free-pkg",
	packageName: "free-pkg",
	currentVersion: "1.0.0",
	latestVersion: "1.1.0",
};
const FREE_SUMMARY = summary("free-pkg", {
	capabilities: ["net.http", "widget.net"],
});
const DESKTOP: LibraryMachine = { getPackageStatus: () => undefined };

async function flush(ms = 20) {
	const { act } = await import("react");
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, ms));
	});
}

async function until(check: () => boolean, label: string) {
	for (let attempt = 0; attempt < 50; attempt += 1) {
		if (check()) return;
		await flush(10);
	}
	throw new Error(`Timed out waiting for ${label}`);
}

async function mount(options: MountOptions) {
	const { act } = await import("react");
	const { QueryClient, QueryClientProvider } = await import(
		"@tanstack/react-query"
	);
	const { useAuthStatusStore, useBackendStore } = await import(
		"../../../state/backend-state"
	);
	const { LibraryPackages } = await import("./library-packages");
	useBackendStore
		.getState()
		.setBackend({ registryState: options.registry } as never);
	useAuthStatusStore.setState({ signedIn: options.signedIn });
	const client = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	/** `hidden` unmounts the tab while keeping the query cache, like leaving for the store detail. */
	const render = async (authState: MountOptions["auth"], hidden = false) => {
		const auth: LibraryAuth = {
			isLoading: false,
			isAuthenticated: options.signedIn === true,
			...authState,
			user: options.signedIn
				? { access_token: "token", profile: { sub: "user-1" } }
				: null,
			signinRedirect: async () => {},
		};
		await act(async () =>
			root.render(
				<QueryClientProvider client={client}>
					{hidden ? null : (
						<LibraryPackages
							auth={auth}
							navigation={<nav data-nav>Mine · Library</nav>}
							machine={options.machine}
						/>
					)}
				</QueryClientProvider>,
			),
		);
		await flush();
	};
	await render(options.auth);
	return { rerender: render };
}

function card(id: string): HTMLElement {
	const element = host.querySelector<HTMLElement>(
		`[data-library-package="${id}"]`,
	);
	if (!element) throw new Error(`No library card for ${id}`);
	return element;
}

/** The ids lookup starts only after the installed list and the library listing settle. */
async function lookupSettled(id: string) {
	await until(() => {
		const text = host.querySelector(
			`[data-library-package="${id}"]`,
		)?.textContent;
		return Boolean(text && !text.includes("Checking permissions"));
	}, `the ${id} lookup`);
}

function buttonsOf(element: Element | null): HTMLButtonElement[] {
	return Array.from(element?.querySelectorAll("button") ?? []);
}

function button(scope: Element, text: string): HTMLButtonElement {
	const found = buttonsOf(scope).find((candidate) =>
		candidate.textContent?.includes(text),
	);
	if (!found) throw new Error(`No button "${text}"`);
	return found;
}

async function click(element: HTMLElement) {
	const { act } = await import("react");
	await act(async () => element.click());
	await flush();
}

async function press(
	key: string,
	target: Element | null = host.ownerDocument.activeElement,
) {
	const { act } = await import("react");
	await act(async () => {
		target?.dispatchEvent(
			new KeyboardEvent("keydown", {
				key,
				bubbles: true,
				cancelable: true,
			}),
		);
		await new Promise((resolve) => setTimeout(resolve, 10));
	});
}

/** Dialogs and menus portal into the body, outside `host`. */
function body(): HTMLElement {
	return host.ownerDocument.body;
}

function menuItems(): HTMLElement[] {
	return Array.from(body().querySelectorAll<HTMLElement>('[role="menuitem"]'));
}

async function openMenu(id: string): Promise<string[]> {
	const { act } = await import("react");
	const more = card(id).querySelector<HTMLButtonElement>(
		`button[aria-label="More actions for ${id}"]`,
	);
	if (!more) throw new Error(`No "More actions for ${id}" button`);
	await act(async () => more?.focus());
	await press("Enter", more);
	await until(() => menuItems().length > 0, `the ${id} menu`);
	return menuItems().map((item) => item.textContent ?? "");
}

async function closeMenu() {
	await press("Escape");
	await until(() => menuItems().length === 0, "the menu to close");
}

function desktopRegistry(log: string[], overrides: RegistryMock = {}) {
	return {
		getOwnedPackages: async () => {
			log.push("owned");
			return {
				packages: [summary("bought-pkg", { viewerPermission: 8, price: 499 })],
			};
		},
		searchPackages: async () => {
			log.push("search");
			return { packages: [FREE_SUMMARY] };
		},
		getInstalledPackages: async () => {
			log.push("installed");
			return [
				installed("free-pkg", {
					...HTTP,
					widgets: [{ id: "w", name: "W", contract: { id: "w" } }],
				}),
			];
		},
		checkForUpdates: async () => {
			log.push("updates");
			return [FREE_UPDATE];
		},
		installPackage: async (id: string, version?: string) => {
			log.push(`install:${id}@${version}`);
			return {};
		},
		updatePackage: async (id: string, version: string) => {
			log.push(`update:${id}@${version}`);
			return {};
		},
		uninstallPackage: async (id: string) => {
			log.push(`uninstall:${id}`);
		},
		...overrides,
	} satisfies RegistryMock;
}

function webRegistry(log: string[], packages: PackageSummary[] = []) {
	return {
		getOwnedPackages: async () => {
			log.push("owned");
			return { packages };
		},
		searchPackages: async () => {
			log.push("search");
			return { packages: [] };
		},
		getInstalledPackages: async () => [],
		checkForUpdates: async () => [],
	} satisfies RegistryMock;
}

describe("auth gating", () => {
	test(
		"while auth is unknown: skeleton under the Packages header, no registry request",
		async () => {
			const log: string[] = [];
			await mount({
				signedIn: undefined,
				auth: { isLoading: true },
				registry: webRegistry(log),
			});
			expect(log).toEqual([]);
			expect(host.querySelector("h1")?.textContent).toBe("Packages");
			expect(host.textContent).not.toContain("Explore");
			expect(host.querySelector("[data-nav]")).not.toBe(null);
			expect(host.querySelector("[data-library-sign-in]")).toBe(null);
			expect(host.querySelector("[data-library-empty]")).toBe(null);
			expect(
				host
					.querySelector("#library-package-results")
					?.getAttribute("aria-busy"),
			).toBe("true");
		},
		COLD_IMPORT_TIMEOUT_MS,
	);

	test("a signed-out push while OIDC is loading keeps the skeleton", async () => {
		const log: string[] = [];
		const { rerender } = await mount({
			signedIn: false,
			auth: { isLoading: true },
			registry: webRegistry(log),
		});
		expect(host.querySelector("[data-library-sign-in]")).toBe(null);
		expect(
			host.querySelector("#library-package-results")?.getAttribute("aria-busy"),
		).toBe("true");

		await rerender({ isLoading: false });
		expect(host.querySelector("[data-library-sign-in]")).not.toBe(null);
		expect(log).toEqual([]);
	});

	test("signed out on web: sign-in call to action, never the empty state", async () => {
		const log: string[] = [];
		await mount({ signedIn: false, registry: webRegistry(log) });
		expect(log).toEqual([]);
		expect(host.querySelector("[data-library-sign-in]")).not.toBe(null);
		expect(host.textContent).not.toContain("Nothing in your library");
	});

	test("a desktop that never pushes sign-in settles from the OIDC state", async () => {
		const log: string[] = [];
		await mount({
			signedIn: undefined,
			registry: desktopRegistry(log),
			machine: DESKTOP,
		});
		await lookupSettled("free-pkg");
		expect(host.querySelector("[data-library-signed-out]")).not.toBe(null);
		expect(log).not.toContain("owned");
		expect(log).toContain("search");
		expect(card("free-pkg").textContent).toContain("Free");
	});
});

describe("registry failures", () => {
	test("a 500 shows an error with Retry, not the empty state", async () => {
		let owned = 0;
		await mount({
			signedIn: true,
			registry: {
				...webRegistry([]),
				getOwnedPackages: async () => {
					owned += 1;
					throw new Error("500");
				},
			},
		});
		await until(
			() => Boolean(host.querySelector("[data-library-error]")),
			"the registry error",
		);
		const error = host.querySelector("[data-library-error]");
		expect(error?.textContent).toContain("Retry");
		expect(host.querySelector("[data-library-empty]")).toBe(null);
		if (error) await click(button(error, "Retry"));
		await until(() => owned === 2, "the retry");
	});
});

describe("desktop cards", () => {
	test("merges access rows with installed packages", async () => {
		const calls: unknown[] = [];
		const log: string[] = [];
		await mount({
			signedIn: true,
			machine: DESKTOP,
			registry: desktopRegistry(log, {
				getOwnedPackages: async (filters: unknown) => {
					calls.push(filters);
					return {
						packages: [
							summary("bought-pkg", { viewerPermission: 8, price: 499 }),
						],
					};
				},
				searchPackages: async (filters: unknown) => {
					calls.push(filters);
					return { packages: [FREE_SUMMARY] };
				},
			}),
		});
		await lookupSettled("free-pkg");
		expect(calls[0]).toEqual({ access: "library", limit: 100 });
		expect(calls[1]).toMatchObject({ ids: ["free-pkg"] });
		const bought = card("bought-pkg");
		expect(bought.textContent).toContain("Bought");
		expect(bought.textContent).toContain("On this machine");
		expect(buttonsOf(bought).map((b) => b.textContent)).toContain("Install");
		expect(bought.querySelector("h3 a")?.getAttribute("href")).toBe(
			"/store/packages?id=bought-pkg&tab=library",
		);
		expect(card("free-pkg").textContent).toContain("adds widget.net");
	});

	test("an update adding permissions asks first and lists only the new ones", async () => {
		const log: string[] = [];
		await mount({
			signedIn: true,
			machine: DESKTOP,
			registry: desktopRegistry(log),
		});
		await lookupSettled("free-pkg");
		await click(button(card("free-pkg"), "Review 1 new permission"));
		const tags = body().querySelector("[data-update-consent-tags]");
		expect(tags?.textContent).toContain("widget.net");
		expect(tags?.textContent).not.toContain("net.http");
		expect(log).not.toContain("update:free-pkg@1.1.0");
		await click(button(body(), "Allow and update"));
		expect(log).toContain("update:free-pkg@1.1.0");
	});

	test("while the library is still loading an update cannot run unchecked", async () => {
		const log: string[] = [];
		await mount({
			signedIn: true,
			machine: DESKTOP,
			registry: desktopRegistry(log, {
				getOwnedPackages: () => new Promise(() => {}),
			}),
		});
		await until(
			() => Boolean(host.querySelector('[data-library-package="free-pkg"]')),
			"the installed card",
		);
		await flush(50);
		const update = button(card("free-pkg"), "Checking permissions");
		expect(update.disabled).toBe(true);
		await click(update);
		expect(log.some((entry) => entry.startsWith("update:"))).toBe(false);
	});

	test("when the lookup fails the update asks before running", async () => {
		const log: string[] = [];
		await mount({
			signedIn: false,
			machine: DESKTOP,
			registry: desktopRegistry(log, {
				searchPackages: async () => {
					throw new Error("503");
				},
			}),
		});
		await lookupSettled("free-pkg");
		const free = card("free-pkg");
		expect(free.textContent).toContain("could not be checked");
		await click(button(free, "Review update"));
		expect(body().querySelector("[data-update-unverified]")).not.toBe(null);
		expect(log.some((entry) => entry.startsWith("update:"))).toBe(false);
		await click(button(body(), "Update anyway"));
		expect(log).toContain("update:free-pkg@1.1.0");
	});

	test("a failed local .wasm offers a reload, never a registry install", async () => {
		const log: string[] = [];
		let loads = 0;
		await mount({
			signedIn: true,
			machine: {
				getPackageStatus: (id) => (id === "dev-pkg" ? "error" : undefined),
				onLoadLocal: async () => {
					loads += 1;
					throw new Error("not a wasm module");
				},
			},
			registry: desktopRegistry(log, {
				getInstalledPackages: async () => {
					log.push("installed");
					return [installed("dev-pkg", {}, "local")];
				},
				checkForUpdates: async () => [],
			}),
		});
		await lookupSettled("dev-pkg");
		const dev = card("dev-pkg");
		expect(dev.textContent).toContain("load it again");
		expect(buttonsOf(dev).map((b) => b.textContent)).not.toContain("Retry");
		const installedReads = () =>
			log.filter((entry) => entry === "installed").length;
		const before = installedReads();
		await click(button(dev, "Load local .wasm"));
		expect(loads).toBe(1);
		await until(() => installedReads() > before, "the installed refetch");
		expect(log.some((entry) => entry.startsWith("install:"))).toBe(false);
	});

	test("the menu is keyboard-selectable and Uninstall refreshes installed and library", async () => {
		const log: string[] = [];
		await mount({
			signedIn: true,
			machine: DESKTOP,
			registry: desktopRegistry(log),
		});
		await lookupSettled("free-pkg");
		const items = await openMenu("free-pkg");
		expect(items).toContain("Clear widget permissions on this device");
		const uninstall = menuItems().find((item) =>
			item.textContent?.startsWith("Uninstall"),
		);
		expect(uninstall).toBeDefined();
		const start = log.length;
		if (uninstall) await click(uninstall);
		expect(log).toContain("uninstall:free-pkg");
		await until(() => {
			const after = log.slice(start);
			return after.includes("installed") && after.includes("owned");
		}, "the installed and library refetch");
	});
});

describe("web cards", () => {
	test("search tolerates typos", async () => {
		const { act } = await import("react");
		await mount({
			signedIn: true,
			registry: webRegistry(
				[],
				[
					summary("alpha-maps", { viewerPermission: 4 }),
					summary("beta-charts", { viewerPermission: 8, price: 100 }),
				],
			),
		});
		await lookupSettled("beta-charts");
		const input = host.querySelector<HTMLInputElement>('input[type="search"]');
		const setValue = Object.getOwnPropertyDescriptor(
			window.HTMLInputElement.prototype,
			"value",
		)?.set;
		await act(async () => {
			setValue?.call(input, "chrts");
			input?.dispatchEvent(
				new window.InputEvent("input", { bubbles: true }) as unknown as Event,
			);
		});
		await flush();
		expect(
			Array.from(host.querySelectorAll("[data-library-package]")).map((item) =>
				item.getAttribute("data-library-package"),
			),
		).toEqual(["beta-charts"]);
	});

	test("returning to the tab refetches the library, so a new purchase shows", async () => {
		let owned = 0;
		let packages = [summary("shared-pkg", { viewerPermission: 4 })];
		const { rerender } = await mount({
			signedIn: true,
			registry: {
				...webRegistry([]),
				getOwnedPackages: async () => {
					owned += 1;
					return { packages };
				},
			},
		});
		await lookupSettled("shared-pkg");
		expect(owned).toBe(1);
		await rerender(undefined, true);
		packages = [
			...packages,
			summary("bought-pkg", { viewerPermission: 8, price: 100 }),
		];
		await rerender(undefined);
		await lookupSettled("bought-pkg");
		expect(owned).toBe(2);
	});

	test("an active access filter stays clearable after its other entries leave", async () => {
		let owned = 0;
		const bought = summary("bought-pkg", { viewerPermission: 8, price: 100 });
		let packages = [bought, summary("shared-pkg", { viewerPermission: 4 })];
		await mount({
			signedIn: true,
			registry: {
				...webRegistry([]),
				getOwnedPackages: async () => {
					owned += 1;
					return { packages };
				},
			},
		});
		await lookupSettled("shared-pkg");
		const accessRow = () => {
			const row = host.querySelector('fieldset[aria-label="Filter by access"]');
			if (!row) throw new Error("No access filter row");
			return row;
		};
		await click(button(accessRow(), "Bought"));
		packages = [bought];
		await click(button(host, "Refresh"));
		await until(() => owned === 2, "the library refetch");
		await flush();
		expect(button(accessRow(), "Bought").getAttribute("aria-pressed")).toBe(
			"true",
		);
		await click(button(accessRow(), "Any access"));
		expect(host.querySelector('fieldset[aria-label="Filter by access"]')).toBe(
			null,
		);
	});

	test("Clear widget permissions shows only for packages with stored grants", async () => {
		const { grantMicroWidgetConsent } = await import(
			"../../a2ui/micro-widget-capability-consent"
		);
		grantMicroWidgetConsent(
			{
				source: "registry:hub.flow-like.com",
				appId: "a",
				packageId: "shared-pkg",
				widgetId: "w",
			},
			{ workers: true },
			"app",
		);
		await mount({
			signedIn: true,
			registry: webRegistry(
				[],
				[
					summary("shared-pkg", { viewerPermission: 4 }),
					summary("plain-pkg", { viewerPermission: 4 }),
				],
			),
		});
		await lookupSettled("plain-pkg");
		for (const [id, expected] of [
			["shared-pkg", true],
			["plain-pkg", false],
		] as const) {
			expect(card(id).textContent).toContain("Shared with you");
			const items = await openMenu(id);
			expect(items.includes("Clear widget permissions on this device")).toBe(
				expected,
			);
			expect(items.some((item) => item.startsWith("Uninstall"))).toBe(false);
			await closeMenu();
		}
	});
});
