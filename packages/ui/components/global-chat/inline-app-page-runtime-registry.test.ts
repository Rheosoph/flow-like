import { afterEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import {
	getInlineAppPageTarget,
	isInlineAppPagePlaced,
	observeInlineAppPageSlot,
	registerInlineAppPageRuntime,
	registerInlineAppPageSlot,
	subscribeInlineAppPagePlacement,
} from "./inline-app-page-runtime-registry";

const window = new Window({ url: "https://localhost" });
Object.assign(window, { SyntaxError, TypeError, Error });
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
			queryParams: {},
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
			queryParams: {},
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
			queryParams: {},
		});
		cleanups.push(runtime.unregister);

		runtime.updateTarget({
			routePath: "/reports",
			eventId: null,
			queryParams: { range: "week" },
		});
		expect(getInlineAppPageTarget("page-3")).toEqual({
			routePath: "/reports",
			eventId: null,
			queryParams: { range: "week" },
		});
		const readTarget = getInlineAppPageTarget("page-3");
		if (readTarget) readTarget.queryParams.range = "month";
		expect(getInlineAppPageTarget("page-3")?.queryParams.range).toBe("week");
	});

	test("parks while a mounted slot has no visible layout box", () => {
		const parking = element();
		const shell = element();
		const slot = element();
		const host = element();
		let hasSize = true;
		Object.defineProperty(slot, "getBoundingClientRect", {
			configurable: true,
			value: () =>
				({
					height: hasSize ? 320 : 0,
					width: hasSize ? 480 : 0,
				}) as DOMRect,
		});
		shell.appendChild(slot);
		window.document.body.append(parking as never, shell as never);

		const runtime = registerInlineAppPageRuntime("page-4", host, parking, {
			routePath: "/",
			eventId: "event-4",
			queryParams: {},
		});
		cleanups.push(runtime.unregister);
		const observation = observeInlineAppPageSlot("page-4", slot);
		cleanups.push(observation.disconnect);
		expect(host.parentElement).toBe(slot);

		shell.style.display = "none";
		observation.refresh();
		expect(host.parentElement).toBe(parking);
		expect(host.getAttribute("inert")).toBe("");

		shell.style.display = "block";
		hasSize = false;
		observation.refresh();
		expect(host.parentElement).toBe(parking);

		hasSize = true;
		observation.refresh();
		expect(host.parentElement).toBe(slot);
		expect(host.hasAttribute("inert")).toBe(false);
	});

	test("carries scroll offsets across a move that drops them", () => {
		const parking = element();
		const slot = element();
		const host = element();
		const scroller = element();
		host.appendChild(scroller);
		window.document.body.append(parking as never, slot as never);

		// Disconnecting a subtree drops its scroll offsets in real browsers; happy-dom keeps
		// them, so the drop is simulated on the destination to make the restore observable.
		const dropScrollOnAdopt = (destination: HTMLElement) => {
			const append = destination.appendChild.bind(destination);
			destination.appendChild = ((node: Node) => {
				scroller.scrollTop = 0;
				scroller.scrollLeft = 0;
				return append(node as never);
			}) as typeof destination.appendChild;
		};
		dropScrollOnAdopt(parking);
		dropScrollOnAdopt(slot);

		const runtime = registerInlineAppPageRuntime("page-5", host, parking, {
			routePath: "/",
			eventId: "event-5",
			queryParams: {},
		});
		cleanups.push(runtime.unregister);

		const removeSlot = registerInlineAppPageSlot("page-5", slot);
		scroller.scrollTop = 240;
		scroller.scrollLeft = 30;

		removeSlot();
		expect(host.parentElement).toBe(parking);
		expect(scroller.scrollTop).toBe(240);
		expect(scroller.scrollLeft).toBe(30);

		cleanups.push(registerInlineAppPageSlot("page-5", slot));
		expect(host.parentElement).toBe(slot);
		expect(scroller.scrollTop).toBe(240);
	});

	test("reports placement to subscribers and on unregister", () => {
		const parking = element();
		const slot = element();
		const host = element();
		window.document.body.append(parking as never, slot as never);

		const runtime = registerInlineAppPageRuntime("page-6", host, parking, {
			routePath: "/",
			eventId: "event-6",
			queryParams: {},
		});

		const seen: boolean[] = [];
		const unsubscribe = subscribeInlineAppPagePlacement("page-6", (placed) =>
			seen.push(placed),
		);
		cleanups.push(unsubscribe);

		expect(seen).toEqual([false]);
		expect(isInlineAppPagePlaced("page-6")).toBe(false);

		const removeSlot = registerInlineAppPageSlot("page-6", slot);
		expect(seen).toEqual([false, true]);
		expect(isInlineAppPagePlaced("page-6")).toBe(true);

		removeSlot();
		expect(seen).toEqual([false, true, false]);

		cleanups.push(registerInlineAppPageSlot("page-6", slot));
		expect(seen).toEqual([false, true, false, true]);

		runtime.unregister();
		expect(seen).toEqual([false, true, false, true, false]);
		expect(isInlineAppPagePlaced("page-6")).toBe(false);
	});
});
