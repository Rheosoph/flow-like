import { afterAll, afterEach, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { createInstance } from "i18next";
import { act } from "react";
import { I18nextProvider } from "react-i18next";

const window = new Window({ url: "https://localhost" });
Object.assign(window, { SyntaxError, TypeError, Error });
const globals = {
	window,
	document: window.document,
	navigator: window.navigator,
	HTMLElement: window.HTMLElement,
	SVGElement: window.SVGElement,
	Element: window.Element,
	Node: window.Node,
	ResizeObserver: window.ResizeObserver,
	getComputedStyle: window.getComputedStyle.bind(window),
	requestAnimationFrame: (callback: FrameRequestCallback) =>
		setTimeout(() => callback(performance.now()), 0),
	cancelAnimationFrame: clearTimeout,
	IS_REACT_ACT_ENVIRONMENT: true,
};
const descriptors = Object.keys(globals).map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
Object.assign(globalThis, globals);
const { createRoot } = await import("react-dom/client");
const { BoardRuntimeProvider } = await import("./board-runtime");
const { EguiCanvasLayer } = await import("./egui-canvas-layer");
const { StandaloneBoardControls } = await import("./standalone-board-chrome");

const translated = {
	boardZoomIn: "Vergroessern",
	boardZoomOut: "Verkleinern",
	boardFitView: "Ansicht anpassen",
	boardToggleInteractivity: "Interaktivitaet umschalten",
	canvasLabel: "Workflow-Flaeche",
};
const i18n = createInstance();
await i18n.init({
	lng: "de",
	fallbackLng: false,
	resources: { de: { flow: translated } },
	initAsync: false,
});

const roots: ReturnType<typeof createRoot>[] = [];
afterEach(async () => {
	await act(async () => {
		for (const root of roots.splice(0)) root.unmount();
	});
	window.document.body.replaceChildren();
});
afterAll(() => {
	for (const [key, descriptor] of descriptors) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
	window.happyDOM.abort();
});

const render = async (ui: React.ReactNode) => {
	const container = window.document.createElement("div");
	window.document.body.append(container);
	const root = createRoot(container as unknown as Element);
	roots.push(root);
	await act(async () =>
		root.render(
			<I18nextProvider i18n={i18n}>
				<BoardRuntimeProvider>{ui}</BoardRuntimeProvider>
			</I18nextProvider>,
		),
	);
	return container;
};

test("the standalone zoom buttons take their labels from the flow namespace", async () => {
	const container = await render(
		<StandaloneBoardControls onScreenshot={() => {}} />,
	);
	const labels = [...container.querySelectorAll("button[aria-label]")].map(
		(button) => button.getAttribute("aria-label"),
	);
	expect(labels).toContain(translated.boardZoomIn);
	expect(labels).toContain(translated.boardZoomOut);
	expect(labels).toContain(translated.boardFitView);
	expect(labels).toContain(translated.boardToggleInteractivity);
	expect(
		container
			.querySelector(".react-flow__controls-zoomin")
			?.getAttribute("title"),
	).toBe(translated.boardZoomIn);
});

test("the interactive canvas announces itself in the active language", async () => {
	const container = await render(
		<EguiCanvasLayer
			scene={{ nodes: [], edges: [] }}
			palette={{} as never}
			boardProps={{} as never}
			onError={() => {}}
			onReady={() => {}}
		/>,
	);
	expect(
		container
			.querySelector("[data-board-canvas-input]")
			?.getAttribute("aria-label"),
	).toBe(translated.canvasLabel);
});
