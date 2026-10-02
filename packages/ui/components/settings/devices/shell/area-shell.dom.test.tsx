import { afterAll, afterEach, expect, test } from "bun:test";
import { installDom } from "../testing/dom-harness";

const dom = installDom();
const { AREA_MAIN_SELECTOR, AreaShell, PAGE_FILTER_SELECTOR } = await import(
	"./area-shell"
);

afterEach(dom.cleanup);
afterAll(dom.restore);

const parts = {
	topbar: <header data-part="topbar" />,
	rail: <aside data-part="rail" />,
	tray: <aside data-part="tray" />,
	statusBar: <footer data-part="status" />,
	banners: <p data-part="banner" />,
};

function order(root: Element): string[] {
	return Array.from(root.querySelectorAll("[data-part]")).map(
		(part) => part.getAttribute("data-part") ?? "",
	);
}

test("top bar, rail, page, tray and status bar stack in the spec's order", async () => {
	const view = await dom.render(
		<AreaShell frame="page" {...parts}>
			<section data-part="screen" />
		</AreaShell>,
	);
	const root = view.container.firstElementChild as HTMLElement;
	expect(order(root)).toEqual([
		"topbar",
		"rail",
		"banner",
		"screen",
		"tray",
		"status",
	]);
	const main = root.querySelector(AREA_MAIN_SELECTOR) as HTMLElement;
	expect(main.className).toContain("overflow-y-auto");
	expect(main.querySelector('[data-part="banner"]')).toBeTruthy();
	expect(main.querySelector('[data-part="rail"]')).toBeNull();
});

test("the area is a size container that never uses fixed positioning", async () => {
	const view = await dom.render(
		<AreaShell frame="page" {...parts}>
			<section />
		</AreaShell>,
	);
	const root = view.container.firstElementChild as HTMLElement;
	expect(root.className).toContain("@container/devices");
	expect(root.className).toContain("overflow-hidden");
	for (const element of [root, ...Array.from(root.querySelectorAll("*"))])
		expect(element.className).not.toMatch(/(^|\s|:)fixed(\s|$)/);
});

test("the account page draws its frame; the app card relies on the config card", async () => {
	const page = await dom.render(
		<AreaShell frame="page" topbar={null}>
			<section />
		</AreaShell>,
	);
	const pageRoot = page.container.firstElementChild as HTMLElement;
	expect(pageRoot.getAttribute("data-frame")).toBe("page");
	expect(pageRoot.className).toContain("border-border");
	expect(pageRoot.className).toContain("h-full");

	const card = await dom.render(
		<AreaShell frame="card" topbar={null}>
			<section />
		</AreaShell>,
	);
	const cardRoot = card.container.firstElementChild as HTMLElement;
	expect(cardRoot.getAttribute("data-frame")).toBe("card");
	expect(cardRoot.className).not.toContain("border-border");
});

test("another object scrolls the page back to the top", async () => {
	const view = await dom.render(
		<AreaShell frame="page" topbar={null} scrollKey="device:a">
			<section />
		</AreaShell>,
	);
	const main = view.container.querySelector(AREA_MAIN_SELECTOR) as HTMLElement;
	main.scrollTop = 240;
	await view.rerender(
		<AreaShell frame="page" topbar={null} scrollKey="device:a">
			<section />
		</AreaShell>,
	);
	expect(main.scrollTop).toBe(240);
	await view.rerender(
		<AreaShell frame="page" topbar={null} scrollKey="device:b">
			<section />
		</AreaShell>,
	);
	expect(main.scrollTop).toBe(0);
});

test("the root ref and the page filter marker are reachable", async () => {
	let rootElement: HTMLDivElement | null = null;
	const view = await dom.render(
		<AreaShell
			frame="page"
			topbar={null}
			rootRef={(element) => {
				rootElement = element;
			}}
		>
			<input data-devices-filter="" aria-label="Filter devices" />
		</AreaShell>,
	);
	expect(rootElement as HTMLDivElement | null).toBe(
		view.container.firstElementChild as HTMLDivElement,
	);
	expect(view.container.querySelector(PAGE_FILTER_SELECTOR)).toBeTruthy();
});
