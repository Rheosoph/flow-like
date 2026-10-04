import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { createInstance } from "i18next";
import { act } from "react";
import type { Root } from "react-dom/client";
import { I18nextProvider } from "react-i18next";
import de from "../../../../locales/locales/de/store.json";
import en from "../../../../locales/locales/en/store.json";
import {
	PackageWidgetUpdates,
	type PackageWidgetUpdatesProps,
} from "./package-widget-updates";

let window: Window;
let root: Root;
let host: HTMLElement;
let restoreGlobals: () => void;
const onUpdate = mock(() => {});
const onCheck = mock(() => {});

beforeEach(async () => {
	window = new Window();
	const globals = {
		window,
		document: window.document,
		navigator: window.navigator,
		HTMLElement: window.HTMLElement,
		Node: window.Node,
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
	const { createRoot } = await import("react-dom/client");
	host = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(host as never);
	root = createRoot(host);
	onUpdate.mockClear();
	onCheck.mockClear();
});

afterEach(async () => {
	await act(() => root.unmount());
	await window.happyDOM.abort();
	restoreGlobals();
});

async function render(
	props: Partial<PackageWidgetUpdatesProps> = {},
	language = "en",
) {
	const i18n = createInstance();
	await i18n.init({
		lng: language,
		fallbackLng: "en",
		resources: { en: { store: en }, de: { store: de } },
		interpolation: { escapeValue: false },
	});
	await act(() =>
		root.render(
			<I18nextProvider i18n={i18n}>
				<PackageWidgetUpdates
					packageName="Learning kit"
					isChecking={false}
					checkFailed={false}
					isUpdating={false}
					disabled={false}
					onUpdate={onUpdate}
					onCheck={onCheck}
					{...props}
				/>
			</I18nextProvider>,
		),
	);
	const buttons = host.querySelectorAll("button");
	return { check: buttons[0], update: buttons[1] };
}

describe("package widget updates", () => {
	test("shows package-specific stale counts and invokes manual actions", async () => {
		const { check, update } = await render({
			outdated: { widgets: 3, pages: 2 },
		});
		expect(host.textContent).toContain("3 outdated widgets on 2 pages");
		expect(update.getAttribute("aria-label")).toBe(
			"Update all widgets of Learning kit",
		);
		await act(() => update.click());
		await act(() => check.click());
		expect(onUpdate).toHaveBeenCalledTimes(1);
		expect(onCheck).toHaveBeenCalledTimes(1);
	});

	test("offers a release update even when the installed widgets are current", async () => {
		let controls = await render({ outdated: { widgets: 0, pages: 0 } });
		expect(controls.update.disabled).toBe(true);
		expect(controls.check.disabled).toBe(false);
		controls = await render({
			outdated: { widgets: 0, pages: 0 },
			nextVersion: "2.0.0",
		});
		expect(controls.update.disabled).toBe(false);
		expect(host.textContent).toContain("Updates the package to v2.0.0 first.");
	});

	test("allows recovery after a failed check and prevents concurrent updates", async () => {
		let controls = await render({ checkFailed: true });
		expect(host.textContent).toContain("Could not check all pages.");
		expect(controls.check.textContent).toContain("Retry check");
		expect(controls.update.disabled).toBe(false);
		controls = await render({ checkFailed: true, isUpdating: true });
		expect(controls.update.disabled).toBe(true);
		expect(controls.check.disabled).toBe(true);
		controls = await render({ checkFailed: true, disabled: true });
		expect(controls.update.disabled).toBe(true);
		expect(controls.check.disabled).toBe(false);
	});

	test("renders singular and plural German counts", async () => {
		await render({ outdated: { widgets: 1, pages: 1 } }, "de");
		expect(host.textContent).toContain("1 veraltetes Widget auf 1 Seite");
		await render({ outdated: { widgets: 3, pages: 2 } }, "de");
		expect(host.textContent).toContain("3 veraltete Widgets auf 2 Seiten");
	});
});
