import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act } from "react";
import type { FormSessionState, Overlay } from "../contracts";
import {
	byRole,
	click,
	clickByText,
	installWorkbenchDom,
	mountWorkbench,
	settle,
} from "../testing/dom";
import { fixture } from "../testing/fixtures";
import { DESKTOP_LAYOUT } from "../testing/layouts";
import { shellKit } from "./shell-test-kit";

const dom = installWorkbenchDom();
const { ShellFrame } = await import("./shell-frame");

afterEach(dom.cleanup);
afterAll(dom.restore);

const waiting = (): FormSessionState => {
	const state = fixture("done");
	return { ...state, runs: fixture("series").runs };
};

const withOverlay = (
	state: FormSessionState,
	overlay: Overlay | null,
): FormSessionState => ({ ...state, view: { ...state.view, overlay } });

async function mount(
	state: FormSessionState,
	options: Parameters<typeof shellKit>[1] = {},
) {
	const kit = shellKit(state, options);
	const view = await mountWorkbench(<ShellFrame {...kit.props} />, {
		layout: DESKTOP_LAYOUT,
	});
	const root = view.container.querySelector("[data-fw-root]") as HTMLElement;
	const dialog = () =>
		root.querySelector("[data-fw-modal]") as HTMLElement | null;
	return { ...kit, view, root, dialog };
}

const active = () => document.activeElement;
const pressKey = async (target: Element, key: string, shift = false) => {
	const win = target.ownerDocument.defaultView as unknown as typeof globalThis;
	const event = new win.KeyboardEvent("keydown", {
		key,
		shiftKey: shift,
		bubbles: true,
		cancelable: true,
	});
	await act(async () => {
		target.dispatchEvent(event);
	});
	return event;
};

describe("route buttons", () => {
	test("leave at once when nothing waits", async () => {
		const { navigated, fake, root } = await mount(fixture("done"));
		await clickByText("Review queue", root);
		expect(navigated).toEqual([{ route: "/review", replace: false }]);
		expect(fake.argsOf("openOverlay")).toEqual([]);
	});

	test("ask first while a run waits or next files are left", async () => {
		const { navigated, fake, root } = await mount(waiting());
		await clickByText("Home", root);
		expect(navigated).toEqual([]);
		expect(fake.argsOf("openOverlay")).toEqual([
			[{ id: "leave", route: "/", replace: false }],
		]);
	});

	test("a form with one route shows one pill, three or more a Navigate menu", async () => {
		const one = await mount(fixture("none"));
		const pill = one.root.querySelector("[data-fw-route-bar] button");
		expect(pill?.textContent).toBe("Support chat");
		await one.view.unmount();

		const many = await mount(fixture("large"));
		const trigger = many.root.querySelector("[data-fw-route-bar] button");
		expect(trigger?.textContent).toBe("Navigate");
	});

	test("the pill and the Navigate trigger are 13 px at weight 500, as the canvas header draws them", async () => {
		const classesOf = async (name: "none" | "large") => {
			const mounted = await mount(fixture(name));
			const button = mounted.root.querySelector("[data-fw-route-bar] button");
			const classes = (button?.getAttribute("class") ?? "").split(/\s+/);
			await mounted.view.unmount();
			return classes;
		};
		for (const classes of [await classesOf("none"), await classesOf("large")]) {
			expect(classes).toContain("text-[13px]");
			expect(classes).not.toContain("text-sm");
			expect(classes).toContain("font-medium");
		}
	});

	test("the buttons in the host's header read the newest state without being pushed again", async () => {
		const state = fixture("done");
		const kit = shellKit(state, { withToolbar: true });
		const view = await mountWorkbench(<ShellFrame {...kit.props} />, {
			layout: DESKTOP_LAYOUT,
		});
		expect(kit.pushed).toHaveLength(1);
		const header = await dom.render(<nav>{kit.pushed[0]}</nav>);

		await view.rerender(<ShellFrame {...kit.props} state={waiting()} />);
		const home = Array.from(header.container.querySelectorAll("button")).find(
			(button) => button.textContent === "Home",
		) as HTMLElement;
		await click(home);
		expect(kit.navigated).toEqual([]);
		expect(kit.fake.argsOf("openOverlay")).toEqual([
			[{ id: "leave", route: "/", replace: false }],
		]);
	});
});

describe("the Navigate menu", () => {
	test("choosing a route while runs wait ends with focus in the dialog, not back on the menu button", async () => {
		const state: FormSessionState = {
			...fixture("large"),
			runs: fixture("series").runs,
		};
		const kit = shellKit(state, {
			withToolbar: true,
			routeLabels: { "/": "Home", "/reports": "Reports", "/orders": "Orders" },
		});
		const view = await mountWorkbench(<ShellFrame {...kit.props} />, {
			layout: DESKTOP_LAYOUT,
		});
		const header = await dom.render(<nav>{kit.pushed[0]}</nav>);
		await click(header.container.querySelector("button") as Element);
		await click(byRole("menuitem", "Reports"));
		expect(kit.navigated).toEqual([]);
		expect(kit.fake.argsOf("openOverlay")).toEqual([
			[{ id: "leave", route: "/reports", replace: false }],
		]);
		await view.rerender(
			<ShellFrame
				{...kit.props}
				state={withOverlay(state, {
					id: "leave",
					route: "/reports",
					replace: false,
				})}
			/>,
		);
		await settle();
		expect(active()?.textContent).toBe("Stay");
	});
});

