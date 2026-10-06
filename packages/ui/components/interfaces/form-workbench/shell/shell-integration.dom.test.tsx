import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { FormSessionState } from "../contracts";
import { click, installWorkbenchDom, mountWorkbench } from "../testing/dom";
import { fakeActions } from "../testing/fake-actions";
import { fixture } from "../testing/fixtures";
import {
	DESKTOP_LAYOUT,
	PHONE_LAYOUT,
	type layoutFor,
	withLayout,
} from "../testing/layouts";

/*
 * The shell with the real Rail, Dock and Stage: the contracts between the lanes (focus hooks, file hooks,
 * the dock variants, the modal frame the rail's dialog uses) as they are today.
 */

const dom = installWorkbenchDom();
const { WorkbenchShell } = await import("./workbench-shell");

afterEach(dom.cleanup);
afterAll(dom.restore);

async function mount(
	state: FormSessionState,
	layout: ReturnType<typeof layoutFor> = DESKTOP_LAYOUT,
) {
	const fake = fakeActions();
	const navigated: unknown[] = [];
	const client = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	const view = await mountWorkbench(
		<QueryClientProvider client={client}>
			<WorkbenchShell
				state={state}
				actions={fake.actions}
				appId={state.form.appId}
				toolbarRef={undefined}
				navigate={(intent) => navigated.push(intent)}
			/>
		</QueryClientProvider>,
		{ layout },
	);
	const root = view.container.querySelector("[data-fw-root]") as HTMLElement;
	return { ...view, fake, navigated, root };
}

const active = () => document.activeElement;

describe("the shell with the real views", () => {
	test("split: the rail with its head and fields, the dock's Run, the stage", async () => {
		const { root } = await mount(fixture("idle"));
		const rail = root.querySelector('[data-fw-region="rail"]') as HTMLElement;
		expect(rail.textContent).toContain("Extract invoice");
		expect(
			rail.querySelector("[data-fw-focus='field:vendor_name']"),
		).not.toBeNull();
		expect(rail.querySelector("[data-fw-focus=run]")).not.toBeNull();
		expect(root.querySelector('[data-fw-region="stage"]')).not.toBeNull();
	});

	test("split: the rail is the column's first item and the dock takes its fill from what the rail shows", async () => {
		const { root } = await mount(fixture("small"));
		const column = root.querySelector('aside[data-fw-region="rail"]');
		const rail = column?.firstElementChild as HTMLElement;
		expect(column?.className).toContain("group/rail");
		expect(rail.dataset.fwTab).toBe("inputs");
		expect(rail.className).toContain("flex-[0_1_auto]");
		const dock = column?.querySelector<HTMLElement>("[data-fw-dock=rail]");
		const fill = dock?.className.split(" ") ?? [];
		expect(fill).toContain("bg-transparent");
		expect(fill).not.toContain("bg-card");
		expect(fill).toContain("group-has-[[data-fw-over]]/rail:bg-card");
		expect(fill).toContain("group-has-[[data-fw-tab=runs]]/rail:bg-card");
	});

	test("narrow: the Inputs pane with the phone dock", async () => {
		const state = withLayout(fixture("idle"), PHONE_LAYOUT);
		const { root } = await mount(state, PHONE_LAYOUT);
		expect(root.getAttribute("data-fw-layout")).toBe("single");
		expect(
			root.querySelector("[data-fw-focus='field:vendor_name']"),
		).not.toBeNull();
		expect(root.querySelector("[data-fw-focus=run]")).not.toBeNull();
	});

	test("a form without fields: the first-run card with the hero's Run", async () => {
		const { root, fake } = await mount(fixture("none"));
		const hero = root.querySelector("[data-fw-hero]") as HTMLElement;
		expect(hero.textContent).toContain("Triage selected request");
		const run = hero.querySelector("[data-fw-focus=run]") as HTMLElement;
		expect(run).not.toBeNull();
		await click(run);
		expect(fake.argsOf("run")).toHaveLength(1);
	});

	test("a repeat user's cursor starts in the form, ring hidden (S6)", async () => {
		const { root } = await mount(fixture("reopen"));
		const first = root.querySelector("[data-fw-focus='field:invoice_file']");
		expect(first).not.toBeNull();
		expect(active()).toBe(first);
		expect(root.hasAttribute("data-fw-quiet-focus")).toBe(true);
	});

	test("a chord pressed in a real field runs", async () => {
		const { root, fake } = await mount(fixture("idle"));
		const vendor = root.querySelector(
			"[data-fw-focus='field:vendor_name']",
		) as HTMLElement;
		const win = vendor.ownerDocument
			.defaultView as unknown as typeof globalThis;
		vendor.dispatchEvent(
			new win.KeyboardEvent("keydown", {
				key: "Enter",
				metaKey: true,
				bubbles: true,
				cancelable: true,
			}),
		);
		expect(fake.argsOf("run")).toEqual([[{ leaveAsIs: false, from: "chord" }]]);
	});

	test("the save dialog of the rail opens inside the interface", async () => {
		const { root } = await mount(fixture("preset-save"));
		const dialog = root.querySelector("[data-fw-modal]") as HTMLElement;
		expect(dialog).not.toBeNull();
		expect(dialog.closest("[data-fw-modal-layer]")).not.toBeNull();
		expect(dialog.textContent).toContain("Save inputs as a preset");
	});

	test("the shortcuts sheet and the leave dialog show for their overlays", async () => {
		const sheet = await mount(fixture("shortcuts"));
		expect(sheet.root.querySelector("[data-fw-modal] h2")?.textContent).toBe(
			"Keyboard shortcuts",
		);
		await sheet.unmount();
		const leave = await mount(fixture("leave"));
		expect(leave.root.querySelector("[data-fw-modal] h2")?.textContent).toBe(
			"Leave this form?",
		);
		expect(active()?.textContent).toBe("Stay");
	});

	test("the run strip, bar and body come with the stage", async () => {
		const { root } = await mount(fixture("done"));
		const stage = root.querySelector('[data-fw-region="stage"]') as HTMLElement;
		expect(stage.textContent).toContain("Run 14");
		expect(stage.textContent).toContain("Done in 48 s");
	});
});
