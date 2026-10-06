import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act } from "react";
import type { FileSlot, FormSessionState } from "../contracts";
import {
	allByRole,
	byRole,
	byText,
	click,
	dropFiles,
	fire,
	installWorkbenchDom,
	mountWorkbench,
	queryByRole,
	settle,
} from "../testing/dom";
import { PHONE_LAYOUT } from "../testing/layouts";
import { focusOn, press } from "./field-events";

const dom = installWorkbenchDom();
const { LiveField, liveStore, patchField } = await import("./field-testing");

afterEach(dom.cleanup);
afterAll(dom.restore);

type Store = ReturnType<typeof liveStore>;

async function mount(
	fixtureName: Parameters<typeof liveStore>[0],
	name: string,
	options: Parameters<typeof liveStore>[1] = {},
) {
	const store: Store = liveStore(fixtureName, options);
	const view = await mountWorkbench(<LiveField store={store} name={name} />, {
		layout: store.layout,
	});
	return { store, view };
}

const file = (name: string) =>
	new File(["x"], name, { type: "application/pdf" });

const hiddenInput = (name: string) =>
	document.querySelector(
		`input[data-fw-file-input="${name}"]`,
	) as HTMLInputElement;

/** Counts the clicks that would open the OS dialog. */
function spyOnDialog(input: HTMLInputElement) {
	const state = { opened: 0 };
	input.click = () => {
		state.opened += 1;
	};
	return state;
}

async function choose(input: HTMLInputElement, files: File[]) {
	Object.defineProperty(input, "files", { configurable: true, value: files });
	await fire(input, new window.Event("change", { bubbles: true }));
}

async function dragOver(target: Element, count: number | null) {
	const event = new window.Event("dragenter", {
		bubbles: true,
		cancelable: true,
	});
	Object.defineProperty(event, "dataTransfer", {
		value: {
			types: ["Files"],
			items: { length: count ?? 0 },
			files: [],
			dropEffect: "none",
		},
	});
	await fire(target, event);
}

const withSlot =
	(name: string, slot: FileSlot | null) => (state: FormSessionState) => ({
		...state,
		rail: { ...state.rail, values: { ...state.rail.values, [name]: slot } },
	});

describe("an empty file field", () => {
	test("the drop row is a button; the field carries the shell's hooks", async () => {
		await mount("idle", "invoice_file");
		const row = byRole("button", "Invoice. Enter or Space chooses a file.");
		expect(row.textContent).toBe("Drop a file or browse");
		expect(row.getAttribute("data-fw-focus")).toBe("field:invoice_file");
		const group = document.querySelector('[data-fw-file-field="invoice_file"]');
		expect(group?.hasAttribute("data-empty")).toBe(true);
		const input = hiddenInput("invoice_file");
		expect(input.hasAttribute("multiple")).toBe(true);
		expect(input.getAttribute("tabindex")).toBe("-1");
	});

	test("clicking the row opens the dialog; a pick replaces", async () => {
		const { store } = await mount("idle", "invoice_file");
		const input = hiddenInput("invoice_file");
		const dialog = spyOnDialog(input);
		await click(byRole("button", "Invoice. Enter or Space chooses a file."));
		expect(dialog.opened).toBe(1);
		const picked = [file("a.pdf"), file("b.pdf")];
		await choose(input, picked);
		expect(store.fake.argsOf("pickFiles")).toEqual([
			["invoice_file", picked, "replace"],
		]);
	});

	test("a drop picks the files", async () => {
		const { store } = await mount("idle", "invoice_file");
		const group = document.querySelector(
			'[data-fw-file-field="invoice_file"]',
		) as Element;
		const dropped = [file("a.pdf")];
		await dropFiles(group, dropped);
		expect(store.fake.argsOf("pickFiles")).toEqual([
			["invoice_file", dropped, "replace"],
		]);
	});

	test("dragging files over says what dropping does", async () => {
		await mount("idle", "invoice_file");
		const group = document.querySelector(
			'[data-fw-file-field="invoice_file"]',
		) as Element;
		await dragOver(group, 10);
		expect(byRole("button").textContent).toBe("Drop 10 files: one run each");
		await dragOver(group, null);
		expect(byRole("button").textContent).toBe("Drop the files: one run each");
		await dragOver(group, 1);
		expect(byRole("button").textContent).toBe("Drop a file or browse");
	});

	test("the hosted page takes one file per run: no promise of one run each", async () => {
		await mount("idle", "invoice_file", {
			change: (state) => ({
				...state,
				form: { ...state.form, host: { ...state.form.host, nextFiles: false } },
			}),
		});
		const group = document.querySelector(
			'[data-fw-file-field="invoice_file"]',
		) as Element;
		await dragOver(group, 10);
		expect(byRole("button").textContent).toBe("Drop a file or browse");
		expect(hiddenInput("invoice_file").hasAttribute("multiple")).toBe(false);
	});

	test("a required field that is missing shows its sentence; the optional one says Optional", async () => {
		await mount("idle", "invoice_file", {
			change: (state) => ({
				...state,
				rail: {
					...state.rail,
					problems: { invoice_file: { code: "required" } },
				},
			}),
		});
		expect(byRole("alert").textContent).toBe("Add the invoice file.");
	});

	test("a label that already says file does not say it twice", async () => {
		await mount("idle", "invoice_file", {
			change: (state) =>
				patchField("invoice_file", { label: "Invoice file" })({
					...state,
					rail: {
						...state.rail,
						problems: { invoice_file: { code: "required" } },
					},
				}),
		});
		expect(byRole("alert").textContent).toBe("Add the invoice file.");
	});
});

