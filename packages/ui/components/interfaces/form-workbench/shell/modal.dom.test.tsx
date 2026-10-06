import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { type ReactNode, act, useState } from "react";
import type { FormSessionState } from "../contracts";
import {
	click,
	installWorkbenchDom,
	mountWorkbench,
	settle,
} from "../testing/dom";
import { fixture } from "../testing/fixtures";
import { DESKTOP_LAYOUT } from "../testing/layouts";
import { shellKit } from "./shell-test-kit";

const dom = installWorkbenchDom();
const { InterfaceModal, ModalLayerContext } = await import("./interface-modal");
const { ShellFrame } = await import("./shell-frame");

afterEach(dom.cleanup);
afterAll(dom.restore);

const active = () => document.activeElement;

async function press(
	target: Element,
	key: string,
	init: KeyboardEventInit = {},
) {
	const win = target.ownerDocument.defaultView as unknown as typeof globalThis;
	const event = new win.KeyboardEvent("keydown", {
		key,
		bubbles: true,
		cancelable: true,
		...init,
	});
	await act(async () => {
		target.dispatchEvent(event);
	});
	return event;
}

interface HarnessProps {
	readonly onClose: () => void;
	readonly placement?: "top" | "center";
	readonly initiallyOpen?: boolean;
	readonly children?: ReactNode;
}

function Harness({
	onClose,
	placement,
	initiallyOpen = true,
	children,
}: Readonly<HarnessProps>) {
	const [open, setOpen] = useState(initiallyOpen);
	return (
		<div data-fw-root="" tabIndex={-1}>
			<button type="button" data-testid="opener" onClick={() => setOpen(true)}>
				Open
			</button>
			<InterfaceModal
				open={open}
				onClose={() => {
					onClose();
					setOpen(false);
				}}
				labelledBy="title"
				width={480}
				placement={placement}
			>
				<h2 id="title">Save inputs as a preset</h2>
				{children ?? (
					<>
						<input
							aria-label="Name"
							defaultValue="Nordwind"
							data-autofocus=""
						/>
						<button type="button">Cancel</button>
						<button type="button">Save preset</button>
					</>
				)}
			</InterfaceModal>
		</div>
	);
}

