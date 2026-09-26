import { afterAll, afterEach, beforeEach, expect, mock, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Window } from "happy-dom";
import { type ComponentProps, type ReactNode, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import type { GeofencePermissionStatus } from "../../../lib/location";

let status: GeofencePermissionStatus;
let editor: {
	value: unknown;
	onChange: (value: unknown, valid: boolean) => void;
};
const commands = mock(
	async (_command: string, _args: unknown, _context: unknown) => status,
);
const update = mock((_config: unknown) => {});
// bun keeps a module mock for every later file in the process, so each mocked module is
// captured first and put back in afterAll. Radix picks its layout effect when first imported,
// so the real modules load under a document.
const documentDescriptor = Object.getOwnPropertyDescriptor(
	globalThis,
	"document",
);
Object.assign(globalThis, { document: new Window().document });
const actual = {
	deviceBridge: { ...(await import("../../../lib/device-bridge")) },
	backendState: { ...(await import("../../../state/backend-state")) },
	oidc: { ...(await import("react-oidc-context")) },
	geometryEditor: { ...(await import("../../flow/variables/geometry-editor")) },
	button: { ...(await import("../../ui/button")) },
	input: { ...(await import("../../ui/input")) },
	label: { ...(await import("../../ui/label")) },
};
if (documentDescriptor)
	Object.defineProperty(globalThis, "document", documentDescriptor);
else Reflect.deleteProperty(globalThis, "document");
mock.module("../../../lib/device-bridge", () => ({
	...actual.deviceBridge,
	executeDeviceCommand: commands,
}));
mock.module("../../../state/backend-state", () => ({
	...actual.backendState,
	useBackend: () => ({
		profile: { id: "profile", hub: "api.test", secure: true },
	}),
}));
mock.module("react-oidc-context", () => ({
	...actual.oidc,
	useAuth: () => ({
		isAuthenticated: true,
		user: { profile: { sub: "account" } },
	}),
}));
mock.module("../../flow/variables/geometry-editor", () => ({
	...actual.geometryEditor,
	GeometryEditor: (props: typeof editor) => {
		editor = props;
		return <div data-testid="geometry" />;
	},
}));
mock.module("../../ui/button", () => ({
	...actual.button,
	Button: ({
		children,
		variant: _variant,
		size: _size,
		...props
	}: ComponentProps<"button"> & { variant?: string; size?: string }) => (
		<button {...props}>{children}</button>
	),
}));
mock.module("../../ui/input", () => ({
	...actual.input,
	Input: (props: ComponentProps<"input">) => <input {...props} />,
}));
mock.module("../../ui/label", () => ({
	...actual.label,
	Label: ({ children }: { children: ReactNode }) => <label>{children}</label>,
}));

const globals = [
	"window",
	"document",
	"navigator",
	"IS_REACT_ACT_ENVIRONMENT",
] as const;
const saved = globals.map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
let browser: Window;
let root: Root;
let client: QueryClient;
beforeEach(() => {
	browser = new Window({ url: "https://flow-like.test/settings" });
	Object.assign(browser, { SyntaxError, TypeError });
	for (const [key, value] of Object.entries({
		window: browser,
		document: browser.document,
		navigator: browser.navigator,
		IS_REACT_ACT_ENVIRONMENT: true,
	}))
		Object.defineProperty(globalThis, key, {
			configurable: true,
			writable: true,
			value,
		});
	client = new QueryClient({
		defaultOptions: { queries: { retry: false, gcTime: 0 } },
	});
	const container = browser.document.createElement("div");
	browser.document.body.appendChild(container);
	root = createRoot(container as unknown as Element);
	status = {
		authorization: "not_determined",
		accuracyAuthorization: "full",
		backgroundSupported: true,
		backgroundDelivery: "system_monitored",
		monitoredRegionCount: 2,
		maxMonitoredRegions: 20,
	};
	commands.mockClear();
	update.mockClear();
});
afterEach(async () => {
	await act(async () => root.unmount());
	client.clear();
	await browser.happyDOM.close();
	for (const [key, descriptor] of saved) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
});
afterAll(() => {
	mock.restore();
	mock.module("../../../lib/device-bridge", () => actual.deviceBridge);
	mock.module("../../../state/backend-state", () => actual.backendState);
	mock.module("react-oidc-context", () => actual.oidc);
	mock.module(
		"../../flow/variables/geometry-editor",
		() => actual.geometryEditor,
	);
	mock.module("../../ui/button", () => actual.button);
	mock.module("../../ui/input", () => actual.input);
	mock.module("../../ui/label", () => actual.label);
});

const config = {
	sink_type: "geolocation",
	longitude: 13.405,
	latitude: 52.52,
	radius: 200,
	trigger_on: "Both",
	background: false,
};
async function render() {
	const modulePath = "./geolocation.tsx?permission-behavior-test";
	const { GeolocationConfig } = await import(modulePath);
	await act(async () =>
		root.render(
			<QueryClientProvider client={client}>
				<GeolocationConfig
					appId="app"
					boardId="board"
					nodeId="node"
					node={{} as never}
					eventId="event"
					eventExecutionMode="Remote"
					config={config}
					isEditing
					onConfigUpdate={update}
				/>
			</QueryClientProvider>,
		),
	);
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 10));
	});
}
function button(text: string) {
	return Array.from(browser.document.querySelectorAll("button")).find(
		(element) => element.textContent === text,
	)!;
}

