import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import type { Root } from "react-dom/client";
import type { WidgetPolicy } from "../a2ui/micro-widget-policy";

/** The first dynamic import transpiles the backend-state graph, which can take seconds on a cold cache. */
const COLD_IMPORT_TIMEOUT_MS = 30_000;

let window: Window;
let root: Root;
let host: HTMLElement;
let restoreGlobals: () => void;

beforeEach(async () => {
	window = new Window({ url: "https://app.flow-like.com/library/config" });
	const globals = {
		window,
		document: window.document,
		navigator: window.navigator,
		localStorage: window.localStorage,
		HTMLElement: window.HTMLElement,
		HTMLButtonElement: window.HTMLButtonElement,
		Element: window.Element,
		Text: window.Text,
		DocumentFragment: window.DocumentFragment,
		Node: window.Node,
		MutationObserver: window.MutationObserver,
		Event: window.Event,
		MouseEvent: window.MouseEvent,
		PointerEvent: window.PointerEvent,
		getComputedStyle: window.getComputedStyle.bind(window),
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
		"../a2ui/micro-widget-capability-consent"
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

const MAP_POLICY: WidgetPolicy = {
	workers: true,
	csp: { connectSrc: ["https://api.maptiler.com"] },
};

function target(appId: string, widgetId: string) {
	return {
		source: "registry:hub.flow-like.com",
		appId,
		packageId: "maps",
		widgetId,
	};
}

async function renderSheetList(
	appId: string,
	registry: { revokeWidgetGrants: (p: string, w?: string) => Promise<void> },
) {
	const { act } = await import("react");
	const {
		WidgetPermissionsList,
		revokeWidgetPermissions,
		useMicroWidgetConsentEntries,
	} = await import("./widget-permissions");

	function ProjectPermissions() {
		const entries = useMicroWidgetConsentEntries({ appId });
		return (
			<WidgetPermissionsList
				entries={entries}
				onRevoke={(entry) => {
					void revokeWidgetPermissions([entry], registry);
				}}
			/>
		);
	}

	await act(async () => root.render(<ProjectPermissions />));
}

function listed(): string[] {
	return Array.from(host.querySelectorAll("[data-widget-permission]")).map(
		(item) => item.querySelector("p")?.textContent ?? "",
	);
}

describe("WidgetPermissionsList markup", () => {
	test(
		"lists each grant with its package, origin, scope, capabilities and sites",
		async () => {
			const { renderToStaticMarkup } = await import("react-dom/server");
			const { grantMicroWidgetConsent } = await import(
				"../a2ui/micro-widget-capability-consent"
			);
			const { WidgetPermissionsList, listWidgetConsentEntries } = await import(
				"./widget-permissions"
			);
			grantMicroWidgetConsent(
				target("app-1", "live-map"),
				{
					...MAP_POLICY,
					csp: {
						...MAP_POLICY.csp,
						imgSrc: ["https://a.tile.openstreetmap.org"],
					},
				},
				"app",
			);
			grantMicroWidgetConsent(
				{ ...target("app-1", "chart"), packageId: "charts" },
				{ wasm: true },
				"session",
			);

			const markup = renderToStaticMarkup(
				<WidgetPermissionsList
					entries={listWidgetConsentEntries({ appId: "app-1" })}
					packageNames={new Map([["maps", "Maps"]])}
					onRevoke={() => {}}
				/>,
			);
			expect(markup.match(/data-widget-permission=/g)).toHaveLength(2);
			expect(markup).toContain("Maps");
			expect(markup).toContain(">charts</bdi>");
			expect(markup).toContain("Registry hub.flow-like.com");
			expect(markup).toContain("Always allowed for this project");
			expect(markup).toContain("Allowed for this session");
			expect(markup).toContain("Background workers");
			expect(markup).toContain("WebAssembly");
			expect(markup).toContain("https://api.maptiler.com");
			expect(markup).toContain("Load images");
			expect(markup.match(/>Revoke</g)).toHaveLength(2);
		},
		COLD_IMPORT_TIMEOUT_MS,
	);

	test(
		"a long right-to-left package name can neither hide nor reorder the origin",
		async () => {
			const { act } = await import("react");
			const { grantMicroWidgetConsent } = await import(
				"../a2ui/micro-widget-capability-consent"
			);
			const { WidgetPermissionsList, listWidgetConsentEntries } = await import(
				"./widget-permissions"
			);
			grantMicroWidgetConsent(target("app-1", "live-map"), MAP_POLICY, "app");
			const name = `${"חבילת מפות רשמית ".repeat(12)}‮`;

			await act(async () =>
				root.render(
					<WidgetPermissionsList
						entries={listWidgetConsentEntries({ appId: "app-1" })}
						packageNames={new Map([["maps", name]])}
						onRevoke={() => {}}
					/>,
				),
			);

			const origin = host.querySelector("[data-widget-permission-origin]");
			expect(origin?.textContent).toBe("Registry hub.flow-like.com");
			expect(origin?.className).not.toContain("truncate");
			expect(origin?.closest(".truncate")).toBe(null);
			const isolated = origin?.parentElement?.querySelector("bdi");
			expect(isolated?.textContent).toBe(name);
			expect(isolated?.className).toContain("truncate");
			expect(isolated?.contains(origin ?? null)).toBe(false);
		},
		COLD_IMPORT_TIMEOUT_MS,
	);

	test(
		"explains when nothing is stored",
		async () => {
			const { renderToStaticMarkup } = await import("react-dom/server");
			const { WidgetPermissionsList } = await import("./widget-permissions");
			const markup = renderToStaticMarkup(
				<WidgetPermissionsList entries={[]} onRevoke={() => {}} />,
			);
			expect(markup).toContain("data-widget-permissions-empty");
			expect(markup).toContain(
				"No widget permissions are stored for this project on this device.",
			);
		},
		COLD_IMPORT_TIMEOUT_MS,
	);
});

describe("runtime addresses and stop asking", () => {
	test(
		"shows levels, runtime addresses with their age, and Ask again clears Not asking",
		async () => {
			const { act } = await import("react");
			const {
				evaluateMicroWidgetConsent,
				grantMicroWidgetConsent,
				muteMicroWidgetRuntime,
			} = await import("../a2ui/micro-widget-capability-consent");
			const mounted = target("app-1", "live-map");
			grantMicroWidgetConsent(
				mounted,
				{
					policy: MAP_POLICY,
					levels: { "https://api.maptiler.com": "known" },
					runtime: [
						{
							directive: "imgSrc",
							source: "https://a.tiles.example.com",
							level: "broad",
							slot: "tileUrl",
						},
					],
				},
				"app",
			);
			muteMicroWidgetRuntime(mounted);

			await renderSheetList("app-1", { revokeWidgetGrants: async () => {} });

			const item = host.querySelector("[data-widget-permission]");
			expect(item?.textContent).toContain("Identified service");
			const runtime = item?.querySelector(
				'[data-widget-permission-runtime="https://a.tiles.example.com"]',
			);
			expect(runtime?.querySelector("svg.lucide-radio")).not.toBe(null);
			expect(runtime?.textContent).toContain("Load images");
			expect(runtime?.textContent).toContain("Anyone can receive");
			expect(runtime?.textContent).toMatch(/now|second|minute/);
			expect(item?.textContent).toContain(
				"1 address provided while the app ran",
			);
			expect(
				item?.querySelector("[data-widget-permission-muted]")?.textContent,
			).toContain("Not asking about new addresses");

			const askAgain = item?.querySelector<HTMLButtonElement>(
				"[data-widget-permission-ask-again]",
			);
			await act(async () => {
				askAgain?.click();
			});

			expect(host.querySelector("[data-widget-permission-muted]")).toBe(null);
			expect(listed()).toEqual(["live-map"]);
			expect(evaluateMicroWidgetConsent(mounted, MAP_POLICY).muted).toBe(false);
			expect(
				host.querySelector(
					'[data-widget-permission-runtime="https://a.tiles.example.com"]',
				),
			).not.toBe(null);
		},
		COLD_IMPORT_TIMEOUT_MS,
	);

	test(
		"a record that only says stop asking shows no scope and disappears on Ask again",
		async () => {
			const { act } = await import("react");
			const { muteMicroWidgetRuntime } = await import(
				"../a2ui/micro-widget-capability-consent"
			);
			muteMicroWidgetRuntime(target("app-1", "live-map"));

			await renderSheetList("app-1", { revokeWidgetGrants: async () => {} });

			const item = host.querySelector("[data-widget-permission]");
			expect(item?.textContent).not.toContain("Always allowed");
			expect(item?.textContent).toContain("Not asking about new addresses");
			await act(async () => {
				item
					?.querySelector<HTMLButtonElement>(
						"[data-widget-permission-ask-again]",
					)
					?.click();
			});
			expect(listed()).toEqual([]);
		},
		COLD_IMPORT_TIMEOUT_MS,
	);
});

describe("project widget permissions list", () => {
	test(
		"lists the project's grants and revoking one removes it and its desktop grant",
		async () => {
			const { act } = await import("react");
			const { grantMicroWidgetConsent, readMicroWidgetConsent } = await import(
				"../a2ui/micro-widget-capability-consent"
			);
			grantMicroWidgetConsent(target("app-1", "legend"), MAP_POLICY, "app");
			grantMicroWidgetConsent(target("app-1", "live-map"), MAP_POLICY, "app");
			grantMicroWidgetConsent(target("app-2", "live-map"), MAP_POLICY, "app");
			const registry = {
				revokeWidgetGrants: mock(async (_p: string, _w?: string) => {}),
			};

			await renderSheetList("app-1", registry);
			expect(listed()).toEqual(["legend", "live-map"]);

			const revokeLegend = host.querySelector("button");
			await act(async () => {
				revokeLegend?.click();
			});

			expect(listed()).toEqual(["live-map"]);
			expect(
				readMicroWidgetConsent(target("app-1", "legend"), MAP_POLICY),
			).toBe("pending");
			expect(registry.revokeWidgetGrants.mock.calls).toEqual([
				["maps", "legend"],
			]);
		},
		COLD_IMPORT_TIMEOUT_MS,
	);

	test(
		"follows grants and a project-wide revoke made elsewhere",
		async () => {
			const { act } = await import("react");
			const { grantMicroWidgetConsent, clearMicroWidgetConsents } =
				await import("../a2ui/micro-widget-capability-consent");
			await renderSheetList("app-1", {
				revokeWidgetGrants: async () => {},
			});
			expect(host.querySelector("[data-widget-permissions-empty]")).not.toBe(
				null,
			);

			await act(async () => {
				grantMicroWidgetConsent(target("app-1", "live-map"), MAP_POLICY, "app");
			});
			expect(listed()).toEqual(["live-map"]);

			await act(async () => {
				clearMicroWidgetConsents({ appId: "app-1" });
			});
			expect(listed()).toEqual([]);
			expect(host.querySelector("[data-widget-permissions-empty]")).not.toBe(
				null,
			);
		},
		COLD_IMPORT_TIMEOUT_MS,
	);
});
