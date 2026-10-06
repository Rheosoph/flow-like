import { afterAll, afterEach, describe, expect, test } from "bun:test";
import type { DockProps, FormSessionState, ViewState } from "../contracts";
import {
	allByRole,
	byRole,
	click,
	installWorkbenchDom,
	mountWorkbench,
	queryByRole,
	settle,
} from "../testing/dom";
import { fakeView } from "../testing/fake-actions";
import { fixture } from "../testing/fixtures";
import { DESKTOP_LAYOUT, PHONE_LAYOUT, withLayout } from "../testing/layouts";

const dom = installWorkbenchDom();
const { Dock } = await import("./dock");

afterEach(dom.cleanup);
afterAll(dom.restore);

async function mountDock(
	state: FormSessionState,
	variant: DockProps["variant"],
	layout = PHONE_LAYOUT,
) {
	const fake = fakeView(withLayout(state, layout), { layout });
	const view = await mountWorkbench(
		<div data-fw-root="">
			<Dock {...fake.props} variant={variant} />
		</div>,
		{ layout },
	);
	return { view, fake };
}

const withView = (
	state: FormSessionState,
	patch: Partial<ViewState>,
): FormSessionState => ({ ...state, view: { ...state.view, ...patch } });

const lineKind = (root: ParentNode) =>
	root
		.querySelector<HTMLElement>("[data-fw-dock-line]")
		?.getAttribute("data-fw-dock-line") ?? null;

describe("the phone bar", () => {
	test("Inputs / Output, Stop for the run on the stage, Run", async () => {
		const { view, fake } = await mountDock(fixture("running"), "phone");
		const panes = byRole("group", "Which pane to show");
		const inputs = byRole("button", "Inputs", panes);
		const output = byRole("button", /^Output/, panes);
		expect(inputs.getAttribute("aria-pressed")).toBe("false");
		expect(output.getAttribute("aria-pressed")).toBe("true");
		await click(inputs);
		expect(fake.argsOf("setPane")).toEqual([["inputs"]]);
		const stop = byRole("button", "Stop run 14");
		expect(stop.textContent).toBe("");
		await click(byRole("button", "Run"));
		expect(fake.argsOf("run")).toEqual([[{ from: "phone" }]]);
		await view.unmount();
	});

	test("Output shows a spinner while a run goes on and a dot for a result waiting on Inputs", async () => {
		const running = await mountDock(fixture("running"), "phone");
		expect(queryByRole("img", "running")).not.toBeNull();
		const panes = byRole("group", "Which pane to show");
		expect(panes.className.split(" ")).toEqual(
			expect.arrayContaining(["grid", "grid-cols-2"]),
		);
		await running.view.unmount();
		const done = fixture("done");
		const waiting = await mountDock(
			withView(done, { pane: "inputs", outputUnseen: true }),
			"phone",
		);
		expect(queryByRole("img", "new result")).not.toBeNull();
		expect(queryByRole("img", "running")).toBeNull();
		await waiting.view.unmount();
	});

	test("no Stop when the run on the stage has ended", async () => {
		const { view } = await mountDock(fixture("done"), "phone");
		expect(queryByRole("button", /^Stop/)).toBeNull();
		await view.unmount();
	});

	test("Run keeps its label with fields; the cursor key hint is left to hardware keyboards", async () => {
		const { view } = await mountDock(fixture("done"), "phone");
		const run = byRole("button", "Run");
		expect(run.querySelector("kbd, [aria-hidden='true']:not(svg)")).toBeNull();
		await view.unmount();
	});

	test("a form without fields: no Inputs / Output, Run again once a run ended", async () => {
		const first = await mountDock(fixture("none"), "phone");
		expect(queryByRole("group", "Which pane to show")).toBeNull();
		expect(queryByRole("button", "Run")).not.toBeNull();
		await first.view.unmount();
		const ended = await mountDock(fixture("none-done"), "phone");
		expect(queryByRole("button", "Run again")).not.toBeNull();
		await ended.view.unmount();
	});
});

describe("the status line above the phone bar", () => {
	test("problems, with 44 px arrows", async () => {
		const { view } = await mountDock(fixture("invalid"), "phone");
		expect(lineKind(view.container)).toBe("problems");
		const next = byRole("button", "Next field that needs a look");
		expect(next.className).toContain("size-11");
		await view.unmount();
	});

	test("never Ready, counts or the comparison", async () => {
		for (const name of ["idle", "running", "done"] as const) {
			const { view } = await mountDock(fixture(name), "phone");
			expect(lineKind(view.container)).toBeNull();
			await view.unmount();
		}
	});

	test("the queue line with Clear queue; the offer with 44 px Yes and No", async () => {
		const queued = await mountDock(fixture("queued"), "phone");
		expect(lineKind(queued.view.container)).toBe("queue");
		await click(byRole("button", "Clear queue"));
		expect(queued.fake.calls.map((call) => call.name)).toContain("clearQueue");
		await queued.view.unmount();
		const offer = await mountDock(fixture("small-offer"), "phone");
		expect(lineKind(offer.view.container)).toBe("question");
		expect(byRole("button", "Yes").className).toContain("h-11");
		await offer.view.unmount();
	});

	test("the start message of a run that queued", async () => {
		const { view } = await mountDock(fixture("series"), "phone");
		expect(lineKind(view.container)).toBe("message");
		expect(
			view.container.querySelector("[data-fw-dock-line] span")?.textContent,
		).toBe("Run 18 is queued: invoice-RE-2026-0921.pdf, 22 Sep 2026.");
		await view.unmount();
	});
});

