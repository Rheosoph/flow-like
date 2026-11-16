import { afterAll, afterEach, describe, expect, test } from "bun:test";
import type {
	EventSectionId,
	IEventSection,
} from "../../../lib/event-sections";
import { installDom } from "../devices/testing/dom-harness";

const dom = installDom();
const { EventSectionRail } = await import("./event-section-rail");

afterEach(dom.cleanup);
afterAll(dom.restore);

const sections: IEventSection[] = [
	{ id: "identity", label: "Identity", blurb: "", icon: "file-text" },
	{ id: "hosting", label: "Hosting", blurb: "", icon: "globe" },
];

async function mountRail() {
	const render = (active: EventSectionId) => (
		<div data-shell style={{ overflow: "hidden", height: 600 }}>
			<div data-page style={{ overflowY: "auto", height: 500 }}>
				<EventSectionRail
					sections={sections}
					active={active}
					onSelect={() => {}}
					issues={[]}
				/>
			</div>
		</div>
	);
	const mounted = await dom.render(render("identity"));
	const shell = mounted.container.querySelector<HTMLElement>("[data-shell]");
	const page = mounted.container.querySelector<HTMLElement>("[data-page]");
	const rail = mounted.container.querySelector<HTMLElement>("nav");
	const hosting = rail?.querySelector<HTMLElement>('[data-section="hosting"]');
	if (!shell || !page || !rail || !hosting) {
		throw new Error("Missing event rail fixture");
	}
	Object.defineProperties(rail, {
		clientWidth: { configurable: true, value: 320 },
		scrollWidth: { configurable: true, value: 1200 },
	});
	rail.getBoundingClientRect = () => new DOMRect(40, -200, 320, 44);
	hosting.getBoundingClientRect = () =>
		new DOMRect(640 - rail.scrollLeft, -200, 120, 44);
	return {
		...mounted,
		shell,
		page,
		rail,
		selectHosting: () => mounted.rerender(render("hosting")),
	};
}

describe("EventSectionRail scrolling", () => {
	test("centers the selected chip without moving the page or clipped shell", async () => {
		const { shell, page, rail, selectHosting } = await mountRail();
		shell.scrollLeft = 24;
		shell.scrollTop = 10;
		page.scrollTop = 260;
		rail.scrollLeft = 160;

		await selectHosting();

		expect(rail.scrollLeft).toBe(500);
		expect(rail.scrollTop).toBe(0);
		expect(page.scrollTop).toBe(260);
		expect(shell.scrollLeft).toBe(24);
		expect(shell.scrollTop).toBe(10);
	});

	test("leaves the desktop rail and page scroll positions alone", async () => {
		const { page, rail, selectHosting } = await mountRail();
		Object.defineProperty(rail, "scrollWidth", { value: 320 });
		page.scrollTop = 260;

		await selectHosting();

		expect(rail.scrollLeft).toBe(0);
		expect(page.scrollTop).toBe(260);
	});
});
