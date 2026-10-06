import { afterAll, afterEach, describe, expect, test } from "bun:test";
import type { FormSessionState } from "../contracts";
import {
	advance,
	installWorkbenchDom,
	mountWorkbench,
	settle,
} from "../testing/dom";
import { fixture } from "../testing/fixtures";
import {
	DESKTOP_LAYOUT,
	PHONE_LAYOUT,
	layoutFor,
	withLayout,
} from "../testing/layouts";
import { shellKit } from "./shell-test-kit";

const dom = installWorkbenchDom();
const { ShellFrame, HERO_WAIT_MS } = await import("./shell-frame");

afterEach(dom.cleanup);
afterAll(dom.restore);

const rootOf = (container: Element) => {
	const root = container.querySelector("[data-fw-root]");
	if (!root) throw new Error("no interface root");
	return root;
};
const has = (scope: Element, selector: string) =>
	scope.querySelector(selector) !== null;

async function mount(
	state: FormSessionState,
	layout = DESKTOP_LAYOUT,
	options: Parameters<typeof shellKit>[1] = {},
) {
	const kit = shellKit(state, options);
	const view = await mountWorkbench(<ShellFrame {...kit.props} />, { layout });
	return { ...kit, view, root: rootOf(view.container) };
}

describe("layout", () => {
	test("renders nothing but the root while no layout is known", async () => {
		const kit = shellKit({ ...fixture("idle"), layout: null });
		const view = await mountWorkbench(<ShellFrame {...kit.props} />, {
			layout: DESKTOP_LAYOUT,
			sizeOf: () => null,
		});
		const root = rootOf(view.container);
		expect(root.getAttribute("data-fw-layout")).toBe("pending");
		expect(root.getAttribute("tabindex")).toBe("-1");
		expect(root.querySelector("[data-testid]")).toBeNull();
		expect(kit.fake.calls).toEqual([]);
	});

	test("reports the measured box and pointer to the session", async () => {
		const { fake, root } = await mount({ ...fixture("idle"), layout: null });
		expect(fake.argsOf("setLayout")).toEqual([[DESKTOP_LAYOUT]]);
		expect(root.getAttribute("data-fw-layout")).toBe("split");
	});

	test("does not report a layout the session already has", async () => {
		const { fake } = await mount(fixture("idle"));
		expect(fake.argsOf("setLayout")).toEqual([]);
	});

	test("a session that keeps another layout is told once, not on every render", async () => {
		const state = fixture("idle");
		const kit = shellKit({
			...state,
			layout: layoutFor(1360, 800),
		});
		const view = await mountWorkbench(<ShellFrame {...kit.props} />, {
			layout: DESKTOP_LAYOUT,
		});
		for (const height of [801, 802, 803]) {
			await view.rerender(
				<ShellFrame
					{...kit.props}
					state={{ ...state, layout: layoutFor(1360, height) }}
				/>,
			);
		}
		expect(kit.fake.argsOf("setLayout")).toEqual([[DESKTOP_LAYOUT]]);
	});

	test("a resized box is reported and re-composed", async () => {
		const { fake, view, root } = await mount(fixture("idle"));
		await view.resize(PHONE_LAYOUT);
		expect(fake.argsOf("setLayout")).toEqual([[PHONE_LAYOUT]]);
		expect(root.getAttribute("data-fw-layout")).toBe("single");
	});

	test("a browser that already knows the box gives the first frame its layout", async () => {
		const proto = dom.window.HTMLElement.prototype as unknown as {
			getBoundingClientRect: () => unknown;
		};
		const original = proto.getBoundingClientRect;
		proto.getBoundingClientRect = () => ({ width: 1100, height: 700 });
		try {
			const kit = shellKit({ ...fixture("idle"), layout: null });
			const view = await mountWorkbench(<ShellFrame {...kit.props} />, {
				layout: DESKTOP_LAYOUT,
				sizeOf: () => null,
			});
			expect(rootOf(view.container).getAttribute("data-fw-layout")).toBe(
				"split",
			);
			expect(kit.fake.argsOf("setLayout")).toEqual([[layoutFor(1100, 700)]]);
		} finally {
			proto.getBoundingClientRect = original;
		}
	});

	test("a touch screen in a wide box keeps the split but not the fine pointer", async () => {
		const wideTouch = layoutFor(1024, 768, { touch: true });
		const { fake, root } = await mount(
			{ ...fixture("idle"), layout: null },
			wideTouch,
		);
		expect(fake.argsOf("setLayout")).toEqual([[wideTouch]]);
		expect(root.getAttribute("data-fw-layout")).toBe("split");
	});
});

