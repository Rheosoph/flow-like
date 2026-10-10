import { afterAll, beforeAll, expect, mock, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { transformSync } from "@babel/core";
import reactCompiler from "babel-plugin-react-compiler";
import { Window } from "happy-dom";
import { act, createRef, useCallback, useMemo, useState } from "react";
import type { ILog } from "../../../lib/schema/flow/log";
import { ILogLevel } from "../../../lib/schema/flow/log";
import type { ILogListHandle, IRowView } from "./log-list";
import { LogPageCache } from "./log-page-cache";
import type { IRowActions } from "./log-row";
import { ROW_HEIGHT } from "./log-row";
import type { ILogWindow } from "./use-log-window";

const window = new Window({ url: "https://localhost" });
Object.assign(window, { SyntaxError, TypeError, Error });
const viewportHeight = ROW_HEIGHT * 10;

// happy-dom has no layout engine. Give the real virtualizer a measured viewport.
Object.defineProperties(window.HTMLElement.prototype, {
	offsetHeight: {
		configurable: true,
		get() {
			return this.getAttribute("role") === "log" ? viewportHeight : 0;
		},
	},
	offsetWidth: { configurable: true, get: () => 800 },
});

const globals = {
	window,
	document: window.document,
	navigator: window.navigator,
	HTMLElement: window.HTMLElement,
	Element: window.Element,
	Node: window.Node,
	MutationObserver: window.MutationObserver,
	CustomEvent: window.CustomEvent,
	getComputedStyle: window.getComputedStyle.bind(window),
	requestAnimationFrame: window.requestAnimationFrame.bind(window),
	cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
	IS_REACT_ACT_ENVIRONMENT: true,
};
const globalDescriptors = Object.keys(globals).map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
Object.assign(globalThis, globals);
beforeAll(() => Object.assign(globalThis, globals));

const actualLocales = { ...(await import("@flow-like/locales")) };
mock.module("@flow-like/locales", () => ({
	...actualLocales,
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));

const { createRoot } = await import("react-dom/client");
const sourcePath = fileURLToPath(new URL("./log-list.tsx", import.meta.url));
// The apps enable React Compiler; a plain TSX import misses its memoization bugs.
const compiled = transformSync(readFileSync(sourcePath, "utf8"), {
	filename: sourcePath,
	parserOpts: { plugins: ["typescript", "jsx"] },
	plugins: [[reactCompiler, { target: "19" }]],
	configFile: false,
	babelrc: false,
});
if (!compiled?.code) throw new Error("Could not compile LogList");
const compiledDirectory = mkdtempSync(
	join(tmpdir(), "flow-like-log-list-test-"),
);
const compiledPath = join(compiledDirectory, "log-list.mjs");
const javascript = new Bun.Transpiler({
	loader: "tsx",
	autoImportJSX: true,
}).transformSync(compiled.code);
writeFileSync(
	compiledPath,
	javascript.replace(
		/from "([^"]+)"/g,
		(_match, specifier: string) =>
			`from ${JSON.stringify(Bun.resolveSync(specifier, import.meta.dir))}`,
	),
);
const { LogList } = (await import(compiledPath)) as typeof import("./log-list");

afterAll(async () => {
	await window.happyDOM.close();
	rmSync(compiledDirectory, { recursive: true, force: true });
	mock.module("@flow-like/locales", () => actualLocales);
	for (const [key, descriptor] of globalDescriptors) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
});

const view: IRowView = {
	selectedKeys: new Set(),
	relative: true,
	base: 0,
	nodeOf: () => undefined,
	nodeName: () => undefined,
	headOf: () => undefined,
};
const actions: IRowActions = {
	toggleSelect: () => {},
	focusRow: () => {},
	scopeNode: () => {},
	setFolded: () => {},
};

function logAt(index: number): ILog {
	return {
		start: { secs_since_epoch: index, nanos_since_epoch: 0 },
		end: { secs_since_epoch: index, nanos_since_epoch: 0 },
		log_level: ILogLevel.Info,
		message: `Log entry ${index}`,
	};
}

test("compiled log list grows its scroll range and loads the last page", async () => {
	const requests: number[] = [];
	const handle = createRef<ILogListHandle>();
	function Harness({ count }: { count: number }) {
		// biome-ignore lint/correctness/useExhaustiveDependencies: a changed count invalidates cached pages, as in useLogWindow.
		const cache = useMemo(() => new LogPageCache<ILog>(), [count]);
		const [version, setVersion] = useState(0);
		const fetchRange = useCallback(
			async (offset: number, limit: number) => {
				requests.push(offset);
				return Array.from({ length: Math.min(limit, count - offset) }, (_, i) =>
					logAt(offset + i),
				);
			},
			[count],
		);
		const ensure = useCallback(
			(first: number, last: number) => {
				for (const page of cache.pagesFor(first, last, count)) {
					cache.request(page, fetchRange)?.then(() => setVersion((v) => v + 1));
				}
			},
			[cache, count, fetchRange],
		);
		const list: ILogWindow = {
			count,
			listKey: String(count),
			cacheKey: String(count),
			version,
			loading: false,
			pageFailed: false,
			row: (index) => cache.row(index),
			failed: () => false,
			ensure,
			fetchRange,
			retry: () => {},
		};
		return (
			<LogList
				handleRef={handle}
				window={list}
				view={view}
				actions={actions}
				onContextRow={() => {}}
				menu={null}
				empty={<div>No logs</div>}
			/>
		);
	}

	const container = window.document.createElement("div");
	window.document.body.appendChild(container);
	const root = createRoot(container as unknown as HTMLElement);
	try {
		await act(async () => root.render(<Harness count={30} />));
		const scroller = container.querySelector('[role="log"]');
		if (!scroller) throw new Error("Log viewport did not mount");
		const content = scroller.firstElementChild;
		if (!(content instanceof window.HTMLElement)) {
			throw new Error("Log content did not mount");
		}
		expect(content.style.height).toBe(`${30 * ROW_HEIGHT}px`);
		expect(requests).toEqual([0]);
		expect(container.textContent).toContain("Log entry 0");

		await act(async () => root.render(<Harness count={600} />));
		expect(content.style.height).toBe(`${600 * ROW_HEIGHT}px`);
		await act(async () => {
			scroller.scrollTop =
				Number.parseFloat(content.style.height) - viewportHeight;
			scroller.dispatchEvent(new window.Event("scroll"));
		});
		expect(requests).toContain(400);
		expect(
			container.querySelector('[data-log-index="599"]')?.textContent,
		).toContain("Log entry 599");
	} finally {
		await act(async () => root.unmount());
		container.remove();
	}
});
