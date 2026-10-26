// @vitest-environment happy-dom

import type { ReactNode } from "react";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CardComponent } from "../types";

const actions = vi.hoisted(() => ({ trigger: vi.fn() }));

vi.mock("../ActionHandler", () => ({
	useComponentEventTrigger: () => actions.trigger,
}));
vi.mock("../DataContext", () => ({
	useData: () => ({
		resolve: (value: { literalString?: string; literalBool?: boolean }) =>
			value.literalString ?? value.literalBool,
	}),
}));

import { A2UICard } from "./Card";

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
});

function card(title?: string): CardComponent {
	return {
		type: "card",
		clickable: { literalBool: true },
		...(title ? { title: { literalString: title } } : {}),
		children: { explicitList: ["body"] },
	} as CardComponent;
}

async function renderCard(component: CardComponent, body: ReactNode) {
	await act(() =>
		root.render(
			<A2UICard
				component={component}
				componentId="rental"
				surfaceId="rentals"
				renderChild={() => body}
			/>,
		),
	);
	const element = host.querySelector<HTMLElement>("[data-slot='card']");
	if (!element) throw new Error("Expected a rendered card");
	return element;
}

const controls = (
	<div>
		<p>Musterstraße 1</p>
		<button type="button">Bearbeiten</button>
		<button type="button">Mietverlauf ansehen</button>
	</div>
);

async function click(target: Element) {
	await act(() =>
		target.dispatchEvent(
			new MouseEvent("click", { bubbles: true, cancelable: true }),
		),
	);
}

function byText(text: string): HTMLElement {
	const match = [...host.querySelectorAll<HTMLElement>("*")]
		.reverse()
		.find((element) => element.textContent === text);
	if (!match) throw new Error(`Expected an element with text "${text}"`);
	return match;
}

describe("A2UI clickable card semantics", () => {
	it("labels a titled card as a group and exposes the action on its title", async () => {
		const element = await renderCard(card("Wohnung 3B"), controls);

		expect(element.getAttribute("role")).toBe("group");
		expect(element.hasAttribute("tabindex")).toBe(false);
		const labelledBy = element.getAttribute("aria-labelledby");
		expect(labelledBy && document.getElementById(labelledBy)?.textContent).toBe(
			"Wohnung 3B",
		);
		expect(
			[...element.querySelectorAll("button")].map((b) => b.textContent),
		).toEqual(["Wohnung 3B", "Bearbeiten", "Mietverlauf ansehen"]);

		await click(byText("Wohnung 3B"));
		expect(actions.trigger).toHaveBeenCalledTimes(1);
	});

	it("keeps an untitled card with controls out of the button role", async () => {
		const element = await renderCard(card(), controls);

		expect(element.getAttribute("role")).toBe("group");
		expect(element.hasAttribute("tabindex")).toBe(false);

		await click(byText("Bearbeiten"));
		expect(actions.trigger).not.toHaveBeenCalled();
		await click(byText("Musterstraße 1"));
		expect(actions.trigger).toHaveBeenCalledTimes(1);
	});

	it("leaves an untitled card without controls a keyboard button", async () => {
		const element = await renderCard(card(), <p>Musterstraße 1</p>);

		expect(element.getAttribute("role")).toBe("button");
		expect(element.getAttribute("tabindex")).toBe("0");

		await act(() =>
			element.dispatchEvent(
				new KeyboardEvent("keydown", { bubbles: true, key: "Enter" }),
			),
		);
		expect(actions.trigger).toHaveBeenCalledTimes(1);
	});

	it("drops the button role once controls render into the card", async () => {
		const element = await renderCard(card(), <p>Musterstraße 1</p>);
		expect(element.getAttribute("role")).toBe("button");

		await renderCard(card(), controls);
		await act(async () => {});
		expect(element.getAttribute("role")).toBe("group");
	});

	it("runs the card action when an ancestor outside the card is a button", async () => {
		const outer = document.createElement("div");
		outer.setAttribute("role", "button");
		host.replaceWith(outer);
		outer.appendChild(host);
		const element = await renderCard(card(), controls);

		await click(element);
		expect(actions.trigger).toHaveBeenCalledTimes(1);
		outer.remove();
	});
});