describe("several files", () => {
	test("rows, the drop row for more, a pick appends, × removes", async () => {
		const { store } = await mount("done", "supporting_documents");
		const rows = allByRole("button").filter((el) =>
			el.getAttribute("aria-label")?.startsWith("Supporting documents:"),
		);
		expect(rows).toHaveLength(2);
		const drop = byRole(
			"button",
			"Supporting documents. Enter or Space chooses files.",
		);
		expect(drop.textContent).toBe("Drop files or browse");
		expect(document.body.textContent).toContain("2 files");
		const input = hiddenInput("supporting_documents");
		expect(input.hasAttribute("multiple")).toBe(true);
		const picked = [file("c.pdf")];
		await choose(input, picked);
		expect(store.fake.argsOf("pickFiles")).toEqual([
			["supporting_documents", picked, "append"],
		]);
		await click(byRole("button", /^Remove Lieferschein/));
		expect(store.fake.argsOf("removeFile")[0][0]).toBe("supporting_documents");
	});

	test("Space on a row swaps just that file: the old one goes, the picks are appended", async () => {
		const { store } = await mount("done", "supporting_documents");
		const row = allByRole("button").find((el) =>
			el
				.getAttribute("aria-label")
				?.startsWith("Supporting documents: Lieferschein"),
		) as HTMLElement;
		const input = hiddenInput("supporting_documents");
		spyOnDialog(input);
		await click(row);
		const picked = [file("new.pdf")];
		await choose(input, picked);
		expect(store.fake.argsOf("removeFile")).toHaveLength(1);
		expect(store.fake.argsOf("pickFiles")).toEqual([
			["supporting_documents", picked, "append"],
		]);
	});
});

