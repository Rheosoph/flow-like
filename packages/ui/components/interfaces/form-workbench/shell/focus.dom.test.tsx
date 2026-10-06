import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act, useState } from "react";
import type { FocusRequest, FormSessionState } from "../contracts";
import { installWorkbenchDom, mountWorkbench } from "../testing/dom";
import { fixture } from "../testing/fixtures";
import {
	DESKTOP_LAYOUT,
	PHONE_LAYOUT,
	layoutFor,
	withLayout,
} from "../testing/layouts";
import { FAKE_PARTS, shellKit } from "./shell-test-kit";

const dom = installWorkbenchDom();
const { ShellFrame } = await import("./shell-frame");
const { rescueFocus } = await import("./focus-rescue");
const { focusSpotOf, returnFocusTo } = await import("./focus");

afterEach(dom.cleanup);
afterAll(dom.restore);

const withFocus = (
	state: FormSessionState,
	focus: FocusRequest | null,
): FormSessionState => ({ ...state, view: { ...state.view, focus } });

const field = (
	key: string,
	options: { select?: boolean; scrollOnly?: boolean } = {},
): FocusRequest["target"] => ({
	kind: "field",
	key,
	select: options.select ?? false,
	scrollOnly: options.scrollOnly ?? false,
});

async function mount(
	state: FormSessionState,
	layout = DESKTOP_LAYOUT,
	parts = FAKE_PARTS,
) {
	const kit = shellKit(state);
	const props = { ...kit.props, parts };
	const view = await mountWorkbench(<ShellFrame {...props} />, { layout });
	const root = view.container.querySelector("[data-fw-root]") as HTMLElement;
	const pick = <T extends HTMLElement>(selector: string) =>
		root.querySelector(selector) as T;
	return { ...kit, props, view, root, pick };
}

const active = () => document.activeElement;

describe("the focus executor", () => {
	test("moves focus to the element a request names, then reports it handled", async () => {
		const { fake, pick } = await mount(
			withFocus(fixture("idle"), {
				seq: 7,
				target: field("vendor_name"),
			}),
		);
		expect(active()).toBe(pick("[aria-label=Vendor]"));
		expect(fake.argsOf("focusHandled")).toEqual([[7]]);
	});

	test("selects the whole value when asked", async () => {
		const state = fixture("idle");
		const kit = shellKit(state);
		const view = await mountWorkbench(<ShellFrame {...kit.props} />, {
			layout: DESKTOP_LAYOUT,
		});
		const input = view.container.querySelector(
			"[aria-label=Vendor]",
		) as HTMLInputElement;
		input.value = "Nordwind Logistik GmbH";
		await view.rerender(
			<ShellFrame
				{...kit.props}
				state={withFocus(state, {
					seq: 3,
					target: field("vendor_name", { select: true }),
				})}
			/>,
		);
		expect(active()).toBe(input);
		expect(input.selectionStart).toBe(0);
		expect(input.selectionEnd).toBe("Nordwind Logistik GmbH".length);
	});

	test("a field request on a phone can scroll without focusing, so no keyboard opens", async () => {
		const state = withLayout(fixture("idle"), PHONE_LAYOUT);
		const kit = shellKit(state);
		const view = await mountWorkbench(<ShellFrame {...kit.props} />, {
			layout: PHONE_LAYOUT,
		});
		const input = view.container.querySelector(
			"[aria-label=Vendor]",
		) as HTMLInputElement;
		let scrolled = 0;
		input.scrollIntoView = () => {
			scrolled += 1;
		};
		await view.rerender(
			<ShellFrame
				{...kit.props}
				state={withFocus(state, {
					seq: 4,
					target: field("vendor_name", { scrollOnly: true }),
				})}
			/>,
		);
		expect(scrolled).toBe(1);
		expect(active()).not.toBe(input);
		expect(kit.fake.argsOf("focusHandled")).toEqual([[4]]);
	});

	test("Stop falls back to Run when the run ended and the button is gone", async () => {
		const RunOnly = () => (
			<button type="button" data-fw-focus="run">
				Run
			</button>
		);
		const { pick, fake } = await mount(
			withFocus(fixture("idle"), { seq: 2, target: { kind: "stop" } }),
			DESKTOP_LAYOUT,
			{ ...FAKE_PARTS, Dock: RunOnly },
		);
		expect(active()).toBe(pick("[data-fw-focus=run]"));
		expect(fake.argsOf("focusHandled")).toEqual([[2]]);
	});

	test("a target that is not on screen leaves focus on the interface, never on the page", async () => {
		const { root, fake } = await mount(
			withFocus(fixture("idle"), { seq: 5, target: field("nowhere") }),
		);
		expect(active()).toBe(root);
		expect(fake.argsOf("focusHandled")).toEqual([[5]]);
	});

	test("the Copy button, a run tab and the Run button are reachable by their values", async () => {
		const state = fixture("done");
		const kit = shellKit(state);
		const view = await mountWorkbench(<ShellFrame {...kit.props} />, {
			layout: DESKTOP_LAYOUT,
		});
		const root = view.container.querySelector("[data-fw-root]") as HTMLElement;
		const go = async (seq: number, target: FocusRequest["target"]) => {
			await view.rerender(
				<ShellFrame {...kit.props} state={withFocus(state, { seq, target })} />,
			);
		};
		await go(1, { kind: "copy" });
		expect(active()).toBe(root.querySelector("[data-fw-focus=copy]"));
		await go(2, { kind: "tab", runId: "run-14" });
		expect(active()).toBe(root.querySelector("[data-fw-focus='tab:run-14']"));
		await go(3, { kind: "run" });
		expect(active()).toBe(root.querySelector("[data-fw-focus=run]"));
		expect(kit.fake.argsOf("focusHandled")).toEqual([[1], [2], [3]]);
	});

	test("while a modal is open focus stays inside it", async () => {
		const state = fixture("shortcuts");
		const { fake, view } = await mount(
			withFocus(state, { seq: 8, target: { kind: "run" } }),
		);
		const modal = view.container.querySelector("[data-fw-modal]");
		expect(modal).not.toBeNull();
		expect(modal?.contains(active())).toBe(true);
		expect(fake.argsOf("focusHandled")).toEqual([[8]]);
	});
});