describe("a form with fields", () => {
	test("split: the rail and its dock on the left, the stage beside them", async () => {
		const { root } = await mount(fixture("idle"));
		const rail = root.querySelector('aside[data-fw-region="rail"]');
		expect(rail?.getAttribute("aria-label")).toBe("Inputs");
		expect(rail?.querySelector('[data-testid="rail"]')).not.toBeNull();
		expect(rail?.querySelector('[data-testid="dock-rail"]')).not.toBeNull();
		const stage = root.querySelector('section[data-fw-region="stage"]');
		expect(stage?.getAttribute("aria-label")).toBe("Output");
		expect(stage?.querySelector('[data-testid="stage"]')).not.toBeNull();
		expect(root.querySelector('[data-testid="dock-phone"]')).toBeNull();
	});

	test("narrow, Inputs pane: the rail above the phone dock, no stage", async () => {
		const state = withLayout(fixture("idle"), PHONE_LAYOUT);
		expect(state.view.pane).toBe("inputs");
		const { root } = await mount(state, PHONE_LAYOUT);
		expect(root.getAttribute("data-fw-layout")).toBe("single");
		expect(has(root, '[data-testid="rail"]')).toBe(true);
		expect(has(root, '[data-testid="stage"]')).toBe(false);
		expect(has(root, '[data-testid="dock-phone"]')).toBe(true);
		expect(has(root, '[data-testid="dock-rail"]')).toBe(false);
	});

	test("narrow, Output pane: the stage above the phone dock, no rail", async () => {
		const state = withLayout(fixture("running"), PHONE_LAYOUT);
		expect(state.view.pane).toBe("output");
		const { root } = await mount(state, PHONE_LAYOUT);
		expect(has(root, '[data-testid="stage"]')).toBe(true);
		expect(has(root, '[data-testid="rail"]')).toBe(false);
		expect(has(root, '[data-testid="dock-phone"]')).toBe(true);
	});

	test("a wide box without a split shows one pane at a time with desktop metrics", async () => {
		const narrowDesktop = layoutFor(820, 900);
		const { root } = await mount(
			withLayout(fixture("idle"), narrowDesktop),
			narrowDesktop,
		);
		expect(root.getAttribute("data-fw-layout")).toBe("single");
		expect(has(root, '[data-testid="dock-phone"]')).toBe(true);
	});
});

describe("a form without fields", () => {
	const none = () => fixture("none");

	test("split, before the first run: a centred card with the name, the description and the hero button", async () => {
		const { root } = await mount(none());
		const hero = root.querySelector("[data-fw-hero]");
		expect(hero?.querySelector("h1")?.textContent).toBe(
			"Triage selected request",
		);
		expect(hero?.textContent).toContain("Analyze the selected customer");
		expect(hero?.querySelector('[data-testid="dock-hero"]')).not.toBeNull();
		expect(has(root, '[data-fw-region="rail"]')).toBe(false);
		expect(has(root, '[data-testid="stage"]')).toBe(false);
		expect(has(root, '[data-testid="dock-phone"]')).toBe(false);
	});

	test("split, after a run: the stage fills the box, no rail and no dock of the shell", async () => {
		const { root } = await mount(fixture("none-done"));
		expect(has(root, "[data-fw-hero]")).toBe(false);
		expect(has(root, '[data-testid="stage"]')).toBe(true);
		expect(has(root, '[data-fw-region="rail"]')).toBe(false);
		expect(has(root, '[data-testid="dock-hero"]')).toBe(false);
	});

	test("narrow, before the first run: bare text and the phone dock, no card", async () => {
		const { root } = await mount(
			withLayout(none(), PHONE_LAYOUT),
			PHONE_LAYOUT,
		);
		const hero = root.querySelector("[data-fw-hero]");
		expect(hero?.querySelector("h1")?.textContent).toBe(
			"Triage selected request",
		);
		expect(hero?.querySelector('[data-testid="dock-hero"]')).toBeNull();
		expect(hero?.querySelector(".bg-card")).toBeNull();
		expect(has(root, '[data-testid="dock-phone"]')).toBe(true);
	});

	test("narrow, after a run: the stage above the phone dock", async () => {
		const { root } = await mount(
			withLayout(fixture("none-done"), PHONE_LAYOUT),
			PHONE_LAYOUT,
		);
		expect(has(root, '[data-testid="stage"]')).toBe(true);
		expect(has(root, '[data-testid="dock-phone"]')).toBe(true);
		expect(has(root, "[data-fw-hero]")).toBe(false);
	});

	test("waits for the device memory before it shows the first-run card, then shows it anyway", async () => {
		const state = none();
		const { root } = await mount({
			...state,
			memory: { ...state.memory, loaded: false },
		});
		expect(has(root, "[data-fw-hero]")).toBe(false);
		await advance(HERO_WAIT_MS + 150);
		expect(has(root, "[data-fw-hero]")).toBe(true);
	});

	test("shows the stage, never the card, once history arrives with runs", async () => {
		const state = none();
		const done = fixture("none-done");
		const { root, view, props } = await mount({
			...state,
			memory: { ...state.memory, loaded: false },
		});
		expect(has(root, "[data-fw-hero]")).toBe(false);
		await view.rerender(
			<ShellFrame
				{...props}
				state={{ ...done, memory: { ...done.memory, loaded: true } }}
			/>,
		);
		await settle();
		expect(has(root, "[data-fw-hero]")).toBe(false);
		expect(has(root, '[data-testid="stage"]')).toBe(true);
	});
});