describe("an attached file (spec M2)", () => {
	test("the row is named with its file and keys, ↵ moves on, ⌫ removes, × is no Tab stop", async () => {
		const { store } = await mount("series", "invoice_file");
		const row = byRole(
			"button",
			"Invoice: invoice-RE-2026-0922.pdf, 1.1 MB. Space replaces it, Delete removes it.",
		);
		expect(row.getAttribute("data-fw-focus")).toBe("field:invoice_file");
		const enter = await press(row, "Enter");
		expect(enter.defaultPrevented).toBe(true);
		expect(store.fake.argsOf("enter")).toEqual([["invoice_file"]]);
		await press(row, "Backspace");
		await press(row, "Delete");
		const removed = store.fake.argsOf("removeFile");
		expect(removed.map(([name]) => name)).toEqual([
			"invoice_file",
			"invoice_file",
		]);
		const cross = byRole("button", "Remove invoice-RE-2026-0922.pdf");
		expect(cross.getAttribute("tabindex")).toBe("-1");
		expect(
			queryByRole("button", "Invoice. Enter or Space chooses a file."),
		).toBeNull();
	});

	test("⇧⌘⌫ and a held ↵ leave the file alone", async () => {
		const { store } = await mount("series", "invoice_file");
		const row = byRole("button", /^Invoice: invoice-RE-2026-0922/);
		await press(row, "Backspace", { shiftKey: true, metaKey: true });
		await press(row, "Enter", { repeat: true });
		await press(row, "Enter", { isComposing: true });
		expect(store.fake.argsOf("removeFile")).toEqual([]);
		expect(store.fake.argsOf("enter")).toEqual([]);
	});

	test("clicking or Space replaces: the dialog opens, a pick replaces", async () => {
		const { store } = await mount("series", "invoice_file");
		const dialog = spyOnDialog(hiddenInput("invoice_file"));
		await click(byRole("button", /^Invoice: invoice-RE-2026-0922/));
		expect(dialog.opened).toBe(1);
		const picked = [file("n.pdf")];
		await choose(hiddenInput("invoice_file"), picked);
		expect(store.fake.argsOf("pickFiles")).toEqual([
			["invoice_file", picked, "replace"],
		]);
	});

	test("dragging over it says Drop to replace the file", async () => {
		await mount("series", "invoice_file");
		const group = document.querySelector(
			'[data-fw-file-field="invoice_file"]',
		) as Element;
		await dragOver(group, 1);
		expect(
			allByRole("button").some(
				(el) => el.textContent === "Drop to replace the file",
			),
		).toBe(true);
		await dragOver(group, 10);
		expect(
			allByRole("button").some(
				(el) => el.textContent === "Drop 10 files: one run each",
			),
		).toBe(true);
	});

	test("a pick moves the cursor to the attached row, never to the page", async () => {
		const { store } = await mount("idle", "invoice_file");
		await focusOn(byRole("button", "Invoice. Enter or Space chooses a file."));
		await choose(hiddenInput("invoice_file"), [file("a.pdf")]);
		await act(async () => {
			store.update(
				withSlot("invoice_file", {
					id: "s1",
					name: "a.pdf",
					size: 1200,
					type: "application/pdf",
					state: "sent",
					progress: null,
					ref: null,
					error: null,
					sentAt: null,
					expiresAt: null,
				}),
			);
		});
		await settle();
		const row = byRole("button", /^Invoice: a\.pdf/);
		expect(dom.document.activeElement).toBe(row);
	});

	test("removing the last file moves the cursor to the drop row, never to the page", async () => {
		const { store } = await mount("series", "invoice_file", {
			change: (state) => ({
				...state,
				rail: { ...state.rail, nextFiles: {} },
			}),
		});
		const row = byRole("button", /^Invoice: invoice-RE-2026-0922/);
		await focusOn(row);
		await press(row, "Backspace");
		expect(store.fake.argsOf("removeFile")).toHaveLength(1);
		await act(async () => {
			store.update(withSlot("invoice_file", null));
		});
		await settle();
		const drop = byRole("button", "Invoice. Enter or Space chooses a file.");
		expect(dom.document.activeElement).toBe(drop);
	});

	test("a dialog that is cancelled leaves the cursor on the field", async () => {
		await mount("idle", "invoice_file");
		const drop = byRole("button", "Invoice. Enter or Space chooses a file.");
		await focusOn(drop);
		await fire(hiddenInput("invoice_file"), new window.Event("cancel"));
		expect(dom.document.activeElement).toBe(drop);
	});

	test("sizes read in the viewer's units, a sending row has its bar and word", async () => {
		await mount("next-files", "invoice_file");
		const row = byRole(
			"button",
			/^Invoice: invoice-RE-2026-0918\.pdf, Sending/,
		);
		expect(row.textContent).toContain("Sending");
		const bar = row.parentElement?.querySelector("span[style]") as HTMLElement;
		expect(bar.style.width).toBe("40%");
	});

	test("touch rows are 44 px", async () => {
		await mount("series", "invoice_file", { layout: PHONE_LAYOUT });
		const row = byRole("button", /^Invoice: invoice-RE-2026-0922/);
		expect(row.parentElement?.className).toContain("min-h-11");
	});
});

