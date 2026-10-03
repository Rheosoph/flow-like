import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { ATTENTION_KEYS } from "../../../../lib/device-management/model/attention";
import type { DevicesT } from "../primitives/area-context";
import { byRole, click, fire, installDom } from "../testing/dom-harness";

const dom = installDom();
const { getI18n } = await import("@flow-like/locales");
const { RailRow, attentionShort, plainClick } = await import("./rail-row");

const t = getI18n().getFixedT("en", "devices") as DevicesT;

afterEach(dom.cleanup);
afterAll(dom.restore);

describe("rail row", () => {
	test("names the presence, the open items and the key state without colour alone", async () => {
		const { container } = await dom.render(
			<RailRow
				name="warehouse-pi"
				href="/settings/devices?device=d1"
				presence="offline"
				presenceLabel="Offline since 11:00"
				counts={{ critical: 2, warning: 1, notice: 0 }}
				keyState="unlocked"
				sub="Crashing · offline · 0.9.2"
				current
			/>,
		);
		const row = byRole("link");
		expect(row.getAttribute("aria-current")).toBe("page");
		expect(row.getAttribute("href")).toBe("/settings/devices?device=d1");
		expect(byRole("img", "Offline since 11:00")).toBeTruthy();
		const glyphs = container.querySelector("[data-rail-glyphs]");
		expect(glyphs?.textContent).toContain("2 critical");
		expect(glyphs?.textContent).toContain("1 warning");
		expect(glyphs?.textContent).not.toContain("notice");
		expect(glyphs?.textContent).toContain("Unlocked");
		expect(row.textContent).toContain("Crashing · offline · 0.9.2");
		expect(row.className).toContain("h-11");
	});

	test("a setup package carries its tag; a dimmed row says so", async () => {
		const { container } = await dom.render(
			<RailRow
				name="factory-line-3"
				tag="package"
				href="#"
				presence="pending"
				sub="Waiting · expires in 20 h"
				dim
			/>,
		);
		expect(container.textContent).toContain("package");
		expect(byRole("img", "Setup package")).toBeTruthy();
		expect(container.querySelector("[data-dim]")).not.toBeNull();
		expect(container.querySelector("[aria-current]")).toBeNull();
	});

	test("a plain click goes to the router; a modified click stays with the browser", async () => {
		const seen: boolean[] = [];
		await dom.render(
			<RailRow
				name="edge-berlin-01"
				href="#"
				presence="online"
				onSelect={(event) => {
					seen.push(plainClick(event));
					event.preventDefault();
				}}
			/>,
		);
		const row = byRole("link");
		await click(row);
		await fire(
			row,
			new window.MouseEvent("click", {
				bubbles: true,
				cancelable: true,
				metaKey: true,
			}) as unknown as Event,
		);
		expect(seen).toEqual([true, false]);
	});
});

describe("short attention reasons", () => {
	test("every condition has a few plain words and no machine key", () => {
		for (const key of ATTENTION_KEYS) {
			const reason = attentionShort(t, key);
			expect(reason.length).toBeGreaterThan(2);
			expect(reason.split(" ").length).toBeLessThanOrEqual(5);
			expect(reason).not.toContain("_");
			expect(reason).not.toBe(key);
		}
	});
});
