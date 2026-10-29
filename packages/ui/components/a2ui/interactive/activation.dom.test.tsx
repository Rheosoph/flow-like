// @vitest-environment happy-dom

import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppLinkComponent, ButtonComponent } from "../types";

const actions = vi.hoisted(() => ({
	trigger: vi.fn(),
	execute: vi.fn(),
}));

vi.mock("../ActionHandler", () => ({
	useComponentEventTrigger: () => actions.trigger,
	useExecuteAction: () => ({ executeAction: actions.execute }),
	useIsComponentTriggering: () => false,
	useMarkComponentTriggering: () => undefined,
}));
vi.mock("../DataContext", () => ({
	useData: () => ({
		resolve: (value: { literalString?: string; literalBool?: boolean }) =>
			value.literalString ?? value.literalBool,
	}),
}));
vi.mock("lucide-react/dynamic", () => ({
	DynamicIcon: () => null,
	iconNames: [],
}));

import { A2UIAppLink } from "./AppLink";
import { A2UIButton } from "./Button";

let root: Root;
let host: HTMLDivElement;

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	vi.clearAllMocks();
	host = document.createElement("div");
	document.body.appendChild(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(() => root.unmount());
	host.remove();
	vi.restoreAllMocks();
});

function button(person: string): ButtonComponent {
	return {
		id: "card-action",
		type: "button",
		label: { literalString: person },
		actions: [{ name: "workflow_event", context: { person } }],
	};
}

async function renderButton(component: ButtonComponent) {
	await act(() =>
		root.render(
			<A2UIButton
				component={component}
				componentId="card-action"
				surfaceId="people"
				renderChild={() => null}
			/>,
		),
	);
	return renderedButton();
}

function renderedButton(): HTMLButtonElement {
	const target = host.querySelector("button");
	if (!target) throw new Error("Expected a rendered button");
	return target;
}

async function press(target: HTMLElement, button = 0) {
	await act(() =>
		target.dispatchEvent(
			new PointerEvent("pointerdown", {
				bubbles: true,
				button,
				isPrimary: true,
				pointerId: 1,
			}),
		),
	);
}

async function click(target: HTMLElement) {
	await act(() =>
		target.dispatchEvent(
			new MouseEvent("click", {
				bubbles: true,
				cancelable: true,
				detail: 1,
			}),
		),
	);
}

describe("A2UI activation identity", () => {
	it("keeps the pressed action across a slow render without starting it early", async () => {
		const now = vi.spyOn(Date, "now").mockReturnValue(10_000);
		const original = button("Alice");
		const target = await renderButton(original);
		await press(target);
		expect(actions.trigger).not.toHaveBeenCalled();
		await renderButton(button("Bob"));
		now.mockReturnValue(25_000);
		await click(target);
		expect(actions.trigger).toHaveBeenCalledExactlyOnceWith("click", original);
		await click(target);
		expect(actions.trigger).toHaveBeenCalledTimes(1);
	});

	it("keeps a keyboard activation bound to the original action", async () => {
		const original = button("Alice");
		const target = await renderButton(original);
		await act(() =>
			target.dispatchEvent(
				new KeyboardEvent("keydown", {
					bubbles: true,
					key: " ",
				}),
			),
		);
		await renderButton(button("Bob"));
		await click(target);
		expect(actions.trigger).toHaveBeenCalledExactlyOnceWith("click", original);
	});

	it("does not transfer a press to a replacement card", async () => {
		const original = await renderButton(button("Alice"));
		await press(original);
		await act(() =>
			root.render(
				<A2UIButton
					key="replacement"
					component={button("Bob")}
					componentId="other-action"
					surfaceId="people"
					renderChild={() => null}
				/>,
			),
		);
		await click(renderedButton());
		expect(actions.trigger).not.toHaveBeenCalled();
	});

	it("clears cancelled and blurred presses and ignores secondary buttons", async () => {
		const target = await renderButton(button("Alice"));
		await press(target);
		await act(() =>
			target.dispatchEvent(
				new PointerEvent("pointercancel", { bubbles: true }),
			),
		);
		await click(target);
		await press(target);
		await act(() =>
			target.dispatchEvent(new FocusEvent("focusout", { bubbles: true })),
		);
		await click(target);
		await press(target, 2);
		await click(target);
		expect(actions.trigger).not.toHaveBeenCalled();
	});

	it("clears a press released outside and starts a fresh keyboard activation", async () => {
		const target = await renderButton(button("Alice"));
		await press(target);
		await act(() =>
			document.body.dispatchEvent(
				new PointerEvent("pointerup", { bubbles: true }),
			),
		);
		await click(target);
		expect(actions.trigger).not.toHaveBeenCalled();
		await press(target);
		const current = button("Bob");
		await renderButton(current);
		await act(() =>
			target.dispatchEvent(
				new KeyboardEvent("keydown", {
					bubbles: true,
					key: "Enter",
				}),
			),
		);
		await click(target);
		expect(actions.trigger).toHaveBeenCalledExactlyOnceWith("click", current);
	});

	it("navigates to the app that was pressed before the card changed", async () => {
		const appLink = (appId: string): AppLinkComponent => ({
			id: "app-card",
			type: "appLink",
			appId: { literalString: appId },
			target: { literalString: "overview" },
		});
		const render = async (appId: string) =>
			act(() =>
				root.render(
					<A2UIAppLink
						component={appLink(appId)}
						componentId="app-card"
						surfaceId="apps"
						renderChild={() => null}
					/>,
				),
			);
		await render("original");
		const target = renderedButton();
		await press(target);
		await render("replacement");
		await click(target);
		expect(actions.execute).toHaveBeenCalledExactlyOnceWith(
			{
				name: "navigate_app_overview",
				context: { appId: "original", eventId: undefined },
			},
			"app-card",
		);
	});
});