describe("next files (spec M3)", () => {
	test("the line names the next file and how many more, with Show", async () => {
		await mount("series", "invoice_file");
		const line = byText(/^Next:/);
		expect(line.parentElement?.textContent).toBe(
			"Next: invoice-RE-2026-0923.pdf · 4 more",
		);
		const show = byRole("button", /Show the next files/);
		expect(show.getAttribute("aria-expanded")).toBe("false");
		expect(show.textContent).toBe("Show");
	});

	test("Show opens the list through the reducer; Hide closes it", async () => {
		const { store } = await mount("series", "invoice_file");
		await click(byRole("button", /Show the next files/));
		expect(store.fake.argsOf("openList")).toEqual([
			["nextFiles", "invoice_file"],
		]);
		const hide = byRole("button", "Hide the next files");
		expect(hide.getAttribute("aria-expanded")).toBe("true");
		expect(hide.getAttribute("aria-controls")).not.toBeNull();
		await click(hide);
		expect(store.fake.argsOf("closeList")).toHaveLength(1);
	});

	test("the open list has the left-out note, rows with their state and the footer", async () => {
		const { store } = await mount("next-files", "invoice_file");
		const list = byRole("list", "Next files");
		const rows = Array.from(list.querySelectorAll("li"));
		expect(rows).toHaveLength(9);
		expect(rows[0].textContent).toContain("invoice-RE-2026-0919.pdf");
		expect(rows[0].textContent).toContain("Sending");
		expect(rows[1].textContent).toContain("Waiting");
		expect(document.body.textContent).toContain(
			"invoice-RE-2026-0917.pdf was already sent in run 14 and was left out.",
		);
		await click(byRole("button", "Add it"));
		expect(store.fake.argsOf("addLeftOut")).toEqual([["invoice_file"]]);
		await click(byText("Remove all next files"));
		expect(store.fake.argsOf("clearNextFiles")).toEqual([["invoice_file"]]);
	});

	test("× removes one next file; the cross is no Tab stop", async () => {
		const { store } = await mount("next-files", "invoice_file");
		const cross = byRole(
			"button",
			"Remove invoice-RE-2026-0919.pdf from the next files",
		);
		expect(cross.getAttribute("tabindex")).toBe("-1");
		await click(cross);
		expect(store.fake.argsOf("removeNextFile")[0][0]).toBe("invoice_file");
	});

	test("↓ on Hide goes into the list; ↓ ↑ move; ⌫ removes; Esc closes and returns to Hide", async () => {
		const { store } = await mount("next-files", "invoice_file");
		const hide = byRole("button", "Hide the next files");
		await focusOn(hide);
		await press(hide, "ArrowDown");
		const rows = Array.from(
			byRole("list", "Next files").querySelectorAll("li"),
		);
		expect(dom.document.activeElement).toBe(rows[0]);
		await press(rows[0], "ArrowDown");
		expect(dom.document.activeElement).toBe(rows[1]);
		await press(rows[1], "ArrowUp");
		expect(dom.document.activeElement).toBe(rows[0]);
		await press(rows[0], "Backspace");
		expect(store.fake.argsOf("removeNextFile")).toHaveLength(1);
		await press(rows[0], "Escape");
		expect(store.fake.argsOf("closeList")).toHaveLength(1);
		expect(dom.document.activeElement).toBe(hide);
	});

	test("a next file that failed shows Not sent with Try again", async () => {
		const { store } = await mount("next-files", "invoice_file", {
			change: (state) => {
				const [first, ...rest] = state.rail.nextFiles.invoice_file;
				const failed: FileSlot = {
					...first,
					state: "failed",
					progress: null,
					error: "boom",
				};
				return {
					...state,
					rail: {
						...state.rail,
						nextFiles: { invoice_file: [failed, ...rest] },
					},
				};
			},
		});
		expect(byRole("list", "Next files").textContent).toContain("Not sent");
		await click(byRole("button", "Try sending invoice-RE-2026-0919.pdf again"));
		expect(store.fake.argsOf("retryFile")).toHaveLength(1);
	});

	test("a several-files field and a blocked field show no Next line", async () => {
		await mount("done", "supporting_documents");
		expect(document.body.textContent).not.toContain("Next:");
	});
});