describe("a form without fields at its cap", () => {
	test("the strip button waits: neutral, aria-disabled, the sentence in its title; a press is still reported", async () => {
		const { view, fake } = await mountDock(
			fixture("none-cap"),
			"strip",
			DESKTOP_LAYOUT,
		);
		const wait = byRole("button", "Wait for a run to end");
		expect(wait.getAttribute("aria-disabled")).toBe("true");
		expect(wait.getAttribute("title")).toBe(
			"3 runs are going. You can run again when one ends.",
		);
		expect(wait.className).toContain("bg-card");
		expect(wait.className).not.toContain("bg-primary");
		expect(wait.getAttribute("data-fw-focus")).toBe("run-again");
		await click(wait);
		expect(fake.argsOf("run")).toEqual([[{ from: "strip" }]]);
		expect(queryByRole("button", /^Stop/)).toBeNull();
		await view.unmount();
	});

	test("on the phone the button keeps its label and the sentence sits above the bar", async () => {
		const { view, fake } = await mountDock(fixture("none-cap"), "phone");
		const run = byRole("button", "Run");
		expect(run.getAttribute("aria-disabled")).toBe("true");
		expect(run.className).not.toContain("bg-primary");
		expect(run.getAttribute("title")).toBe(
			"3 runs are going. You can run again when one ends.",
		);
		expect(queryByRole("button", "Wait for a run to end")).toBeNull();
		await click(run);
		expect(fake.argsOf("run")).toEqual([[{ from: "phone" }]]);
		expect(lineKind(view.container)).toBe("capped");
		expect(
			view.container.querySelector("[data-fw-dock-line] span")?.textContent,
		).toBe("3 runs are going. You can run again when one ends.");
		await view.unmount();
	});
});

describe("hero and strip", () => {
	test("the hero's Run is one coral button, nothing else", async () => {
		const { view, fake } = await mountDock(
			fixture("none"),
			"hero",
			DESKTOP_LAYOUT,
		);
		const run = byRole("button", "Run");
		expect(run.getAttribute("data-fw-focus")).toBe("run");
		expect(run.className).toContain("bg-primary");
		expect(view.container.querySelector("[data-fw-dock-line]")).toBeNull();
		await click(run);
		expect(fake.argsOf("run")).toEqual([[{ from: "hero" }]]);
		await view.unmount();
	});

	test("the strip's Run says Run until a run has ended, then Run again, and carries the run-again focus hook", async () => {
		const first = await mountDock(fixture("none"), "strip", DESKTOP_LAYOUT);
		expect(byRole("button", "Run").getAttribute("data-fw-focus")).toBe(
			"run-again",
		);
		await first.view.unmount();
		const ended = await mountDock(
			fixture("none-done"),
			"strip",
			DESKTOP_LAYOUT,
		);
		const again = byRole("button", "Run again");
		expect(again.getAttribute("title")).toBe("Run again · ⌘↵");
		await click(again);
		expect(ended.fake.argsOf("run")).toEqual([[{ from: "strip" }]]);
		await ended.view.unmount();
	});

	test("the event's own label wins in every variant", async () => {
		const state = fixture("none");
		const custom: FormSessionState = {
			...state,
			form: { ...state.form, submitLabel: "Start" },
		};
		for (const variant of ["hero", "strip", "phone"] as const) {
			const { view } = await mountDock(custom, variant, DESKTOP_LAYOUT);
			expect(queryByRole("button", "Start")).not.toBeNull();
			await view.unmount();
		}
	});
});

describe("the Per run bottom sheet", () => {
	test("opens inside the interface with 44 px rows, Uncheck all and Done", async () => {
		const { view, fake } = await mountDock(fixture("after-run"), "phone");
		await settle();
		const dialog = byRole("dialog");
		const root = view.container.querySelector("[data-fw-root]");
		expect(root?.contains(dialog)).toBe(true);
		const rows = allByRole("checkbox", undefined, dialog);
		expect(rows).toHaveLength(9);
		expect(rows[0]?.closest("label")?.className).toContain("min-h-11");
		expect(byRole("button", "Done", dialog).className).toContain("h-11");
		await click(byRole("button", "Done", dialog));
		expect(fake.calls.map((call) => call.name)).toContain("closeOverlay");
		await view.unmount();
	});

	test("a toggle applies at once", async () => {
		const { view, fake } = await mountDock(fixture("after-run"), "phone");
		await settle();
		const dialog = byRole("dialog");
		const vendor = dialog.querySelector<HTMLInputElement>(
			'input[data-per-run="vendor_name"]',
		);
		if (!vendor) throw new Error("no Vendor row");
		await click(vendor);
		expect(fake.argsOf("setPerRun")).toEqual([["vendor_name", true]]);
		await view.unmount();
	});

	test("not open while the overlay is something else", async () => {
		const { view } = await mountDock(fixture("series"), "phone");
		await settle();
		expect(queryByRole("dialog")).toBeNull();
		await view.unmount();
	});
});
