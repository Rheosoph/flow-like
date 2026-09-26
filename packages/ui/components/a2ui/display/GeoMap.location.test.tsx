import { afterAll, afterEach, beforeEach, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { type ComponentProps, type ReactNode, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import type { LocationFix } from "../../../lib/location";
import type { MapControls } from "../../ui/map";
import type { GeoMapComponent } from "../types";

const fix: LocationFix = {
	geometry: { type: "Point", coordinates: [13.405, 52.52] },
	latitude: 52.52,
	longitude: 13.405,
	accuracy: 18,
	timestamp: 1720000000000,
	altitude: 30,
	altitudeAccuracy: 5,
	speed: null,
	heading: null,
};
let pathname = "/map";
let runtime = true;
let controls: ComponentProps<typeof MapControls> = {};
const trigger = mock(
	async (
		_event: string,
		_component: unknown,
		_context: unknown,
		_options?: unknown,
	) => {},
);
const action = mock((_action: unknown) => {});
// bun keeps globals and module mocks for every later file in the process, so both are
// captured first and put back in afterAll.
const globalDescriptors = [
	"window",
	"document",
	"navigator",
	"HTMLElement",
	"Element",
	"Node",
	"IS_REACT_ACT_ENVIRONMENT",
].map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
// Radix picks its layout effect when first imported, so the real modules load under a document.
Object.assign(globalThis, { document: new Window().document });
const actual = {
	nextNavigation: { ...(await import("next/navigation")) },
	locales: { ...(await import("@flow-like/locales")) },
	actionHandler: { ...(await import("../ActionHandler")) },
	dataContext: { ...(await import("../DataContext")) },
	styleResolver: { ...(await import("../StyleResolver")) },
	map: { ...(await import("../../ui/map")) },
};
mock.module("next/navigation", () => ({
	...actual.nextNavigation,
	usePathname: () => pathname,
}));
mock.module("@flow-like/locales", () => ({
	...actual.locales,
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));
mock.module("../ActionHandler", () => ({
	...actual.actionHandler,
	useActionContext: () => ({ appId: "app", isPreviewMode: runtime }),
	useComponentEventTrigger: () => trigger,
}));
mock.module("../DataContext", () => ({
	...actual.dataContext,
	useData: () => ({
		resolve: (value: Record<string, unknown>) =>
			value.literalBool ?? value.literalString ?? value.literalNumber,
	}),
}));
mock.module("../StyleResolver", () => ({
	...actual.styleResolver,
	resolveStyle: () => "",
	resolveInlineStyle: () => ({}),
}));
mock.module("../../ui/map", () => ({
	...actual.map,
	Map: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
	MapControls: (props: ComponentProps<typeof MapControls>) => {
		controls = props;
		return props.showLocate ? (
			<button
				type="button"
				disabled={!props.locateEnabled}
				onClick={() => void props.onLocationFix?.(fix)}
			>
				Locate
			</button>
		) : null;
	},
	MapMarker: () => null,
	MapPopup: () => null,
	MapRoute: () => null,
	MarkerContent: () => null,
	MarkerLabel: () => null,
}));

let browser: Window;
let root: Root;
const defaults: GeoMapComponent = {
	id: "map",
	type: "geoMap",
	showLocate: { literalBool: true },
};
beforeEach(() => {
	browser = new Window({ url: "https://flow-like.test/map" });
	Object.assign(browser, { SyntaxError, TypeError });
	Object.assign(globalThis, {
		window: browser,
		document: browser.document,
		navigator: browser.navigator,
		HTMLElement: browser.HTMLElement,
		Element: browser.Element,
		Node: browser.Node,
		IS_REACT_ACT_ENVIRONMENT: true,
	});
	const container = browser.document.createElement("div");
	browser.document.body.appendChild(container);
	root = createRoot(container as unknown as Element);
	pathname = "/map";
	runtime = true;
	trigger.mockClear();
	action.mockClear();
});
afterEach(async () => {
	await act(async () => root.unmount());
	await browser.happyDOM.close();
});
afterAll(() => {
	for (const [key, descriptor] of globalDescriptors) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
	mock.restore();
	mock.module("next/navigation", () => actual.nextNavigation);
	mock.module("@flow-like/locales", () => actual.locales);
	mock.module("../ActionHandler", () => actual.actionHandler);
	mock.module("../DataContext", () => actual.dataContext);
	mock.module("../StyleResolver", () => actual.styleResolver);
	mock.module("../../ui/map", () => actual.map);
});
async function render(component = defaults) {
	const modulePath = "./GeoMap.tsx?location-events-test";
	const { A2UIGeoMap } = await import(modulePath);
	await act(async () =>
		root.render(
			<A2UIGeoMap
				component={component}
				componentId="map"
				surfaceId="page"
				renderChild={() => null}
				onAction={action}
			/>,
		),
	);
}

test("locate preserves legacy coordinates and adds the Geometry Point and full fix", async () => {
	await render();
	expect(trigger).not.toHaveBeenCalled();
	await act(async () =>
		browser.document
			.querySelector("button")
			?.dispatchEvent(new browser.MouseEvent("click", { bubbles: true })),
	);
	const [event, , context] = trigger.mock.calls[0] ?? [];
	expect(event).toBe("locate");
	expect(context).toEqual({
		event: "locate",
		coordinate: { longitude: 13.405, latitude: 52.52 },
		geometry: fix.geometry,
		location: fix,
	});
	expect(action.mock.calls[0]?.[0]).toMatchObject({
		name: "locate",
		sourceComponentId: "map",
		surfaceId: "page",
		context: { geometry: fix.geometry },
	});
});

test("location errors require an exact Event binding and retain structured details", async () => {
	await render();
	await act(async () =>
		controls.onLocateError?.({
			code: "permission_denied",
			message: "Allow location access",
		}),
	);
	expect(trigger.mock.calls[0]).toEqual([
		"locateError",
		defaults,
		{
			event: "locateError",
			code: "permission_denied",
			message: "Allow location access",
			error: { code: "permission_denied", message: "Allow location access" },
		},
		{ legacyFallback: false, wildcardFallback: false },
	]);
});

test("Locate is opt-in, disabled in the editor, and scoped to the current route", async () => {
	await render({ ...defaults, showLocate: { literalBool: false } });
	expect(browser.document.querySelector("button")).toBeNull();
	runtime = false;
	await render();
	expect(browser.document.querySelector("button")?.disabled).toBe(true);
	runtime = true;
	await render();
	const previous = controls.locationScope;
	expect(controls.locationAppId).toBe("app");
	pathname = "/another-page";
	await render();
	expect(controls.locationScope).not.toBe(previous);
});
