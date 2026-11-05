import { Window } from "happy-dom";
import { type ReactNode, act } from "react";
import type { IBackendState } from "../../../state/backend-state";
import type { IBot } from "../../../state/backend-state/bot-state";

const KEY = { kty: "OKP", crv: "Ed25519", x: "AAAA" };

export function deviceRow(
	id: string,
	overrides: Record<string, unknown> = {},
): Record<string, unknown> {
	return {
		device_id: id,
		owner_id: "me",
		name: id,
		status: "active",
		registered_at: 1,
		last_seen_at: null,
		auth_epoch: 0,
		identity: {
			auth_key: KEY,
			telemetry_key: KEY,
			management_key: new Array(32).fill(0),
		},
		relationship: "owner",
		...overrides,
	};
}

export function botOf(id: string, overrides: Partial<IBot> = {}): IBot {
	return {
		id,
		owner_id: "me",
		system: null,
		preset_id: null,
		name: id,
		description: null,
		instructions: null,
		appearance: { kind: "mochi" },
		model: { primary: null, fallbacks: [] },
		classification: { pii: false },
		capabilities: [],
		profile_id: null,
		location: "device",
		device_id: null,
		created_at: "",
		updated_at: "",
		...overrides,
	} as IBot;
}

export function backendWith(
	devices: unknown[],
	botState: Record<string, unknown> | undefined = undefined,
): IBackendState {
	return {
		botState,
		apiState: { get: async () => devices },
		userState: {
			getSettingsProfile: async () => ({
				hub_profile: { id: "profile_1", bits: [] },
			}),
		},
	} as unknown as IBackendState;
}

async function settle() {
	for (let attempt = 0; attempt < 5; attempt++)
		await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
}

const WINDOW_GLOBALS = [
	"Node",
	"NodeFilter",
	"Element",
	"HTMLElement",
	"HTMLButtonElement",
	"HTMLInputElement",
	"HTMLIFrameElement",
	"HTMLAnchorElement",
	"SVGElement",
	"Text",
	"Document",
	"DocumentFragment",
	"Event",
	"CustomEvent",
	"InputEvent",
	"KeyboardEvent",
	"FocusEvent",
	"MouseEvent",
	"PointerEvent",
	"MutationObserver",
	"ResizeObserver",
] as const;

/**
 * Mounts `node` over a fake backend and a query client, on a fresh happy-dom window that is put on
 * `globalThis` before react-dom loads. `unmount` removes it and restores the old globals.
 */
export async function mountWithBackend(
	node: ReactNode,
	backend: IBackendState,
	options: { route?: boolean } = {},
) {
	const window = new Window();
	Object.assign(window, { SyntaxError, TypeError });
	const globals: Record<string, unknown> = {
		window,
		document: window.document,
		navigator: window.navigator,
		IS_REACT_ACT_ENVIRONMENT: true,
		getComputedStyle: window.getComputedStyle.bind(window),
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
	};
	for (const key of WINDOW_GLOBALS)
		globals[key] = (window as unknown as Record<string, unknown>)[key];
	const saved = Object.keys(globals).map(
		(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
	);
	Object.assign(globalThis, globals);
	const { createI18n } = await import("@flow-like/locales");
	createI18n({ language: "en" });
	const { createRoot } = await import("react-dom/client");
	const { QueryClient, QueryClientProvider } = await import(
		"@tanstack/react-query"
	);
	const { BackendContext } = await import("../../../state/backend-state");
	const { MemoryDevicesRoute } = await import(
		"../../settings/devices/routing/use-devices-route"
	);
	const client = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	const container = window.document.createElement("div");
	window.document.body.append(container);
	const root = createRoot(container as unknown as HTMLElement);
	const body = options.route ? (
		<MemoryDevicesRoute>{node}</MemoryDevicesRoute>
	) : (
		node
	);
	await act(async () =>
		root.render(
			<QueryClientProvider client={client}>
				<BackendContext.Provider value={backend}>
					{body}
				</BackendContext.Provider>
			</QueryClientProvider>,
		),
	);
	await settle();
	return {
		container: container as unknown as HTMLElement,
		unmount: async () => {
			await act(async () => root.unmount());
			client.clear();
			await window.happyDOM.close();
			for (const [key, descriptor] of saved) {
				if (descriptor) Object.defineProperty(globalThis, key, descriptor);
				else Reflect.deleteProperty(globalThis, key);
			}
		},
	};
}
