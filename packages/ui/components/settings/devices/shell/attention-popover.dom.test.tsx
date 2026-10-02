import { afterAll, afterEach, describe, expect, test } from "bun:test";
import type { Freshness } from "../../../../lib/device-management/model/types";
import type { AttentionEntry } from "../primitives/attention-list";
import { byRole, click, installDom, queryByRole } from "../testing/dom-harness";

const dom = installDom();
const { ATTENTION_POPOVER_CAP, AttentionPopoverView, stampOf } = await import(
	"./attention-popover"
);

function entry(
	index: number,
	over: Partial<AttentionEntry> = {},
): AttentionEntry {
	return {
		id: `item-${index}`,
		severity: "warning",
		sentence: `Sentence ${index}`,
		...over,
	};
}

const ITEMS: AttentionEntry[] = [
	entry(1, { severity: "critical" }),
	entry(2, { severity: "critical" }),
	...Array.from({ length: 6 }, (_, index) => entry(index + 3)),
	entry(9, { severity: "notice" }),
	entry(10, { severity: "info" }),
];

const shownIds = (root: ParentNode) =>
	Array.from(root.querySelectorAll("[data-attention]"), (el) => {
		return el.getAttribute("data-attention");
	});

afterEach(dom.cleanup);
afterAll(dom.restore);

describe("attention popover", () => {
	test("shows the five most severe items and offers the rest on the fleet overview", async () => {
		let shown = 0;
		const { container } = await dom.render(
			<AttentionPopoverView
				items={ITEMS}
				critical={2}
				total={9}
				onShowAll={() => {
					shown += 1;
				}}
			/>,
		);
		expect(ATTENTION_POPOVER_CAP).toBe(5);
		expect(shownIds(container)).toEqual([
			"item-1",
			"item-2",
			"item-3",
			"item-4",
			"item-5",
		]);
		expect(byRole("heading", /Needs you/).textContent).toContain("9");
		expect(byRole("heading", /Needs you/).textContent).toContain("2 critical");
		expect(
			container
				.querySelector("[data-attention-list]")
				?.hasAttribute("data-compact"),
		).toBe(true);
		await click(byRole("button", "Show all 9 on Fleet overview"));
		expect(shown).toBe(1);
		expect(queryByRole("button", "All devices' attention")).toBeNull();
		expect(container.querySelectorAll("[data-dv-primary]")).toHaveLength(0);
	});

	test("nothing open: says so and offers no list link", async () => {
		const { container } = await dom.render(
			<AttentionPopoverView
				items={[]}
				critical={0}
				total={0}
				onShowAll={() => {}}
			/>,
		);
		expect(container.textContent).toContain("Nothing needs you right now.");
		expect(queryByRole("button", /Show all/)).toBeNull();
		expect(container.textContent).not.toContain("critical");
	});

	test("app scope: names the app and links to both lists", async () => {
		const calls: string[] = [];
		const { container } = await dom.render(
			<AttentionPopoverView
				items={ITEMS.slice(0, 2)}
				critical={2}
				total={2}
				appName="Invoice AI"
				onShowAll={() => calls.push("app")}
				onAllDevices={() => calls.push("all")}
			/>,
		);
		expect(byRole("heading", /Needs you in Invoice AI/)).toBeTruthy();
		await click(byRole("button", "Show all 2 for Invoice AI"));
		await click(byRole("button", "All devices' attention"));
		expect(calls).toEqual(["app", "all"]);
		expect(container.textContent).not.toContain("Fleet overview");
	});

	test("app scope with nothing open keeps the way to every device's attention", async () => {
		const { container } = await dom.render(
			<AttentionPopoverView
				items={[]}
				critical={0}
				total={0}
				appName="Invoice AI"
				onShowAll={() => {}}
				onAllDevices={() => {}}
			/>,
		);
		expect(container.textContent).toContain(
			"Nothing in Invoice AI needs you right now.",
		);
		expect(byRole("button", "All devices' attention")).toBeTruthy();
	});
});

describe("stampOf", () => {
	test("maps a model freshness to stamp props, keeping the failure", () => {
		const current: Freshness = {
			src: "snap",
			age: "current",
			at: 100,
			cadenceS: 60,
		};
		expect(stampOf(current)).toEqual({
			source: "snap",
			age: "current",
			observedAt: 100,
			cadenceSec: 60,
		});
		const failing: Freshness = {
			src: "hub",
			age: "error",
			dataFrom: 90,
			error: { code: "refresh_failed", retryAt: 130 },
			skewS: 200,
		};
		expect(stampOf(failing)).toEqual({
			source: "hub",
			age: "error",
			skewSec: 200,
			error: { dataFrom: 90, retryAt: 130 },
		});
	});
});