describe("the leave dialog", () => {
	const leaving = () =>
		withOverlay(waiting(), { id: "leave", route: "/", replace: false });

	test("asks what the spec asks and names what would be lost", async () => {
		const { dialog } = await mount(fixture("leave"));
		const box = dialog();
		expect(box).not.toBeNull();
		const title = box?.querySelector("h2");
		expect(title?.textContent).toBe("Leave this form?");
		expect(box?.getAttribute("aria-labelledby")).toBe(title?.id);
		expect(box?.getAttribute("aria-modal")).toBe("true");
		expect(box?.textContent).toContain(
			"1 run will not start and 5 next files will be removed.",
		);
	});

	test("Stay is the coral button and has focus; Leave is the secondary one", async () => {
		const { dialog } = await mount(fixture("leave"));
		const buttons = Array.from(
			dialog()?.querySelectorAll<HTMLElement>("button") ?? [],
		);
		expect(buttons.map((button) => button.textContent)).toEqual([
			"Leave",
			"Stay",
		]);
		expect(buttons[0].getAttribute("data-variant")).toBe("default");
		expect(buttons[1].getAttribute("data-variant")).toBe("primary");
		expect(active()).toBe(buttons[1]);
	});

	test("lives inside the interface: the scrim covers the interface, not the page", async () => {
		const { dialog, root } = await mount(fixture("leave"));
		const layer = root.querySelector("[data-fw-modal-layer]");
		expect(layer).not.toBeNull();
		expect(layer?.contains(dialog())).toBe(true);
		expect(layer?.querySelector("[data-fw-scrim]")).not.toBeNull();
		expect(root.contains(layer)).toBe(true);
	});

	test("Esc stays", async () => {
		const { dialog, fake, navigated } = await mount(fixture("leave"));
		const event = await pressKey(
			dialog()?.querySelector("button") as Element,
			"Escape",
		);
		expect(event.defaultPrevented).toBe(true);
		expect(fake.calls.map((call) => call.name)).toEqual(["closeOverlay"]);
		expect(navigated).toEqual([]);
	});

	test("Stay and a click on the scrim stay", async () => {
		const first = await mount(fixture("leave"));
		await clickByText("Stay", first.root);
		expect(first.fake.calls.map((call) => call.name)).toEqual(["closeOverlay"]);
		await first.view.unmount();

		const second = await mount(fixture("leave"));
		await click(second.root.querySelector("[data-fw-scrim]") as Element);
		expect(second.fake.calls.map((call) => call.name)).toEqual([
			"closeOverlay",
		]);
		expect(second.navigated).toEqual([]);
	});

	test("Leave closes the dialog and goes where the button pointed", async () => {
		const state = withOverlay(waiting(), {
			id: "leave",
			route: "/review",
			replace: false,
		});
		const { root, fake, navigated } = await mount(state);
		await clickByText("Leave", root);
		expect(fake.calls.map((call) => call.name)).toEqual(["closeOverlay"]);
		expect(navigated).toEqual([{ route: "/review", replace: false }]);
	});

	test("Tab stays inside the dialog and wraps", async () => {
		const { dialog } = await mount(leaving());
		const [leave, stay] = Array.from(
			dialog()?.querySelectorAll<HTMLElement>("button") ?? [],
		);
		expect(active()).toBe(stay);
		const forward = await pressKey(stay, "Tab");
		expect(forward.defaultPrevented).toBe(true);
		expect(active()).toBe(leave);
		const back = await pressKey(leave, "Tab", true);
		expect(back.defaultPrevented).toBe(true);
		expect(active()).toBe(stay);
	});

	test("no chord of the form works behind it", async () => {
		const { dialog, fake } = await mount(leaving());
		const win = document.defaultView as unknown as typeof globalThis;
		await act(async () => {
			dialog()
				?.querySelector("button")
				?.dispatchEvent(
					new win.KeyboardEvent("keydown", {
						key: "Enter",
						metaKey: true,
						bubbles: true,
						cancelable: true,
					}),
				);
		});
		expect(fake.argsOf("run")).toEqual([]);
	});

	test("focus returns to the button that opened it", async () => {
		const state = waiting();
		const kit = shellKit(state);
		const view = await mountWorkbench(<ShellFrame {...kit.props} />, {
			layout: DESKTOP_LAYOUT,
		});
		const root = view.container.querySelector("[data-fw-root]") as HTMLElement;
		const home = Array.from(root.querySelectorAll("button")).find(
			(button) => button.textContent === "Home",
		) as HTMLElement;
		home.focus();
		await view.rerender(
			<ShellFrame
				{...kit.props}
				state={withOverlay(state, { id: "leave", route: "/", replace: false })}
			/>,
		);
		expect(root.querySelector("[data-fw-modal]")).not.toBeNull();
		expect(active()).not.toBe(home);
		await view.rerender(<ShellFrame {...kit.props} state={state} />);
		expect(root.querySelector("[data-fw-modal]")).toBeNull();
		expect(active()).toBe(home);
	});

	test("its words follow the queue while it is open, and it is a plain question when nothing waits", async () => {
		const state = leaving();
		const kit = shellKit(state);
		const view = await mountWorkbench(<ShellFrame {...kit.props} />, {
			layout: DESKTOP_LAYOUT,
		});
		const root = view.container.querySelector("[data-fw-modal]") as HTMLElement;
		expect(root.textContent).toContain("will not start");
		const calm = withOverlay(fixture("done"), {
			id: "leave",
			route: "/",
			replace: false,
		});
		await view.rerender(<ShellFrame {...kit.props} state={calm} />);
		const after = view.container.querySelector(
			"[data-fw-modal]",
		) as HTMLElement;
		expect(after.querySelector("p")).toBeNull();
		expect(after.querySelector("h2")?.textContent).toBe("Leave this form?");
	});
});
