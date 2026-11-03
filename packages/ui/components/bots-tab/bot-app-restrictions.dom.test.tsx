import { afterAll, beforeAll, describe, expect, mock, test } from "bun:test";
import { act, createElement } from "react";
import { presetRestrictions } from "./bot-restrictions-model";
import {
	fakeI18n,
	installDomGlobals,
	restoreDomGlobals,
	translate,
	domWindow as window,
} from "./testing/dom-harness";

const actual = { locales: { ...(await import("@flow-like/locales")) } };
mock.module("@flow-like/locales", () => ({
	...actual.locales,
	useTranslation: () => ({ t: translate, i18n: fakeI18n }),
}));

const { createRoot } = await import("react-dom/client");
const { RestrictionsEditor } = await import("./bot-app-restrictions");

let container: HTMLElement;
let root: ReturnType<typeof createRoot>;

beforeAll(() => installDomGlobals());
afterAll(() => {
	mock.module("@flow-like/locales", () => actual.locales);
	restoreDomGlobals();
});

const options = {
	tables: [{ id: "sales", name: "sales" }],
	events: [{ id: "e1", name: "Send report" }],
	boards: [],
};

async function show(onChange: (next: unknown) => void, preset = "read_only") {
	container = window.document.createElement("div");
	window.document.body.appendChild(container);
	root = createRoot(container as unknown as Element);
	await act(async () =>
		root.render(
			createElement(RestrictionsEditor, {
				value: presetRestrictions(preset as never),
				options,
				onChange,
			}),
		),
	);
}
async function hide() {
	await act(async () => root.unmount());
	container.remove();
}

async function openAdvanced() {
	const trigger = [...container.querySelectorAll("button")].find((button) =>
		button.textContent?.includes("Advanced"),
	);
	await act(async () => trigger?.click());
}

describe("the restrictions editor", () => {
	test("says restrictions apply in every chat and shows the preset", async () => {
		await show(() => {});
		expect(container.textContent).toContain(
			"Restrictions apply in every chat, including yours",
		);
		expect(container.textContent).toContain("Read only");
		await hide();
	});

	test("a switch narrows or widens the permissions", async () => {
		const onChange = mock((_next: unknown) => {});
		await show(onChange);
		await openAdvanced();
		const toggle = container.querySelector(
			'button[role="switch"][aria-label="Call events"]',
		);
		expect(toggle?.getAttribute("aria-checked")).toBe("false");
		await act(async () => (toggle as HTMLElement).click());
		const next = onChange.mock.calls[0][0] as {
			permissions: string[];
		};
		expect(next.permissions).toContain("execute_events");
		expect(next.permissions).toContain("read_files");
		await hide();
	});

	test("an allow-list starts empty when limited and lists the options", async () => {
		const onChange = mock((_next: unknown) => {});
		await show(onChange, "full");
		await openAdvanced();
		const limit = container.querySelector(
			'button[role="checkbox"][aria-label="Only some tables"]',
		);
		await act(async () => (limit as HTMLElement).click());
		const next = onChange.mock.calls[0][0] as {
			scopes: { tables: string[] };
		};
		expect(next.scopes.tables).toEqual([]);
		await hide();
	});

	test("a folder is added without slashes", async () => {
		const onChange = mock((_next: unknown) => {});
		await show(onChange);
		await openAdvanced();
		const input = container.querySelector(
			'input[aria-label="Add a folder"]',
		) as HTMLInputElement;
		await act(async () => {
			const setter = Object.getOwnPropertyDescriptor(
				window.HTMLInputElement.prototype,
				"value",
			)?.set;
			setter?.call(input, "/reports/2026/");
			input.dispatchEvent(new window.Event("input", { bubbles: true }));
		});
		const add = [...container.querySelectorAll("button")].find(
			(button) => button.textContent === "Add a folder",
		);
		await act(async () => add?.click());
		const next = onChange.mock.calls[0][0] as {
			scopes: { files: { prefixes: string[] } };
		};
		expect(next.scopes.files.prefixes).toEqual(["reports/2026"]);
		await hide();
	});
});