describe("InterfaceModal", () => {
	test("renders nothing while closed", async () => {
		const view = await dom.render(
			<Harness onClose={() => {}} initiallyOpen={false} />,
		);
		expect(view.container.querySelector("[role=dialog]")).toBeNull();
	});

	test("is a named modal dialog with a scrim, a width ceiling and the top placement by default", async () => {
		const view = await dom.render(<Harness onClose={() => {}} />);
		const dialog = view.container.querySelector("[role=dialog]") as HTMLElement;
		expect(dialog.getAttribute("aria-modal")).toBe("true");
		expect(dialog.getAttribute("aria-labelledby")).toBe("title");
		expect(dialog.style.width).toBe("480px");
		expect(dialog.getAttribute("data-placement")).toBe("top");
		expect(dialog.className).toContain("max-w-full");
		expect(view.container.querySelector("[data-fw-scrim]")).not.toBeNull();
		expect(dialog.parentElement?.className).toContain("pt-[72px]");
	});

	test("can be centred", async () => {
		const view = await dom.render(
			<Harness onClose={() => {}} placement="center" />,
		);
		const dialog = view.container.querySelector("[role=dialog]") as HTMLElement;
		expect(dialog.getAttribute("data-placement")).toBe("center");
		expect(dialog.parentElement?.className).toContain("items-center");
	});

	test("focus goes to the marked control, whose text is selected; else to the first control", async () => {
		const view = await dom.render(<Harness onClose={() => {}} />);
		const input = view.container.querySelector("input") as HTMLInputElement;
		expect(active()).toBe(input);
		expect(input.selectionStart).toBe(0);
		expect(input.selectionEnd).toBe("Nordwind".length);
		await view.unmount();

		const plain = await dom.render(
			<Harness onClose={() => {}}>
				<button type="button">First</button>
				<button type="button">Second</button>
			</Harness>,
		);
		expect(active()?.textContent).toBe("First");
	});

	test("Esc closes it, once", async () => {
		let closed = 0;
		const view = await dom.render(<Harness onClose={() => closed++} />);
		const event = await press(active() as Element, "Escape");
		expect(event.defaultPrevented).toBe(true);
		expect(closed).toBe(1);
		expect(view.container.querySelector("[role=dialog]")).toBeNull();
	});

	test("an inner layer that handled Esc keeps the dialog open", async () => {
		let closed = 0;
		const view = await dom.render(
			<Harness onClose={() => closed++}>
				<div onKeyDown={(event) => event.preventDefault()} data-testid="inner">
					<button type="button">Inside</button>
				</div>
			</Harness>,
		);
		await press(
			view.container.querySelector(
				"button[type=button]:not([data-testid])",
			) as Element,
			"Escape",
		);
		expect(closed).toBe(0);
		expect(view.container.querySelector("[role=dialog]")).not.toBeNull();
	});

	test("Esc during an IME composition is the IME's", async () => {
		let closed = 0;
		await dom.render(<Harness onClose={() => closed++} />);
		await press(active() as Element, "Escape", { isComposing: true });
		expect(closed).toBe(0);
	});

	test("a click on the scrim closes it, a click inside does not", async () => {
		let closed = 0;
		const view = await dom.render(<Harness onClose={() => closed++} />);
		await click(view.container.querySelector("h2") as Element);
		expect(closed).toBe(0);
		await click(view.container.querySelector("[data-fw-scrim]") as Element);
		expect(closed).toBe(1);
	});

	test("Tab and Shift+Tab wrap inside the dialog", async () => {
		const view = await dom.render(<Harness onClose={() => {}} />);
		const controls = Array.from(
			view.container.querySelectorAll<HTMLElement>(
				"[role=dialog] input, [role=dialog] button",
			),
		);
		const first = controls[0];
		const last = controls[controls.length - 1];
		last.focus();
		const forward = await press(last, "Tab");
		expect(forward.defaultPrevented).toBe(true);
		expect(active()).toBe(first);
		const backward = await press(first, "Tab", { shiftKey: true });
		expect(backward.defaultPrevented).toBe(true);
		expect(active()).toBe(last);
		const middle = controls[1];
		middle.focus();
		const inside = await press(middle, "Tab");
		expect(inside.defaultPrevented).toBe(false);
	});

	test("focus returns to the opener when it closes", async () => {
		const view = await dom.render(
			<Harness onClose={() => {}} initiallyOpen={false} />,
		);
		const opener = view.container.querySelector(
			"[data-testid=opener]",
		) as HTMLElement;
		opener.focus();
		await click(opener);
		expect(view.container.querySelector("[role=dialog]")).not.toBeNull();
		expect(active()).not.toBe(opener);
		await press(active() as Element, "Escape");
		await settle();
		expect(view.container.querySelector("[role=dialog]")).toBeNull();
		expect(active()).toBe(opener);
	});

	test("with a modal layer it portals into it", async () => {
		const layer = document.createElement("div");
		document.body.append(layer);
		const view = await dom.render(
			<ModalLayerContext.Provider value={layer}>
				<Harness onClose={() => {}} />
			</ModalLayerContext.Provider>,
		);
		expect(layer.querySelector("[role=dialog]")).not.toBeNull();
		expect(view.container.querySelector("[role=dialog]")).toBeNull();
	});
});

