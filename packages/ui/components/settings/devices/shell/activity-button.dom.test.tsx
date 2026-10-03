import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { byRole, click, installDom } from "../testing/dom-harness";

const dom = installDom();
const { ActivityButtonView } = await import("./activity-button");
const { ACTIVITY_TRAY_ID } = await import("./activity-tray");

afterEach(dom.cleanup);
afterAll(dom.restore);

describe("activity button", () => {
	test("shows the in-flight count and controls the tray", async () => {
		let toggles = 0;
		const { container } = await dom.render(
			<ActivityButtonView
				inFlight={2}
				active={false}
				open={false}
				onToggle={() => {
					toggles += 1;
				}}
			/>,
		);
		const button = byRole("button", "Activity: 2 in progress");
		expect(button.getAttribute("aria-expanded")).toBe("false");
		expect(button.getAttribute("aria-controls")).toBe(ACTIVITY_TRAY_ID);
		expect(container.querySelector("[data-count=total]")?.textContent).toBe(
			"2",
		);
		expect(container.querySelector(".animate-spin")).toBeNull();
		await click(button);
		expect(toggles).toBe(1);
	});

	test("a spinner replaces the icon while something runs, and the button still toggles", async () => {
		let toggles = 0;
		const { container } = await dom.render(
			<ActivityButtonView
				inFlight={1}
				active
				open
				onToggle={() => {
					toggles += 1;
				}}
			/>,
		);
		const button = byRole("button", "Activity: 1 in progress");
		expect(button.hasAttribute("data-active")).toBe(true);
		expect(button.getAttribute("aria-expanded")).toBe("true");
		expect(button.getAttribute("aria-busy")).toBeNull();
		expect(container.querySelectorAll("svg")).toHaveLength(1);
		expect(container.querySelector(".animate-spin")).not.toBeNull();
		await click(button);
		expect(toggles).toBe(1);
	});
});