test("reading status never requests permission; background permission needs a deliberate click", async () => {
	await render();
	expect(
		commands.mock.calls.every(
			([, args]) => (args as { mode: string }).mode === "status",
		),
	).toBe(true);
	expect(browser.document.body.textContent).toContain(
		"This device monitors the region",
	);
	expect(browser.document.body.textContent).toContain("2 of 20 region slots");
	await act(async () => button("Allow background geofencing").click());
	expect(commands).toHaveBeenCalledWith(
		"location.geofencePermission",
		{ mode: "background" },
		{ appId: "app", executionTarget: "local", userInitiated: true },
	);
});

test("Point editing preserves longitude-first Geometry and the geolocation sink fields", async () => {
	await render();
	expect(editor.value).toEqual({ type: "Point", coordinates: [13.405, 52.52] });
	await act(async () =>
		editor.onChange({ type: "Point", coordinates: [-73.9, 40.7] }, true),
	);
	expect(update).toHaveBeenLastCalledWith({
		...config,
		longitude: -73.9,
		latitude: 40.7,
	});
	const monitor = browser.document.querySelectorAll("select")[1]!;
	await act(async () => {
		monitor.value = "background";
		monitor.dispatchEvent(new browser.Event("change", { bubbles: true }));
	});
	expect(update).toHaveBeenLastCalledWith({ ...config, background: true });
	expect(
		commands.mock.calls.every(
			([, args]) => (args as { mode: string }).mode === "status",
		),
	).toBe(true);
});

test("native limits and monitoring errors remain visible without prompting", async () => {
	status = {
		...status,
		authorization: "when_in_use",
		backgroundDelivery: "app_running",
		monitoredRegionCount: 20,
		error: {
			code: "region_limit",
			message: "A region could not be registered",
		},
	};
	await render();
	expect(browser.document.body.textContent).toContain(
		"All region slots are in use",
	);
	expect(browser.document.body.textContent).toContain(
		"A region could not be registered",
	);
	expect(browser.document.body.textContent).toContain(
		"the computer must be awake",
	);
	expect(button("Allow background geofencing").disabled).toBe(true);
});

test("unsupported platforms show an unavailable state with disabled permission actions", async () => {
	status = {
		...status,
		backgroundSupported: false,
		error: { code: "unsupported", message: "Use an iOS or macOS app" },
	};
	await render();
	expect(browser.document.body.textContent).toContain(
		"Unavailable on this platform",
	);
	expect(button("Allow while using app").disabled).toBe(true);
	expect(button("Allow background geofencing").disabled).toBe(true);
});