describe("live region", () => {
	const announce = (
		state: FormSessionState,
		announcement: FormSessionState["announcement"],
	): FormSessionState => ({ ...state, announcement });

	const textOf = (container: Element) =>
		container.querySelector("[data-fw-live]")?.textContent ?? "";

	test("is polite and empty at first; an announcement the form mounted with is not read", async () => {
		const state = fixture("done");
		expect(state.announcement).not.toBeNull();
		const { view } = await mount(state);
		const live = view.container.querySelector("[data-fw-live]");
		expect(live?.getAttribute("aria-live")).toBe("polite");
		expect(textOf(view.container)).toBe("");
	});

	test("reads a run that ends, then the next one, then a refused press", async () => {
		const base = announce(fixture("idle"), null);
		const { view, props } = await mount(base);
		const show = async (announcement: FormSessionState["announcement"]) => {
			await view.rerender(
				<ShellFrame {...props} state={announce(base, announcement)} />,
			);
			return textOf(view.container);
		};
		expect(
			await show({ seq: 2, kind: "done", n: 15, seconds: 41, step: null }),
		).toBe("Run 15 done in 41 s.");
		expect(
			await show({
				seq: 3,
				kind: "failed",
				n: 18,
				seconds: 12,
				step: { number: 2, title: "Run OCR" },
			}),
		).toBe("Run 18 failed after 12 s at step 2: Run OCR.");
		expect(
			await show({ seq: 4, kind: "stopped", n: 17, seconds: 37, step: null }),
		).toBe("Run 17 stopped at 0:37.");
		expect(await show({ seq: 5, kind: "capReached", running: 3 })).toBe(
			"3 runs are going. You can run again when one ends.",
		);
	});

	test("a repeated sentence is announced again: its node is replaced", async () => {
		const base = announce(fixture("none"), null);
		const { view, props } = await mount(base);
		await view.rerender(
			<ShellFrame
				{...props}
				state={announce(base, { seq: 2, kind: "capReached", running: 3 })}
			/>,
		);
		const first = view.container.querySelector("[data-fw-live] span");
		await view.rerender(
			<ShellFrame
				{...props}
				state={announce(base, { seq: 3, kind: "capReached", running: 3 })}
			/>,
		);
		const second = view.container.querySelector("[data-fw-live] span");
		expect(second?.textContent).toBe(first?.textContent ?? "");
		expect(second).not.toBe(first);
	});
});

describe("route buttons in the host's header", () => {
	test("are pushed once, in the header, never inline", async () => {
		const kit = shellKit(fixture("done"), { withToolbar: true });
		const view = await mountWorkbench(<ShellFrame {...kit.props} />, {
			layout: DESKTOP_LAYOUT,
		});
		expect(kit.pushed).toHaveLength(1);
		expect(kit.pushed[0]).toHaveLength(1);
		expect(view.container.querySelector("[data-fw-route-bar]")).toBeNull();
	});

	test("are cleared when the form goes away", async () => {
		const kit = shellKit(fixture("done"), { withToolbar: true });
		const view = await mountWorkbench(<ShellFrame {...kit.props} />, {
			layout: DESKTOP_LAYOUT,
		});
		await view.unmount();
		expect(kit.pushed).toHaveLength(2);
		expect(kit.pushed[1]).toEqual([]);
	});

	test("a form without routes pushes nothing", async () => {
		const kit = shellKit(fixture("small"), { withToolbar: true });
		await mountWorkbench(<ShellFrame {...kit.props} />, {
			layout: DESKTOP_LAYOUT,
		});
		expect(kit.pushed).toEqual([]);
	});

	test("new routes replace the old buttons", async () => {
		const state = fixture("done");
		const kit = shellKit(state, { withToolbar: true });
		const view = await mountWorkbench(<ShellFrame {...kit.props} />, {
			layout: DESKTOP_LAYOUT,
		});
		const fewer: FormSessionState = {
			...state,
			form: { ...state.form, routes: ["/"] },
		};
		await view.rerender(<ShellFrame {...kit.props} state={fewer} />);
		expect(kit.pushed.map((elements) => elements.length)).toEqual([1, 0, 1]);
	});

	test("without a header the same buttons sit in a bar at the top of the interface", async () => {
		const { root } = await mount(fixture("done"));
		const bar = root.querySelector("[data-fw-route-bar]");
		expect(bar?.getAttribute("aria-label")).toBe("Screens of this app");
		expect(
			Array.from(bar?.querySelectorAll("button") ?? []).map(
				(button) => button.textContent,
			),
		).toEqual(["Home", "Review queue"]);
	});

	test("a form without routes has no bar", async () => {
		const { root } = await mount(fixture("small"));
		expect(has(root, "[data-fw-route-bar]")).toBe(false);
	});
});
