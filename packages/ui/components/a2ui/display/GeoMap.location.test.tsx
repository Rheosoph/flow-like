import { afterAll, afterEach, beforeEach, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { GeoMapComponent } from "../types";
import type { LocationFix } from "../../../lib/location";

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
let controls: any;
const trigger = mock(
	async (
		_event: string,
		_component: unknown,
		_context: unknown,
		_options?: unknown,
	) => {},
);
const action = mock((_action: unknown) => {});
mock.module("next/navigation", () => ({ usePathname: () => pathname }));
mock.module("@flow-like/locales", () => ({
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));
mock.module("../ActionHandler", () => ({
	useActionContext: () => ({ appId: "app", isPreviewMode: runtime }),
	useComponentEventTrigger: () => trigger,
}));
mock.module("../DataContext", () => ({
	useData: () => ({
		resolve: (value: any) =>
			value.literalBool ?? value.literalString ?? value.literalNumber,
	}),
}));
mock.module("../StyleResolver", () => ({
	resolveStyle: () => "",
	resolveInlineStyle: () => ({}),
}));
mock.module("../../ui/map", () => ({
	Map: ({ children }: any) => <div>{children}</div>,
	MapControls: (props: any) => {
		controls = props;
		return props.showLocate ? (
			<button
				type="button"
				disabled={!props.locateEnabled}
				onClick={() => void props.onLocationFix(fix)}
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
afterAll(() => mock.restore());
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
	const [event, , context] = trigger.mock.calls[0]!;
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
		controls.onLocateError({
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