describe("the cursor starts in the form for repeat users (S6)", () => {
	test("in the first empty required field, with the focus ring hidden until a key or a click", async () => {
		const { root, pick } = await mount(fixture("reopen"));
		expect(active()).toBe(pick("[data-fw-focus='field:invoice_file']"));
		expect(root.hasAttribute("data-fw-quiet-focus")).toBe(true);
		expect(root.querySelector("style")?.textContent).toContain(
			"data-fw-quiet-focus",
		);

		const view = root.ownerDocument.defaultView as unknown as typeof globalThis;
		await act(async () => {
			active()?.dispatchEvent(
				new view.KeyboardEvent("keydown", { key: "Tab", bubbles: true }),
			);
		});
		expect(root.hasAttribute("data-fw-quiet-focus")).toBe(false);
	});

	test("a click gives the ring back too", async () => {
		const { root, pick } = await mount(fixture("reopen"));
		expect(root.hasAttribute("data-fw-quiet-focus")).toBe(true);
		const view = root.ownerDocument.defaultView as unknown as typeof globalThis;
		await act(async () => {
			pick("[aria-label=Vendor]").dispatchEvent(
				new view.PointerEvent("pointerdown", { bubbles: true }),
			);
		});
		expect(root.hasAttribute("data-fw-quiet-focus")).toBe(false);
	});

	test("the root paints selections neutral and smooths its text as the canvas does", async () => {
		const { root } = await mount(fixture("idle"));
		expect(root.querySelector("style")?.textContent).toContain(
			"[data-fw-root] ::selection",
		);
		expect(root.classList.contains("antialiased")).toBe(true);
	});

	test("never on a first visit", async () => {
		const { root } = await mount(fixture("idle"));
		expect(active()).toBe(document.body);
		expect(root.hasAttribute("data-fw-quiet-focus")).toBe(false);
	});

	test("never in a tile", async () => {
		const state = fixture("reopen");
		const { root } = await mount({
			...state,
			form: {
				...state.form,
				host: { ...state.form.host, presentation: "tile" },
			},
		});
		expect(active()).toBe(document.body);
		expect(root.hasAttribute("data-fw-quiet-focus")).toBe(false);
	});

	test("never on a touch screen or in a narrow box", async () => {
		const touch = layoutFor(1024, 768, { touch: true });
		const first = await mount(withLayout(fixture("reopen"), touch), touch);
		expect(active()).toBe(document.body);
		await first.view.unmount();
		const second = await mount(
			withLayout(fixture("reopen"), PHONE_LAYOUT),
			PHONE_LAYOUT,
		);
		expect(active()).toBe(document.body);
		expect(second.root.hasAttribute("data-fw-quiet-focus")).toBe(false);
	});

	test("never in a form without fields", async () => {
		const done = fixture("none-done");
		await mount({ ...done, runs: fixture("done").runs });
		expect(active()).toBe(document.body);
	});

	test("does not take focus from something else that already has it", async () => {
		const outside = document.createElement("button");
		document.body.append(outside);
		outside.focus();
		const { root } = await mount(fixture("reopen"));
		expect(active()).toBe(outside);
		expect(root.hasAttribute("data-fw-quiet-focus")).toBe(false);
	});

	test("happens once: a later render does not pull the cursor back", async () => {
		const state = fixture("reopen");
		const { view, props, pick } = await mount(state);
		expect(active()).toBe(pick("[data-fw-focus='field:invoice_file']"));
		pick<HTMLElement>("[data-fw-focus=run]").focus();
		await view.rerender(
			<ShellFrame {...props} state={{ ...state, seq: 99 }} />,
		);
		expect(active()).toBe(pick("[data-fw-focus=run]"));
	});

	test("waits for the device memory", async () => {
		const state = fixture("reopen");
		const { view, props, pick } = await mount({
			...state,
			memory: { ...state.memory, loaded: false },
		});
		expect(active()).toBe(document.body);
		await view.rerender(<ShellFrame {...props} state={state} />);
		expect(active()).toBe(pick("[data-fw-focus='field:invoice_file']"));
	});
});

