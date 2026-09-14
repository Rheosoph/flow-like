import { afterEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import {
	type NativeWidgetPageDefinition,
	saveNativeWidgetDefinitions,
} from "../../lib/native-widget";
import {
	NATIVE_WIDGET_PAGE_CAPTURE,
	type NativeWidgetPageCaptureDetail,
} from "../../lib/native-widget-page";
import type { IPage } from "../../state/backend-state/page-state";
import { DataProvider } from "../a2ui/DataContext";
import type { Surface } from "../a2ui/types";
import {
	NativeWidgetPageCapture,
	matchingNativeWidgetPages,
} from "./native-widget-page-capture";

const definition = (id = "widget"): NativeWidgetPageDefinition => ({
	id,
	kind: "page",
	appId: "app",
	path: "/orders",
	queryParams: [],
	title: "Orders",
	accent: "orange",
	refreshMinutes: 30,
	updatedAt: "2026-09-13T10:00:00.000Z",
});
const page: IPage = {
	id: "page",
	name: "Orders",
	layoutType: "stack",
	content: [],
	components: [],
	createdAt: "2026-09-01",
	updatedAt: "2026-09-13",
	onLoadEventId: "load",
};
const surface: Surface = {
	id: "page",
	rootComponentId: "root",
	components: {
		root: {
			id: "root",
			component: { id: "root", type: "text", content: { path: "value" } },
		},
	},
};
let cleanup: (() => Promise<void>) | undefined;
afterEach(async () => {
	await cleanup?.();
	cleanup = undefined;
});

async function setup() {
	const browser = new Window({ url: "https://app.test" });
	const globals = {
		window: browser,
		document: browser.document,
		localStorage: browser.localStorage,
		CustomEvent: browser.CustomEvent,
		HTMLElement: browser.HTMLElement,
		Node: browser.Node,
		navigator: browser.navigator,
		IS_REACT_ACT_ENVIRONMENT: true,
	};
	const previous = Object.fromEntries(
		Object.keys(globals).map((key) => [
			key,
			Object.getOwnPropertyDescriptor(globalThis, key),
		]),
	);
	Object.assign(globalThis, globals);
	const nativeSetTimeout = browser.setTimeout.bind(browser);
	const nativeClearTimeout = browser.clearTimeout.bind(browser);
	type TimerHandle = ReturnType<typeof browser.setTimeout>;
	const timers = new Map<TimerHandle, () => void>();
	browser.setTimeout = (callback, _delay, ...args) => {
		const id = nativeSetTimeout(() => {}, 60_000);
		nativeClearTimeout(id);
		timers.set(id, () => callback(...args));
		return id;
	};
	browser.clearTimeout = (id) => {
		timers.delete(id);
	};
	const captures: NativeWidgetPageCaptureDetail[] = [];
	browser.addEventListener(NATIVE_WIDGET_PAGE_CAPTURE, (event) =>
		captures.push(
			(event as unknown as CustomEvent<NativeWidgetPageCaptureDetail>).detail,
		),
	);
	const host = browser.document.createElement("div");
	browser.document.body.appendChild(host);
	let mounted: Root;
	await act(async () => {
		mounted = createRoot(host as unknown as HTMLElement);
	});
	cleanup = async () => {
		await act(async () => mounted.unmount());
		await browser.happyDOM.close();
		for (const [key, descriptor] of Object.entries(previous)) {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	};
	const render = async ({
		scope = "account-a",
		ready = true,
		value = "Current",
		search = "",
	} = {}) => {
		await act(async () => {
			mounted.render(
				<DataProvider initialData={[{ path: "value", value }]}>
					<NativeWidgetPageCapture
						scope={scope}
						appId="app"
						page={page}
						surface={surface}
						path="/orders"
						search={search}
						pageRevision="content-authority-v2"
						ready={ready}
					/>
				</DataProvider>,
			);
		});
	};
	const flush = async () => {
		await act(async () => {
			const callbacks = [...timers.values()];
			timers.clear();
			for (const callback of callbacks) callback();
		});
	};
	return { browser, captures, render, flush, timers };
}

describe("configured native page capture", () => {
	test("matches the exact account, app, path and encoded query values", async () => {
		await setup();
		saveNativeWidgetDefinitions("account-a", [
			{
				...definition(),
				queryParams: [
					{ name: "search", value: "café & 50%" },
					{ name: "tag", value: "a+b" },
					{ name: "tag", value: "second" },
				],
			},
		]);
		expect(
			matchingNativeWidgetPages(
				"account-a",
				"app",
				"/orders/",
				"tag=a%2Bb&search=caf%C3%A9+%26+50%25&tag=second",
			),
		).toHaveLength(1);
		expect(
			matchingNativeWidgetPages(
				"account-a",
				"app",
				"/orders",
				"tag=second&search=caf%C3%A9+%26+50%25&tag=a%2Bb",
			),
		).toEqual([]);
		expect(
			matchingNativeWidgetPages("account-b", "app", "/orders", "tag=a%2Bb"),
		).toEqual([]);
		expect(
			matchingNativeWidgetPages("account-a", "other", "/orders", "tag=a%2Bb"),
		).toEqual([]);
		expect(
			matchingNativeWidgetPages(
				"account-a",
				"app",
				"/orders",
				"search=caf%C3%A9+%26+50%25&tag=a+b&tag=second",
			),
		).toEqual([]);
	});
	test("waits for the active page load and captures the resolved live values only", async () => {
		const harness = await setup();
		saveNativeWidgetDefinitions("account-a", [definition()]);
		await harness.render({ ready: false, value: "Loading" });
		await harness.flush();
		expect(harness.captures).toEqual([]);
		await harness.render({ value: "Loaded" });
		await harness.flush();
		expect(harness.captures).toHaveLength(1);
		expect(harness.captures[0]).toMatchObject({
			scope: "account-a",
			definitionId: "widget",
			revision: definition().updatedAt,
			pageId: "page",
			pageRevision: "content-authority-v2",
			result: { root: { kind: "text", text: "Loaded" } },
		});
		expect(JSON.stringify(harness.captures[0])).not.toContain("dataModel");
	});
	test("cancels pending snapshots when the account changes or the page closes", async () => {
		const harness = await setup();
		saveNativeWidgetDefinitions("account-a", [definition("a")]);
		saveNativeWidgetDefinitions("account-b", [definition("b")]);
		await harness.render({ value: "A secret" });
		await harness.render({ scope: "account-b", value: "B value" });
		await harness.flush();
		expect(harness.captures.map((capture) => capture.definitionId)).toEqual([
			"b",
		]);
		expect(harness.captures[0].result.root?.text).toBe("B value");
		await harness.render({ scope: "account-b", value: "Pending" });
		await harness.render({
			scope: "account-b",
			ready: false,
			value: "Pending",
		});
		await harness.flush();
		expect(harness.captures).toHaveLength(1);
	});
	test("does not capture unconfigured pages and resumes after visibility changes", async () => {
		const harness = await setup();
		await harness.render();
		await harness.flush();
		expect(harness.captures).toEqual([]);
		Object.defineProperty(harness.browser.document, "visibilityState", {
			configurable: true,
			value: "hidden",
		});
		await act(async () =>
			saveNativeWidgetDefinitions("account-a", [definition()]),
		);
		await harness.flush();
		expect(harness.captures).toEqual([]);
		Object.defineProperty(harness.browser.document, "visibilityState", {
			configurable: true,
			value: "visible",
		});
		await act(async () =>
			harness.browser.document.dispatchEvent(
				new harness.browser.Event("visibilitychange"),
			),
		);
		await harness.flush();
		expect(harness.captures).toHaveLength(1);
	});
});
