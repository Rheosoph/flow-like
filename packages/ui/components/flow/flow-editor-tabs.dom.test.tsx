import {
	afterAll,
	afterEach,
	beforeEach,
	describe,
	expect,
	mock,
	test,
} from "bun:test";
import { Window } from "happy-dom";
import { type ComponentProps, act } from "react";
import type { IBoard } from "../../lib/schema/flow/board";
import type { IEditorTab } from "./shell/editor-documents";

const testWindow = new Window({ url: "https://localhost" });
Object.assign(testWindow, { SyntaxError, TypeError, Error });
const domGlobals = {
	window: testWindow,
	document: testWindow.document,
	navigator: testWindow.navigator,
	HTMLElement: testWindow.HTMLElement,
	Element: testWindow.Element,
	Node: testWindow.Node,
	MutationObserver: testWindow.MutationObserver,
	getComputedStyle: testWindow.getComputedStyle.bind(testWindow),
	IS_REACT_ACT_ENVIRONMENT: true,
};
const previousGlobals = new Map(
	Object.keys(domGlobals).map((key) => [
		key,
		Object.getOwnPropertyDescriptor(globalThis, key),
	]),
);
const installDom = () => Object.assign(globalThis, domGlobals);
installDom();

const { createRoot } = await import("react-dom/client");
const { FlowEditorTabs } = await import("./flow-editor-tabs");

let root: ReturnType<typeof createRoot>;
let container: HTMLDivElement;

beforeEach(() => {
	installDom();
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(() => {
	act(() => root.unmount());
	container.remove();
});

afterAll(() => {
	for (const [key, descriptor] of previousGlobals) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
	testWindow.happyDOM.abort();
});

const board = { layers: {} } as unknown as IBoard;
const tab = (fileId: string, extra: Partial<IEditorTab> = {}): IEditorTab => ({
	key: `board:${fileId}`,
	doc: { kind: "board", fileId },
	...extra,
});

function render(
	tabs: IEditorTab[],
	props: Partial<ComponentProps<typeof FlowEditorTabs>> = {},
) {
	const onPin = mock((_key: string, _pinned: boolean) => {});
	const onClose = mock((_key: string) => {});
	act(() =>
		root.render(
			<FlowEditorTabs
				board={board}
				tabs={tabs}
				activeKey={tabs[0]?.key ?? null}
				activeModuleId={null}
				resolveLabel={(entry) => entry.key}
				onSelect={() => {}}
				onClose={onClose}
				onSplit={() => {}}
				onPin={onPin}
				onColor={() => {}}
				executeCommand={async () => undefined}
				readOnly={false}
				trailing={<button type="button" data-testid="trailing" />}
				{...props}
			/>,
		),
	);
	return { onPin, onClose };
}

const scrollLane = (element: Element | null) =>
	element?.closest(".overflow-x-auto") ?? null;

const tabElement = (key: string) =>
	container.querySelector<HTMLElement>(`[data-tab-key="${key}"]`);

describe("FlowEditorTabs layout", () => {
	test("only the tabs scroll; the new-module button and editor actions stay put", () => {
		render([tab("main"), tab("a"), tab("b")]);
		expect(scrollLane(tabElement("board:a"))).not.toBeNull();
		expect(
			scrollLane(container.querySelector('[data-testid="trailing"]')),
		).toBeNull();
		const newModule = Array.from(container.querySelectorAll("button")).find(
			(button) => /new ?module/i.test(button.getAttribute("aria-label") ?? ""),
		);
		expect(newModule).toBeDefined();
		expect(scrollLane(newModule ?? null)).toBeNull();
	});

	test("pinned tabs get their own lane ahead of the rest", () => {
		render([tab("a", { pinned: true }), tab("main"), tab("b")]);
		const pinnedLane = scrollLane(tabElement("board:a"));
		const looseLane = scrollLane(tabElement("board:main"));
		expect(pinnedLane).not.toBeNull();
		expect(looseLane).not.toBeNull();
		expect(pinnedLane).not.toBe(looseLane);
		expect(scrollLane(tabElement("board:b"))).toBe(looseLane);
		const order = (pinnedLane as Node).compareDocumentPosition(
			looseLane as Node,
		);
		expect(order & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
	});

	test("a pinned tab trades its close button for unpin", () => {
		const { onPin, onClose } = render([
			tab("main"),
			tab("a", { pinned: true }),
		]);
		const buttons = Array.from(
			tabElement("board:a")?.querySelectorAll("button") ?? [],
		);
		const labels = buttons.map((button) =>
			(button.getAttribute("aria-label") ?? "").toLowerCase(),
		);
		expect(labels.some((label) => label.includes("close"))).toBe(false);
		const unpin = buttons.find((button) =>
			/unpin/i.test(button.getAttribute("aria-label") ?? ""),
		);
		expect(unpin).toBeDefined();
		act(() => unpin?.click());
		expect(onPin.mock.calls).toEqual([["board:a", false]]);
		expect(onClose).not.toHaveBeenCalled();
	});

	test("a coloured tab tints its icon and draws a colour bar", () => {
		render([tab("main"), tab("a", { color: "teal" })]);
		const element = tabElement("board:a");
		expect(element?.querySelector(".bg-tab-teal")).not.toBeNull();
		expect(element?.querySelector("svg.text-tab-teal")).not.toBeNull();
		expect(
			tabElement("board:main")?.querySelector("[class*='bg-tab-']"),
		).toBeNull();
	});
});
