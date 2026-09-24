import { afterAll, beforeAll, describe, expect, mock, test } from "bun:test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import audit from "@flow-like/locales/locales/en/audit.json";
import { Window } from "happy-dom";
import { act, createElement } from "react";
import type { IGenericCommand } from "../../../lib";
import { ILayerType } from "../../../lib/schema/flow/board";
import { ICommandType } from "../../../lib/schema/flow/board/commands/generic-command";

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
	"NodeFilter",
	"HTMLInputElement",
	"HTMLTextAreaElement",
	"HTMLButtonElement",
	"SVGElement",
	"Event",
	"CustomEvent",
	"FocusEvent",
	"KeyboardEvent",
	"MouseEvent",
	"PointerEvent",
	"File",
	"getComputedStyle",
	"requestAnimationFrame",
	"cancelAnimationFrame",
	"ResizeObserver",
	"IS_REACT_ACT_ENVIRONMENT",
].map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
const actualLocales = { ...(await import("@flow-like/locales")) };

/**
 * Every DOM global Radix and React reach for. Installed in `beforeAll` rather
 * than at load: other test files assign their own window while loading, and the
 * last assignment before the test body runs is the one that counts.
 */
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
		NodeFilter: window.NodeFilter,
		HTMLInputElement: window.HTMLInputElement,
		HTMLTextAreaElement: window.HTMLTextAreaElement,
		HTMLButtonElement: window.HTMLButtonElement,
		SVGElement: window.SVGElement,
		Event: window.Event,
		CustomEvent: window.CustomEvent,
		FocusEvent: window.FocusEvent,
		KeyboardEvent: window.KeyboardEvent,
		MouseEvent: window.MouseEvent,
		PointerEvent: window.PointerEvent,
		File: window.File,
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

const translate = (
	_key: string,
	fallback: string | Record<string, unknown>,
	opts?: Record<string, unknown>,
) => {
	const options =
		typeof fallback === "object"
			? fallback
			: (opts ?? ({} as Record<string, unknown>));
	const template =
		typeof fallback === "string"
			? fallback
			: ((options.count === 1
					? options.defaultValue_one
					: (options.defaultValue_other ?? options.defaultValue)) as string);
	return (template ?? "").replace(/\{\{(\w+)\}\}/g, (_, key: string) =>
		String(options[key] ?? ""),
	);
};
const fakeI18n = {
	t: translate,
	language: "en",
	changeLanguage: async () => {},
	on: () => {},
	off: () => {},
};
mock.module("@flow-like/locales", () => ({
	...actualLocales,
	useTranslation: () => ({ t: translate, i18n: fakeI18n }),
	Trans: ({ children }: { children?: unknown }) => children ?? null,
	i18n: fakeI18n,
	getI18n: () => fakeI18n,
	createI18n: () => fakeI18n,
	I18nProvider: ({ children }: { children?: unknown }) => children ?? null,
	useLanguage: () => ({ language: "en", setLanguage: () => {} }),
	LANGUAGES: ["en"],
	NAMESPACES: ["common"],
	DEFAULT_NAMESPACE: "common",
	SOURCE_LANGUAGE: "en",
	LOCALE_CONFIG: {},
	// audit-sentence.ts reads these at import time. Read from the JSON, since
	// other files' mocks of this module may already be in place.
	SOURCE_RESOURCES: { audit },
	LANGUAGE_STORAGE_KEY: "language",
	listLanguages: () => [],
	describeLanguage: () => undefined,
	isRtl: () => false,
}));

// react-dom probes the DOM once at load (e.g. whether `input` events exist), so
// it must be imported after the globals above are in place.
const { createRoot } = await import("react-dom/client");
const { QueryClient, QueryClientProvider } = await import(
	"@tanstack/react-query"
);
const { bpmnCatalogFixture } = await import(
	"../../../lib/importer/fixtures/catalog"
);
const { useBackendStore } = await import("../../../state/backend-state");
const { ImportFlowDialog } = await import("./import-flow-dialog");

const previousBackend = useBackendStore.getState().backend;

beforeAll(() => {
	installDomGlobals();
	useBackendStore.getState().setBackend({
		boardState: { getCatalog: async () => bpmnCatalogFixture() },
	} as never);
});

