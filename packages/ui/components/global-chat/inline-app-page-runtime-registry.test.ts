import { afterEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import {
	getInlineAppPageTarget,
	registerInlineAppPageRuntime,
	registerInlineAppPageSlot,
} from "./inline-app-page-runtime-registry";

const window = new Window({ url: "https://localhost" });
Object.assign(globalThis, {
	window,
	document: window.document,
	HTMLElement: window.HTMLElement,
	Element: window.Element,
	Node: window.Node,
});

const cleanups: (() => void)[] = [];

afterEach(() => {
	for (const cleanup of cleanups.splice(0).reverse()) cleanup();
	window.document.body.replaceChildren();
});

function element(): HTMLElement {
	return window.document.createElement("div") as unknown as HTMLElement;
}

describe("inline app page runtime placement", () => {
	test("moves one stable host from parking to a card slot and back", () => {
		const parking = element();
		const slot = element();
		const host = element();
		const pageContent = element();
		host.appendChild(pageContent);
		window.document.body.append(parking as never, slot as never);

		const runtime = registerInlineAppPageRuntime("page-1", host, parking, {
			routePath: "/",
			eventId: "event-1",
		});
		cleanups.push(runtime.unregister);

		expect(host.parentElement).toBe(parking);
		expect(host.getAttribute("inert")).toBe("");
		expect(host.getAttribute("aria-hidden")).toBe("true");

		const removeSlot = registerInlineAppPageSlot("page-1", slot);
		expect(host.parentElement).toBe(slot);
		expect(host.hasAttribute("inert")).toBe(false);
		expect(host.hasAttribute("aria-hidden")).toBe(false);
		expect(host.firstElementChild).toBe(pageContent);

		removeSlot();
		expect(host.parentElement).toBe(parking);
		expect(host.firstElementChild).toBe(pageContent);
		expect(host.dataset.flowpilotPageRuntimeState).toBe("parked");
	});

	test("returns to an earlier card slot when the latest slot unmounts", () => {
		const parking = element();
		const firstSlot = element();
		const secondSlot = element();
		const host = element();
		window.document.body.append(
			parking as never,
			firstSlot as never,
			secondSlot as never,
		);

		const runtime = registerInlineAppPageRuntime("page-2", host, parking, {
			routePath: "/orders",
			eventId: null,
		});
		cleanups.push(runtime.unregister);
		const removeFirst = registerInlineAppPageSlot("page-2", firstSlot);
		cleanups.push(removeFirst);
		const removeSecond = registerInlineAppPageSlot("page-2", secondSlot);

		expect(host.parentElement).toBe(secondSlot);
		removeSecond();
		expect(host.parentElement).toBe(firstSlot);
	});

	test("keeps the latest navigation target with the runtime", () => {
		const parking = element();
		const host = element();
		window.document.body.append(parking as never);
		const runtime = registerInlineAppPageRuntime("page-3", host, parking, {
			routePath: "/",
			eventId: "event-3",
		});
		cleanups.push(runtime.unregister);

		runtime.updateTarget({ routePath: "/reports", eventId: null });
		expect(getInlineAppPageTarget("page-3")).toEqual({
			routePath: "/reports",
			eventId: null,
		});
	});
});
