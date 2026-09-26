import { afterAll, beforeAll, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act, createElement } from "react";

const window = new Window({ url: "https://localhost" });
Object.assign(window, { SyntaxError, TypeError, Error });

// bun keeps globals and module mocks for every later file in the process, so both are
// captured first and put back in afterAll.
const globalDescriptors = [
	"window",
	"document",
	"navigator",
	"localStorage",
	"HTMLElement",
	"Element",
	"Node",
	"MutationObserver",
	"HTMLButtonElement",
	"SVGElement",
	"Event",
	"CustomEvent",
	"MouseEvent",
	"PointerEvent",
	"Blob",
	"getComputedStyle",
	"requestAnimationFrame",
	"cancelAnimationFrame",
	"ResizeObserver",
	"IS_REACT_ACT_ENVIRONMENT",
].map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);

function installDomGlobals() {
	Object.assign(globalThis, {
		window,
		document: window.document,
		navigator: window.navigator,
		localStorage: window.localStorage,
		HTMLElement: window.HTMLElement,
		Element: window.Element,
		Node: window.Node,
		MutationObserver: window.MutationObserver,
		HTMLButtonElement: window.HTMLButtonElement,
		SVGElement: window.SVGElement,
		Event: window.Event,
		CustomEvent: window.CustomEvent,
		MouseEvent: window.MouseEvent,
		PointerEvent: window.PointerEvent,
		Blob: window.Blob,
		getComputedStyle: window.getComputedStyle.bind(window),
		requestAnimationFrame: (cb: FrameRequestCallback) =>
			setTimeout(() => cb(0), 0),
		cancelAnimationFrame: (id: number) => clearTimeout(id),
		ResizeObserver: class {
			observe() {}
			unobserve() {}
			disconnect() {}
		},
	});
}
installDomGlobals();

// @ts-expect-error — react-dom checks this flag before touching the DOM.
globalThis.IS_REACT_ACT_ENVIRONMENT = true;

const actual = {
	locales: { ...(await import("@flow-like/locales")) },
	nextThemes: { ...(await import("next-themes")) },
	sonner: { ...(await import("sonner")) },
	backendState: { ...(await import("../../../state/backend-state")) },
	monacoReact: { ...(await import("@monaco-editor/react")) },
	monacoCodeEditor: { ...(await import("../../ui/monaco-code-editor")) },
	appearanceRail: { ...(await import("./appearance-rail")) },
};

const translate = (_key: string, fallback: string) => fallback;
mock.module("@flow-like/locales", () => ({
	...actual.locales,
	useTranslation: () => ({ t: translate }),
}));
mock.module("next-themes", () => ({
	...actual.nextThemes,
	useTheme: () => ({ resolvedTheme: "dark" }),
}));
mock.module("sonner", () => ({
	...actual.sonner,
	toast: { success: () => {}, error: () => {} },
}));

let storedSheet = "";
const setAppStylesheet = mock(async (_appId: string, css: string) => {
	storedSheet = css;
});
/** One stable object, the way the real backend context hands it out. */
const backend = {
	appState: {
		getAppStylesheet: async () => storedSheet,
		setAppStylesheet,
	},
};
mock.module("../../../state/backend-state", () => ({
	...actual.backendState,
	useBackend: () => backend,
}));

/** Monaco loads its editor from a CDN; the studio only needs a text surface here. */
mock.module("@monaco-editor/react", () => ({
	...actual.monacoReact,
	default: () => createElement("div"),
}));
mock.module("../../ui/monaco-code-editor", () => ({
	...actual.monacoCodeEditor,
	MonacoCodeEditor: ({
		value,
		onChange,
	}: {
		value: string;
		onChange: (next: string) => void;
	}) =>
		createElement("textarea", {
			"data-testid": "sheet",
			value,
			onChange: (event: { target: { value: string } }) =>
				onChange(event.target.value),
		}),
}));

mock.module("./appearance-rail", () => ({
	...actual.appearanceRail,
	AppearanceRail: ({
		onChange,
		state,
	}: {
		onChange: (next: unknown) => void;
		state: { effects: Record<string, boolean> };
	}) =>
		createElement("button", {
			id: "appearance-fx-aurora",
			type: "button",
			onClick: () =>
				onChange({ ...state, effects: { ...state.effects, aurora: true } }),
		}),
}));

const { createRoot } = await import("react-dom/client");
const { AppearanceStudio } = await import("./appearance-studio");
const { APPEARANCE_END, APPEARANCE_START } = await import(
	"../../../lib/appearance/appearance-theme"
);

beforeAll(() => installDomGlobals());

const roots: ReturnType<typeof createRoot>[] = [];

async function render(): Promise<HTMLElement> {
	const container = window.document.createElement(
		"div",
	) as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	const root = createRoot(container);
	roots.push(root);
	act(() => {
		root.render(createElement(AppearanceStudio, { appId: "app-1" }));
	});
	// The sheet arrives from the backend asynchronously; flush until the editor is up.
	for (let pass = 0; pass < 25; pass += 1) {
		if (container.querySelector("[data-testid=sheet]")) break;
		await new Promise((resolve) => setTimeout(resolve, 0));
		act(() => {});
	}
	return container;
}

afterAll(async () => {
	await act(async () => {
		for (const root of roots) root.unmount();
	});
	mock.module("@flow-like/locales", () => actual.locales);
	mock.module("next-themes", () => actual.nextThemes);
	mock.module("sonner", () => actual.sonner);
	mock.module("../../../state/backend-state", () => actual.backendState);
	mock.module("@monaco-editor/react", () => actual.monacoReact);
	mock.module("../../ui/monaco-code-editor", () => actual.monacoCodeEditor);
	mock.module("./appearance-rail", () => actual.appearanceRail);
	for (const [key, descriptor] of globalDescriptors) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
});

const sheetOf = (container: HTMLElement) =>
	(container.querySelector("[data-testid=sheet]") as HTMLTextAreaElement).value;

describe("AppearanceStudio", () => {
	test("seeds a managed sheet and previews it on the app surfaces", async () => {
		storedSheet = "";
		const container = await render();

		const sheet = sheetOf(container);
		expect(sheet).toContain(APPEARANCE_START);
		expect(sheet).toContain(APPEARANCE_END);
		expect(sheet).toContain("--primary:");

		const preview = container.querySelector('[data-appearance-preview="1"]');
		expect(preview).not.toBeNull();
		expect(preview?.textContent).toContain("Operations");
		// The preview is styled by the sheet itself, scoped the way the runtime scopes it.
		expect(container.querySelector("style")?.textContent).toContain(
			'[data-appearance-preview="1"]',
		);
	});

	test("a control writes into the managed block", async () => {
		storedSheet = "";
		const container = await render();
		expect(sheetOf(container)).not.toContain("@fx aurora");

		const aurora = container.querySelector(
			"#appearance-fx-aurora",
		) as HTMLElement;
		expect(aurora).not.toBeNull();
		act(() => {
			aurora.click();
		});

		expect(sheetOf(container)).toContain("@fx aurora");
		expect(sheetOf(container)).toContain("@keyframes fl-appearance-aurora");
	});

	test("an existing sheet without markers is kept below the block", async () => {
		storedSheet = ".mine {\n  color: red;\n}";
		const container = await render();
		const sheet = sheetOf(container);
		expect(sheet.indexOf(".mine")).toBeGreaterThan(
			sheet.indexOf(APPEARANCE_END),
		);
	});
});