/** A stage whose "Dismiss" button removes itself on a click, by its own state: the shell does not render. */
function DismissStage() {
	const [shown, setShown] = useState(true);
	return shown ? (
		<button type="button" onClick={() => setShown(false)}>
			Dismiss
		</button>
	) : null;
}

describe("the focus rescue", () => {
	test("an element that removes itself with focus on it hands focus to Run, not to the page", async () => {
		const { root } = await mount(fixture("idle"), DESKTOP_LAYOUT, {
			...FAKE_PARTS,
			Stage: DismissStage,
		});
		const dismiss = Array.from(root.querySelectorAll("button")).find(
			(button) => button.textContent === "Dismiss",
		);
		if (!dismiss) throw new Error("no Dismiss button");
		await act(async () => dismiss.focus());
		await act(async () => dismiss.click());
		expect(dismiss.isConnected).toBe(false);
		await act(async () => {
			for (let step = 0; step < 3; step++)
				await new Promise((resolve) => setTimeout(resolve, 0));
		});
		expect(active()?.getAttribute("data-fw-focus")).toBe("run");
	});

	test("focus given back puts a text caret where it was; a control without one just takes focus", () => {
		const box = document.createElement("div");
		box.innerHTML =
			'<textarea>Nordwind Logistik GmbH</textarea><input type="checkbox" />';
		document.body.append(box);
		const text = box.querySelector("textarea") as HTMLTextAreaElement;
		text.focus();
		text.setSelectionRange(8, 8);
		const spot = focusSpotOf(document);
		expect(spot?.selection).toEqual([8, 8]);
		text.addEventListener("focus", () => text.select());
		text.blur();
		expect(returnFocusTo(spot)).toBe(true);
		expect([text.selectionStart, text.selectionEnd]).toEqual([8, 8]);

		const check = box.querySelector("input") as HTMLInputElement;
		check.focus();
		const plain = focusSpotOf(document);
		check.blur();
		expect(returnFocusTo(plain)).toBe(true);
		expect(active() === check).toBe(true);
		box.remove();
		expect(returnFocusTo(plain)).toBe(false);
		expect(focusSpotOf(document)).toBeNull();
	});

	test("an element that will not take focus is passed over", () => {
		const root = document.createElement("div");
		root.tabIndex = -1;
		root.innerHTML =
			'<button data-fw-focus="copy">Copy answer</button><button data-fw-focus="run-again">Run again</button>';
		document.body.append(root);
		const copy = root.querySelector("[data-fw-focus=copy]") as HTMLElement;
		copy.focus = () => {};
		const source = { runs: fixture("done").runs, view: fixture("done").view };
		const took = rescueFocus(root, "stop", source);
		expect(took.getAttribute("data-fw-focus")).toBe("run-again");
		expect(active()?.getAttribute("data-fw-focus")).toBe("run-again");
		root.remove();
	});
});
