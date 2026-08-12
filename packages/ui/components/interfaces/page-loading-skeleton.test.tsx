import { describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";

/**
 * happy-dom has no WebGL, so `getContext("webgl")` returns null throughout —
 * which is also the real degradation path on a machine without a GL context.
 * The readout must still mount and advance.
 */
async function setup({ reducedMotion = false } = {}) {
	const window = new Window();
	Object.assign(window, { SyntaxError, TypeError });
	Object.assign(window, {
		matchMedia: (query: string) => ({
			matches: reducedMotion && query.includes("prefers-reduced-motion"),
			media: query,
			addEventListener() {},
			removeEventListener() {},
		}),
	});
	Object.assign(globalThis, {
		document: window.document,
		Element: window.Element,
		Event: window.Event,
		HTMLElement: window.HTMLElement,
		MouseEvent: window.MouseEvent,
		Node: window.Node,
		navigator: window.navigator,
		window,
		Document: window.Document,
		DocumentFragment: window.DocumentFragment,
		Text: window.Text,
		MutationObserver: window.MutationObserver,
		getComputedStyle: window.getComputedStyle.bind(window),
		matchMedia: window.matchMedia,
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		IS_REACT_ACT_ENVIRONMENT: true,
	});
	const { createRoot } = await import("react-dom/client");
	const container = window.document.createElement("div");
	window.document.body.append(container);
	return { container, root: createRoot(container) };
}

const PHASE_MS = 2200;

/** Two phase dwells plus the first-import cost of the component's module graph. */
const TIMEOUT_MS = 20_000;

describe("PageLoadingSkeleton", () => {
	test(
		"renders the run title as the eyebrow and four ticks, the first one live",
		async () => {
			const { PageLoadingSkeleton } = await import("./page-loading-skeleton");
			const { container, root } = await setup();

			await act(async () => {
				root.render(<PageLoadingSkeleton title="Preparing workflow" />);
			});

			expect(container.querySelector(".fxl-kicker")?.textContent).toBe(
				"Preparing workflow",
			);
			expect(container.querySelectorAll(".fxl-tick")).toHaveLength(4);
			expect(container.querySelectorAll(".fxl-tick.is-cur")).toHaveLength(1);
			expect(container.querySelectorAll(".fxl-tick.is-done")).toHaveLength(0);
			expect(container.querySelector(".fxl-phase")?.textContent).toBe(
				"Initializing workflow",
			);
			expect(
				container.querySelector(".fxl-ticks")?.getAttribute("aria-label"),
			).toBe("Step 1 of 4");

			await act(async () => root.unmount());
		},
		TIMEOUT_MS,
	);

	test(
		"advancing hands the line over: the outgoing phase animates out, then leaves",
		async () => {
			const { PageLoadingSkeleton } = await import("./page-loading-skeleton");
			const { container, root } = await setup();

			await act(async () => {
				root.render(<PageLoadingSkeleton />);
			});

			await act(async () => {
				await Bun.sleep(PHASE_MS + 120);
			});

			const phases = [...container.querySelectorAll(".fxl-phase")];
			expect(phases).toHaveLength(2);
			expect(phases[0].className).toContain("is-out");
			expect(phases[0].textContent).toBe("Initializing workflow");
			expect(phases[1].className).toContain("is-in");
			expect(phases[1].textContent).toBe("Loading resources");

			expect(container.querySelectorAll(".fxl-tick.is-done")).toHaveLength(1);
			expect(
				container.querySelector(".fxl-ticks")?.getAttribute("aria-label"),
			).toBe("Step 2 of 4");

			// The outgoing layer is torn down once its animation is over, so the stack
			// never accumulates dead phrases over a long run.
			await act(async () => {
				await Bun.sleep(1000);
			});
			expect(container.querySelectorAll(".fxl-phase")).toHaveLength(1);

			await act(async () => root.unmount());
		},
		TIMEOUT_MS,
	);

	test(
		"reduced motion holds the first phase instead of cycling",
		async () => {
			const { PageLoadingSkeleton } = await import("./page-loading-skeleton");
			const { container, root } = await setup({ reducedMotion: true });

			await act(async () => {
				root.render(<PageLoadingSkeleton />);
			});

			await act(async () => {
				await Bun.sleep(PHASE_MS + 300);
			});

			expect(container.querySelectorAll(".fxl-phase")).toHaveLength(1);
			expect(container.querySelector(".fxl-phase")?.textContent).toBe(
				"Initializing workflow",
			);

			await act(async () => root.unmount());
		},
		TIMEOUT_MS,
	);
});
