import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	byRole,
	click,
	installDom,
} from "../settings/devices/testing/dom-harness";

const dom = installDom();
const { ThinkingPicker } = await import("./thinking-picker");

afterEach(dom.cleanup);
afterAll(dom.restore);

const OPTIONS = [
	{ id: "off", label: "Off", level: 0 as const },
	{ id: "low", label: "Low", description: "A short think.", level: 1 as const },
	{ id: "high", label: "High", level: 3 as const },
];

describe("thinking picker", () => {
	test("shows only the glyph until opened, then every level with its text", async () => {
		const picked: string[] = [];
		const { container } = await dom.render(
			<ThinkingPicker
				value="low"
				options={OPTIONS}
				onChange={(id) => picked.push(id)}
			/>,
		);

		const trigger = byRole("button", /^Thinking: Low$/, container);
		expect(trigger.textContent).toBe("");
		expect(trigger.querySelector(".fl-think-l1")).not.toBeNull();

		await click(trigger);
		expect(byRole("button", /A short think/).getAttribute("aria-pressed")).toBe(
			"true",
		);

		await click(byRole("button", /^High/));
		expect(picked).toEqual(["high"]);
	});

	test("renders nothing without options", async () => {
		const { container } = await dom.render(
			<ThinkingPicker value="" options={[]} onChange={() => {}} />,
		);
		expect(container.innerHTML).toBe("");
	});
});
