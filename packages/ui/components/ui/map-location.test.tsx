import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { LocationFix } from "../../lib/location";
import { useMapLocation } from "./map-location";

const fix: LocationFix = {
	geometry: { type: "Point", coordinates: [13.405, 52.52] },
	latitude: 52.52,
	longitude: 13.405,
	accuracy: 12,
	timestamp: 1720000000000,
	altitude: null,
	altitudeAccuracy: null,
	speed: null,
	heading: null,
};
let browser: Window;
let root: Root;
let api: ReturnType<typeof useMapLocation>;
let resolve: (fix: LocationFix) => void;
let signal: AbortSignal;
const found = mock(async (_fix: LocationFix) => {});
const error = mock(() => {});
const reader = mock(
	async (_appId: string | undefined, requestSignal: AbortSignal) => {
		signal = requestSignal;
		return new Promise<LocationFix>((done) => {
			resolve = done;
		});
	},
);

function Harness({
	scope = "page",
	enabled = true,
}: { scope?: string; enabled?: boolean }) {
	api = useMapLocation({
		appId: "app",
		scope,
		enabled,
		onLocation: found,
		onError: error,
		readLocation: reader,
	});
	return (
		<div ref={api.elementRef}>
			<button type="button" onClick={() => void api.locate()}>
				Locate
			</button>
			{api.error && <span role="alert">{api.error.message}</span>}
		</div>
	);
}
beforeEach(() => {
	browser = new Window({ url: "https://flow-like.test" });
	Object.assign(browser, { SyntaxError, TypeError });
	Object.assign(globalThis, {
		window: browser,
		document: browser.document,
		navigator: browser.navigator,
		HTMLElement: browser.HTMLElement,
		Element: browser.Element,
		Node: browser.Node,
		MutationObserver: browser.MutationObserver,
		getComputedStyle: browser.getComputedStyle.bind(browser),
		IS_REACT_ACT_ENVIRONMENT: true,
	});
	const container = browser.document.createElement("div");
	browser.document.body.appendChild(container);
	root = createRoot(container as unknown as Element);
	found.mockClear();
	error.mockClear();
	reader.mockClear();
});
afterEach(async () => {
	await act(async () => root.unmount());
	await browser.happyDOM.close();
});
async function render(scope = "page", enabled = true) {
	await act(async () =>
		root.render(<Harness scope={scope} enabled={enabled} />),
	);
}
async function click() {
	await act(async () => {
		browser.document
			.querySelector("button")
			?.dispatchEvent(new browser.MouseEvent("click", { bubbles: true }));
	});
}

test("only an enabled user click reads location and duplicate clicks stay in one request", async () => {
	await render();
	expect(reader).not.toHaveBeenCalled();
	await click();
	await click();
	expect(reader).toHaveBeenCalledTimes(1);
	expect(reader.mock.calls[0]?.[0]).toBe("app");
	expect(api.waiting).toBe(true);
	await act(async () => resolve(fix));
	expect(found).toHaveBeenCalledWith(fix);
	expect(api.waiting).toBe(false);
	await render("page", false);
	await click();
	expect(reader).toHaveBeenCalledTimes(1);
});

test("route replacement aborts acquisition and drops the previous screen's late fix", async () => {
	await render();
	await click();
	await render("another-page");
	expect(signal.aborted).toBe(true);
	await act(async () => resolve(fix));
	expect(found).not.toHaveBeenCalled();
	expect(api.waiting).toBe(false);
	expect(error).not.toHaveBeenCalled();
});

test("unmount aborts location and cannot dispatch a late Event", async () => {
	await render();
	await click();
	await act(async () => root.render(null));
	expect(signal.aborted).toBe(true);
	await act(async () => resolve(fix));
	expect(found).not.toHaveBeenCalled();
});

test("background and native location signals abort foreground fixes", async () => {
	await render();
	await click();
	await act(async () =>
		browser.window.dispatchEvent(
			new browser.Event("flow-like:location-background"),
		),
	);
	expect(signal.aborted).toBe(true);
	await act(async () => resolve(fix));
	await click();
	await act(async () => {
		Object.defineProperty(browser.document, "visibilityState", {
			configurable: true,
			value: "hidden",
		});
		browser.document.dispatchEvent(new browser.Event("visibilitychange"));
	});
	expect(signal.aborted).toBe(true);
	await act(async () => resolve(fix));
	expect(found).not.toHaveBeenCalled();
});

test("a permission popup may blur the window without cancelling a visible map", async () => {
	await render();
	await click();
	await act(async () => {
		browser.window.dispatchEvent(new browser.Event("blur"));
		browser.window.dispatchEvent(
			new browser.Event("flow-like:device-inactive"),
		);
	});
	expect(signal.aborted).toBe(false);
	await act(async () => resolve(fix));
	expect(found).toHaveBeenCalledWith(fix);
});

test("a hidden ancestor blocks a fix even before a visibility observer runs", async () => {
	await render();
	await click();
	await act(async () => {
		browser.document.body.hidden = true;
		resolve(fix);
	});
	expect(found).not.toHaveBeenCalled();
	expect(error).not.toHaveBeenCalled();
});

test("permission errors clear waiting state and expose a structured error", async () => {
	reader.mockImplementationOnce(async () => {
		throw Object.assign(new Error("Location permission denied"), {
			code: "permission_denied",
		});
	});
	await render();
	await click();
	expect(api.waiting).toBe(false);
	expect(api.error).toEqual({
		code: "permission_denied",
		message: "Location permission denied",
	});
	expect(error).toHaveBeenCalledWith(api.error);
	expect(
		browser.document.querySelector('[role="alert"]')?.textContent,
	).toContain("denied");
});