describe("a FlowPath field this host cannot fill (spec F)", () => {
	test("a disabled row in the drop row's place: no button, no Tab stop", async () => {
		await mount("hosted-files", "invoice_file");
		const group = document.querySelector(
			'[data-fw-file-field="invoice_file"]',
		) as HTMLElement;
		expect(group.getAttribute("aria-disabled")).toBe("true");
		expect(group.textContent).toBe("Files can't be sent from this page yet.");
		expect(group.querySelector("button")).toBeNull();
		const description =
			group.getAttribute("aria-describedby")?.split(" ") ?? [];
		const blocked = byText("Files can't be sent from this page yet.");
		expect(description).toContain(blocked.parentElement?.id ?? "");
	});

	test("dropping on it picks nothing", async () => {
		const { store } = await mount("hosted-files", "invoice_file");
		const group = document.querySelector(
			'[data-fw-file-field="invoice_file"]',
		) as Element;
		await dropFiles(group, [file("a.pdf")]);
		expect(store.fake.argsOf("pickFiles")).toEqual([]);
	});

	test("a required one explains itself under the field", async () => {
		await mount("hosted-files", "invoice_file", {
			change: (state) => ({
				...state,
				rail: {
					...state.rail,
					problems: { invoice_file: { code: "fileNotHere" } },
				},
			}),
		});
		expect(byRole("alert").textContent).toBe(
			"Invoice can't be sent from this page.",
		);
	});

	test("an optional several-files field shows the same row", async () => {
		await mount("hosted-files", "supporting_documents");
		expect(document.body.textContent).toContain(
			"Files can't be sent from this page yet.",
		);
	});
});

describe("Pick again (spec S4)", () => {
	test("a reminder row: dashed, named, with its own ×; Space opens the dialog, ⌫ removes it", async () => {
		const { store } = await mount("pick-again", "invoice_file");
		const row = byRole(
			"button",
			"Invoice: invoice-RE-2026-0902.pdf. Pick again.",
		);
		expect(row.textContent).toContain("Pick again");
		expect(row.parentElement?.getAttribute("title")).toBe(
			"Files from earlier runs can be sent again while this page is open.",
		);
		const dialog = spyOnDialog(hiddenInput("invoice_file"));
		await click(row);
		expect(dialog.opened).toBe(1);
		await press(row, "Backspace");
		expect(store.fake.argsOf("removeFile")).toHaveLength(1);
		const cross = byRole(
			"button",
			"Remove the reminder for invoice-RE-2026-0902.pdf",
		);
		expect(cross.getAttribute("tabindex")).toBe("-1");
	});

	test("several reminders and the drop row", async () => {
		await mount("pick-again", "supporting_documents");
		const reminders = allByRole("button").filter((el) =>
			el.textContent?.endsWith("Pick again"),
		);
		expect(reminders).toHaveLength(2);
		byRole("button", "Supporting documents. Enter or Space chooses files.");
	});

	test("the message asks to pick it again", async () => {
		await mount("pick-again", "invoice_file", {
			change: (state) => ({
				...state,
				rail: {
					...state.rail,
					problems: {
						invoice_file: {
							code: "pickAgain",
							fileName: "invoice-RE-2026-0902.pdf",
						},
					},
				},
			}),
		});
		expect(byRole("alert").textContent).toBe(
			"Pick invoice-RE-2026-0902.pdf again.",
		);
	});
});