const roots: ReturnType<typeof createRoot>[] = [];
const client = new QueryClient();

function render(element: React.ReactElement): HTMLElement {
	const container = window.document.createElement(
		"div",
	) as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	const root = createRoot(container);
	roots.push(root);
	act(() =>
		root.render(createElement(QueryClientProvider, { client }, element)),
	);
	return container;
}

afterAll(async () => {
	await act(async () => {
		for (const root of roots) root.unmount();
	});
	useBackendStore.setState({ backend: previousBackend });
	// Radix's FocusScope dispatches its unmount event on a timer, with this window's globals.
	await new Promise((resolve) => setTimeout(resolve, 0));
	mock.restore();
	mock.module("@flow-like/locales", () => actualLocales);
	for (const [key, descriptor] of globalDescriptors) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
});

const bpmn = readFileSync(
	resolve(
		import.meta.dir,
		"../../../lib/importer/fixtures/bpmn/simple-linear.bpmn",
	),
	"utf-8",
);

/**
 * Drops a file on the intake zone. This is the real path a user takes, and it
 * avoids react-dom's change-event plugin, which behaves differently depending
 * on whether react-dom was loaded before this file installed its DOM globals.
 */
async function dropFile(zone: HTMLElement, name: string, content: string) {
	const event = new window.Event("drop", { bubbles: true, cancelable: true });
	Object.defineProperty(event, "dataTransfer", {
		value: { files: [new window.File([content], name, { type: "text/xml" })] },
	});
	await act(async () => {
		zone.dispatchEvent(event as never);
		// The file is read asynchronously, and the preview is computed from a
		// deferred copy of the text, so both land on later ticks.
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
}

describe("ImportFlowDialog", () => {
	test("pastes a BPMN file, previews it and imports it as one batch into a new module", async () => {
		const batches: IGenericCommand[][] = [];
		let imported: string | null | undefined;
		const executeCommands = mock(async (commands: IGenericCommand[]) => {
			batches.push(commands);
			return commands.map((command) =>
				command.command_type === ICommandType.CopyPaste
					? {
							...command,
							new_layers: [{ id: "minted-module", type: ILayerType.Module }],
						}
					: command,
			);
		});

		render(
			createElement(ImportFlowDialog, {
				open: true,
				onOpenChange: () => {},
				appId: "app",
				board: undefined,
				currentFileId: "main",
				reservedRoots: ["function"],
				executeCommands,
				onImported: (id: string | null) => {
					imported = id;
				},
			}),
		);

		const body = window.document.body as unknown as HTMLElement;
		expect(body.textContent).toContain("BPMN 2.0");
		expect(body.textContent).toContain("n8n");

		// The innermost match is the intake zone; its ancestors only stop the
		// browser from opening a file dropped beside it.
		const zone = Array.from(body.querySelectorAll("div"))
			.filter((element) => element.textContent?.includes("Or drop a file here"))
			.pop() as HTMLElement;
		expect(zone).toBeDefined();
		await dropFile(zone, "simple-linear.bpmn", bpmn);

		expect(body.textContent).toContain("Simple Linear");
		expect(body.textContent).toContain("4 elements");
		expect(body.textContent).toContain("2 to model");

		// The module name is suggested from the file name.
		const nameInput = body.querySelector(
			"input[type='text'], input:not([type])",
		) as HTMLInputElement;
		expect(nameInput?.value).toBe("simple linear");

		const importButton = Array.from(body.querySelectorAll("button")).find(
			(b) => b.textContent?.trim() === "Import",
		) as HTMLButtonElement;
		expect(importButton.disabled).toBe(false);
		await act(async () => {
			importButton.click();
			await Promise.resolve();
		});

		expect(batches).toHaveLength(1);
		const last = batches[0][batches[0].length - 1] as IGenericCommand & {
			original_layers: Array<{ type: ILayerType; name: string }>;
		};
		expect(last.command_type).toBe(ICommandType.CopyPaste);
		expect(
			last.original_layers.find((l) => l.type === ILayerType.Module)?.name,
		).toBe("simple linear");
		expect(imported).toBe("minted-module");
	});
});
