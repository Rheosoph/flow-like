import { afterAll, afterEach, expect, test } from "bun:test";
import type { LocalSummary } from "../../../../lib/device-management/workspace/types";
import { byRole, installDom, queryByRole } from "../testing/dom-harness";

const dom = installDom();
const { MemoryDevicesRoute } = await import("../routing/use-devices-route");
const { PlatformBadge, keysAtRisk } = await import("./platform-badge");

afterEach(dom.cleanup);
afterAll(dom.restore);

async function mount(
	platform: LocalSummary["platform"],
	persistence: LocalSummary["persistence"],
) {
	return dom.render(
		<MemoryDevicesRoute>
			<PlatformBadge platform={platform} persistence={persistence} />
		</MemoryDevicesRoute>,
	);
}

test("the desktop app shows no badge", async () => {
	const view = await mount("desktop", "denied");
	expect(view.container.innerHTML).toBe("");
});

test("only a browser that won't keep the keys counts as a risk", () => {
	expect(keysAtRisk("denied")).toBe(true);
	expect(keysAtRisk("unavailable")).toBe(true);
	expect(keysAtRisk("persisted")).toBe(false);
	expect(keysAtRisk("unknown")).toBe(false);
});

test("web with kept keys is a plain chip, not a link", async () => {
	for (const persistence of ["persisted", "unknown"] as const) {
		const view = await mount("web", persistence);
		const chip = view.container.querySelector('[data-platform="web"]');
		expect(chip?.textContent).toBe("Web");
		expect(chip?.getAttribute("data-tone")).toBe("outline");
		expect(queryByRole("link")).toBeNull();
		await dom.cleanup();
	}
});

test("web with keys the browser may delete warns and opens Keys & recovery", async () => {
	for (const persistence of ["denied", "unavailable"] as const) {
		await mount("web", persistence);
		const badge = byRole("link", "Web · keys can be deleted by the browser");
		expect(badge.getAttribute("href")).toBe("/settings/devices?view=keys");
		expect(badge.getAttribute("title")).toContain(
			"Back them up or use the desktop app.",
		);
		expect(badge.querySelector("[data-tone]")?.getAttribute("data-tone")).toBe(
			"warning",
		);
		expect(badge.className).toContain("@max-[480px]/devices:hidden");
		await dom.cleanup();
	}
});
