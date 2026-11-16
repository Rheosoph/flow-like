// @vitest-environment happy-dom

import type { ReactNode } from "react";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CardComponent } from "../types";

const actions = vi.hoisted(() => ({ trigger: vi.fn() }));
const assets = vi.hoisted(() => ({
	signed: new Map<string, string>(),
	pending: new Set<string>(),
}));

vi.mock("../ActionHandler", () => ({
	useComponentEventTrigger: () => actions.trigger,
}));
vi.mock("../hooks/use-asset-url", () => ({
	useAssetUrl: (source: string) => ({
		url: assets.signed.get(source),
		isLoading: assets.pending.has(source),
		refresh: () => {},
	}),
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

	it("shows a header image edge to edge, holds its slot while signing and drops it when it cannot load", async () => {
		assets.signed.set("bot/ix3.png", "https://signed.test/ix3.png");
		const withImage = (source: string): CardComponent =>
			({
				...card("iX3"),
				headerImage: { literalString: source },
			}) as CardComponent;

		const element = await renderCard(withImage("bot/ix3.png"), controls);
		const media = element.firstElementChild;
		expect(media?.getAttribute("data-slot")).toBe("card-media");
		expect(media?.querySelector("img")?.getAttribute("src")).toBe(
			"https://signed.test/ix3.png",
		);
		expect(media?.querySelector("img")?.getAttribute("alt")).toBe("iX3");
		expect(element.className).toContain("pt-0");

		assets.pending.add("bot/i3.png");
		await renderCard(withImage("bot/i3.png"), controls);
		expect(element.querySelector("[data-slot='card-media']")).not.toBeNull();
		expect(element.querySelector("[data-slot='card-media'] img")).toBeNull();

		assets.pending.delete("bot/i3.png");
		await renderCard(withImage("bot/i3.png"), controls);
		expect(element.querySelector("[data-slot='card-media']")).toBeNull();
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