describe("the shortcuts sheet", () => {
	async function sheet(
		state: FormSessionState,
		options: Parameters<typeof shellKit>[1] = {},
	) {
		const kit = shellKit(state, options);
		const view = await mountWorkbench(<ShellFrame {...kit.props} />, {
			layout: DESKTOP_LAYOUT,
		});
		const dialog = view.container.querySelector(
			"[data-fw-modal]",
		) as HTMLElement | null;
		return { ...kit, view, dialog };
	}

	const rowsOf = (dialog: HTMLElement) =>
		Array.from(dialog.querySelectorAll("li")).map((row) => ({
			action: row.querySelector("span")?.textContent,
			keys: Array.from(row.querySelectorAll("kbd")).map(
				(key) => key.textContent,
			),
		}));

	const groupsOf = (dialog: HTMLElement) =>
		Array.from(dialog.querySelectorAll("section h3")).map(
			(heading) => heading.textContent,
		);

	test("is a named dialog with the spec's title and line; the close button takes focus", async () => {
		const { dialog } = await sheet(fixture("shortcuts"));
		const box = dialog as HTMLElement;
		const title = box.querySelector("h2") as HTMLElement;
		expect(title.textContent).toBe("Keyboard shortcuts");
		expect(box.textContent).toContain(
			"They work while the cursor is in this form.",
		);
		expect(box.getAttribute("aria-labelledby")).toBe(title.id);
		expect(active()?.getAttribute("aria-label")).toBe("Close");
		expect(box.style.width).toBe("640px");
	});

	test("lists the form's groups with ⌘ chips on a Mac", async () => {
		const { dialog } = await sheet(fixture("shortcuts"));
		const box = dialog as HTMLElement;
		expect(groupsOf(box)).toEqual([
			"Run",
			"Move around",
			"Fill in faster",
			"Typing dates",
		]);
		const rows = rowsOf(box);
		expect(rows.slice(0, 3)).toEqual([
			{ action: "Run", keys: ["⌘↵"] },
			{ action: "Run and leave the inputs as they are", keys: ["⇧⌘↵"] },
			{
				action: "Stop the run on screen, or take it out of the queue",
				keys: ["⌘."],
			},
		]);
		expect(rows).toContainEqual({ action: "Reset this input", keys: ["⇧⌘⌫"] });
		expect(rows).toContainEqual({ action: "Choose files", keys: ["⌘O"] });
		expect(rows).toContainEqual({ action: "Open the calendar", keys: ["⌥↓"] });
		expect(rows).toContainEqual({ action: "Presets", keys: ["⌘P"] });
		expect(rows).toContainEqual({
			action: "Save the inputs as a preset",
			keys: ["⌘S"],
		});
		expect(rows).toContainEqual({
			action: "Next or previous field",
			keys: ["Tab", "⇧Tab"],
		});
	});

	test("typing dates read in the viewer's order against the form's last date", async () => {
		const { dialog } = await sheet(fixture("shortcuts"));
		const rows = rowsOf(dialog as HTMLElement);
		expect(rows).toContainEqual({
			action: "18 Sep 2026, in the month of the last date",
			keys: ["18"],
		});
		expect(rows).toContainEqual({ action: "18 Sep 2026", keys: ["18/9"] });
		expect(rows).toContainEqual({ action: "18 Sep 2026", keys: ["18/9/26"] });
		expect(rows).toContainEqual({
			action: "Counted from today",
			keys: ["today", "yesterday", "tomorrow"],
		});
	});

	test("Ctrl words outside macOS and in the footer", async () => {
		const state = fixture("shortcuts");
		const { dialog } = await sheet({
			...state,
			form: { ...state.form, viewer: { ...state.form.viewer, mac: false } },
		});
		const rows = rowsOf(dialog as HTMLElement);
		expect(rows[0]).toEqual({ action: "Run", keys: ["Ctrl+Enter"] });
		expect(rows).toContainEqual({
			action: "Reset this input",
			keys: ["Ctrl+Shift+Backspace"],
		});
		expect(dialog?.textContent).toContain("Open this list again with Ctrl+/.");
	});

	test("the footer names the key that opens it again", async () => {
		const { dialog } = await sheet(fixture("shortcuts"));
		expect(dialog?.textContent).toContain("Open this list again with ⌘/.");
	});

	test("a form without fields lists only Run and Move around", async () => {
		const state = fixture("none");
		const { dialog } = await sheet({
			...state,
			view: { ...state.view, overlay: { id: "shortcuts" } },
		});
		const box = dialog as HTMLElement;
		expect(groupsOf(box)).toEqual(["Run", "Move around"]);
		expect(rowsOf(box).map((row) => row.action)).toEqual([
			"Run",
			"Stop the run on screen, or take it out of the queue",
			"Previous or next run, in the run strip",
		]);
	});

	test("a big form adds the slash for Filter fields; a small one does not", async () => {
		const large = fixture("large");
		const big = await sheet({
			...large,
			view: { ...large.view, overlay: { id: "shortcuts" } },
		});
		expect(rowsOf(big.dialog as HTMLElement)).toContainEqual({
			action: "Filter fields",
			keys: ["/"],
		});
		await big.view.unmount();
		const small = await sheet(fixture("shortcuts"));
		expect(
			rowsOf(small.dialog as HTMLElement).some(
				(row) => row.action === "Filter fields",
			),
		).toBe(false);
	});

	test("the close button, Esc and ⌘/ close it", async () => {
		const byButton = await sheet(fixture("shortcuts"));
		await click(
			byButton.dialog?.querySelector("button[aria-label=Close]") as Element,
		);
		expect(byButton.fake.calls.map((call) => call.name)).toEqual([
			"closeOverlay",
		]);
		await byButton.view.unmount();

		const byEsc = await sheet(fixture("shortcuts"));
		await press(active() as Element, "Escape");
		expect(byEsc.fake.calls.map((call) => call.name)).toEqual(["closeOverlay"]);
		await byEsc.view.unmount();

		const byChord = await sheet(fixture("shortcuts"));
		await press(active() as Element, "/", { metaKey: true });
		expect(byChord.fake.calls.map((call) => call.name)).toEqual([
			"closeOverlay",
		]);
	});

	test("is not in the page while it is closed", async () => {
		const { dialog } = await sheet(fixture("done"));
		expect(dialog).toBeNull();
	});
});
