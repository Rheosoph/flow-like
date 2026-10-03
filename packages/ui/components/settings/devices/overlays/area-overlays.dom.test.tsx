import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act } from "react";
import { SAMPLE_IDS } from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import {
	allByRole,
	byRole,
	installDom,
	keyDown,
	queryByRole,
} from "../testing/dom-harness";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { useOverlayStore } = await import("../workspace/overlay-store");
const { AreaOverlays } = await import("./area-overlays");

const { lab, warehouse } = SAMPLE_IDS;
const store = () => useOverlayStore.getState();

async function request(run: () => void) {
	await act(async () => {
		run();
	});
}

afterEach(async () => {
	await request(() => store().close());
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

describe("area overlays", () => {
	test("renders nothing until a sheet is asked for", async () => {
		await mountDevices(<div />, { overlays: true });
		expect(queryByRole("dialog")).toBeNull();
		expect(document.body.querySelectorAll("[data-dv-primary]")).toHaveLength(0);
		expect(store().overlay.kind).toBe("none");
	});

	test("shows one sheet at a time: a new request replaces the open one", async () => {
		const mounted = await mountDevices(<div />, { overlays: true });
		const titles: string[] = [];
		const steps: (() => void)[] = [
			() => store().openUnlock(lab),
			() => store().openUnlockSeveral(),
			() => store().openDiagnose(warehouse),
			() => store().openPlane("live"),
			() =>
				store().openRunNow({
					deviceId: warehouse,
					serviceId: "notes",
					eventId: "evt_notes_form",
				}),
		];
		for (const step of steps) {
			await request(step);
			await mounted.settle();
			expect(allByRole("dialog")).toHaveLength(1);
			titles.push(
				byRole("heading", undefined, byRole("dialog")).textContent ?? "",
			);
			expect(
				byRole("dialog").querySelectorAll("[data-dv-primary]").length,
			).toBeLessThanOrEqual(1);
		}
		expect(titles).toEqual([
			"Unlock lab-gpu-02",
			"Unlock several devices",
			"Diagnose warehouse-pi",
			"Data sources",
			"Run an action or form on warehouse-pi",
		]);
	});

	test("Run now… opens for one event of one service, and a request for another run replaces it", async () => {
		const mounted = await mountDevices(<div />, { overlays: true });
		const target = { deviceId: warehouse, serviceId: "notes" };
		await request(() =>
			store().openRunNow({ ...target, eventId: "evt_notes_form" }),
		);
		await mounted.settle();
		expect(store().overlay).toEqual({
			kind: "run_now",
			...target,
			eventId: "evt_notes_form",
		});
		expect(
			byRole("dialog")
				.querySelector("[data-run-now-sheet]")
				?.getAttribute("data-run-now-sheet"),
		).toBe("evt_notes_form");
		await request(() =>
			store().openRunNow({
				...target,
				eventId: "evt_other",
				operationId: "op-1",
			}),
		);
		await mounted.settle();
		expect(allByRole("dialog")).toHaveLength(1);
		expect(store().overlay).toMatchObject({
			eventId: "evt_other",
			operationId: "op-1",
		});
	});

	test("Escape and the close button clear the request", async () => {
		const mounted = await mountDevices(<div />, { overlays: true });
		await request(() => store().openPlane("hub"));
		await mounted.settle();
		await keyDown(byRole("dialog"), "Escape");
		await mounted.settle();
		expect(store().overlay.kind).toBe("none");
		expect(queryByRole("dialog")).toBeNull();
	});

	test("a request made before a host exists waits for it", async () => {
		store().openDiagnose(warehouse);
		const mounted = await mountDevices(<div />, { overlays: true });
		await mounted.settle();
		expect(byRole("dialog").textContent).toContain("Diagnose warehouse-pi");
	});

	test("outside the area the links are built for the scope the page names", async () => {
		const hrefs: string[] = [];
		const mounted = await mountDevices(
			<AreaOverlays
				scope={{ kind: "app", appId: "app_support_portal" }}
				onNavigate={(href) => hrefs.push(href)}
			/>,
			{ passive: true },
		);
		await request(() => store().openPlane("hub"));
		await mounted.settle();
		const link = byRole("link", "warehouse-pi", byRole("dialog"));
		expect(link.getAttribute("href")).toBe(
			`/library/config/devices?id=app_support_portal&device=${warehouse}&tab=overview`,
		);
		expect(hrefs).toEqual([]);
		expect(mounted.navigations).toEqual([]);
	});

	test("signed out there is no workspace: nothing renders and nothing throws", async () => {
		store().openUnlock(lab);
		await mountDevices(<AreaOverlays />, { passive: true, signedIn: false });
		expect(queryByRole("dialog")).toBeNull();
	});
});
