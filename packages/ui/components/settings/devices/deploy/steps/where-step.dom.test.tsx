import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	byRole,
	click,
	clickByText,
	installDom,
} from "../../testing/dom-harness";

const dom = installDom();
const { mountDevices, cleanupDevices, preloadDevices } = await import(
	"../../testing/mount-devices"
);
const { createFakeWorkspace } = await import("../../testing/fake-workspace");
const { useOverlayStore } = await import("../../workspace/overlay-store");
const kit = await import("../deploy-test-kit");
const { EDGE, STUDIO, LAB, WAREHOUSE, COLD, VISITOR, CRM, INVOICE, text } = kit;
await preloadDevices();

afterEach(async () => {
	useOverlayStore.getState().close();
	await cleanupDevices();
	await dom.cleanup();
	globalThis.sessionStorage.clear();
});
afterAll(dom.restore);

const card = (root: ParentNode, deviceId: string) =>
	root.querySelector<HTMLElement>(`[data-device="${deviceId}"]`) as HTMLElement;
const box = (root: ParentNode, deviceId: string) =>
	card(root, deviceId).querySelector<HTMLButtonElement>(
		"button[role=checkbox]",
	) as HTMLButtonElement;
const where = (appId: string, params = {}, options = {}) =>
	kit.mountApp(mountDevices, appId, { step: "where", ...params }, options);

describe("Where · devices (APP §3.7)", () => {
	test("app-where: picked devices show their facts and plan line, gated ones say why", async () => {
		const view = await where(VISITOR, { device: [EDGE, STUDIO] });
		const edge = text(card(view.container, EDGE));
		expect(box(view.container, EDGE).getAttribute("data-state")).toBe(
			"checked",
		);
		expect(edge).toContain("Linux · agent 0.9.4 · sandbox required");
		expect(edge).toContain("Live · relayed");
		expect(edge).toContain("New service visitor-check-in");
		expect(text(card(view.container, STUDIO))).toContain(
			"Mac · agent 0.9.4 · can't sandbox",
		);

		// First gate that fails, with its fix (R7: visible, disabled, reason underneath).
		const cold = card(view.container, COLD);
		expect(cold.getAttribute("data-ok")).toBe("false");
		expect(box(view.container, COLD).disabled).toBe(true);
		expect(text(cold)).toContain(
			"cold-storage-nas hasn't checked in yet. Deploying needs a live connection.",
		);
		expect(text(cold)).toContain("Start instructions");
		expect(text(card(view.container, WAREHOUSE))).toMatch(
			/warehouse-pi is offline since .+\. Deploying needs a live connection\.Diagnose/,
		);
		expect(text(card(view.container, LAB))).toContain(
			"Needs Deploy & configure on this app.",
		);
		expect(text(view.container)).toContain(
			"2 of 5 devices selected · 3 can't take a deploy right now",
		);
		expect(text(view.container)).toContain("2 revoked devices aren't listed.");
		expect(text(view.container)).toContain(
			"Same on every device: all 2 events can run.",
		);
		expect(kit.footBlocking(view.container)).toBeNull();
		expect(kit.primaries(view.container)).toBe(1);
		expect(kit.copyOf(view.container)).not.toMatch(kit.MACHINE_WORDS);
	});

	test("a gated card can't be ticked, and its fix opens where it helps", async () => {
		const view = await where(VISITOR, { device: EDGE });
		const commands = view.fake.api.commands.length;
		// It is unlocked but can't be picked, so its card promises no connection.
		expect(text(card(view.container, WAREHOUSE))).toContain("Unlocked");
		expect(text(card(view.container, WAREHOUSE))).not.toContain(
			"connects live when you pick it",
		);
		await click(box(view.container, WAREHOUSE));
		expect(box(view.container, WAREHOUSE).getAttribute("data-state")).toBe(
			"unchecked",
		);
		expect(text(view.container)).toContain("1 of 5 devices selected");
		await clickByText("Diagnose", card(view.container, WAREHOUSE));
		expect(useOverlayStore.getState().overlay).toEqual({
			kind: "diagnose",
			deviceId: WAREHOUSE,
		});
		expect(view.fake.api.commands.length).toBe(commands);
	});

	test("ticking and unticking a device changes the plan", async () => {
		const view = await where(VISITOR, { device: EDGE });
		await click(box(view.container, STUDIO));
		await view.settle();
		expect(text(view.container)).toContain("2 of 5 devices selected");
		expect(text(view.container)).toContain(
			"Deploy Visitor Check-in to 2 devices.",
		);
		await click(box(view.container, EDGE));
		expect(text(view.container)).toContain("1 of 5 devices selected");
		await click(box(view.container, STUDIO));
		expect(kit.footBlocking(view.container)).toBe("Pick at least one device.");
	});

	test("storage room shows when the agent reports it (BG17)", async () => {
		const view = await where(VISITOR, { device: EDGE });
		expect(text(card(view.container, EDGE))).toContain("used by app versions");
		expect(
			view.fake.api.commands.filter(
				([deviceId, type, command]) =>
					deviceId === EDGE &&
					type === "artifact" &&
					(command.request as { kind?: string }).kind === "usage",
			),
		).toHaveLength(1);
	});
});

describe("Where · locked devices and updates", () => {
	test("app-locked: a locked device can be ticked; Continue waits for the unlock", async () => {
		const view = await where(INVOICE, {
			mode: "update",
			device: [EDGE, LAB],
		});
		const page = text(view.container);
		expect(box(view.container, LAB).getAttribute("data-state")).toBe("checked");
		expect(text(card(view.container, LAB))).toContain(
			"Platform, agent and isolation load when you unlock",
		);
		expect(page).toContain(
			"2 of 2 devices selected · 1 locked · 1 can't take a deploy right now",
		);
		// The busy update on edge-berlin-01 is a gate, with the way to follow it.
		expect(text(card(view.container, EDGE))).toMatch(
			/Wait for invoice-extractor's update to finish \(by .+ at the latest\)\.Follow/,
		);
		expect(text(card(view.container, EDGE))).toContain(
			"Remove from this deploy",
		);
		// The foot says the device's own reason, not only that it is gated.
		expect(kit.footBlocking(view.container)).toMatch(
			/^edge-berlin-01: Wait for invoice-extractor's update to finish \(by .+ at the latest\)\.$/,
		);
		expect(page).toContain(
			"Invoice AI · runs online · v1.5.0 · 2 devices · update of 2 services",
		);

		await clickByText("Unlock 1 selected…", view.container);
		expect(useOverlayStore.getState().overlay).toEqual({
			kind: "unlock",
			deviceId: LAB,
			connectLive: true,
		});

		await clickByText("Remove from this deploy", view.container);
		expect(kit.footBlocking(view.container)).toBe(
			"Unlock lab-gpu-02 to continue.",
		);
	});

	test("a one-service update keeps its device", async () => {
		const view = await where(CRM, {
			mode: undefined,
			device: EDGE,
			service: "nightly-sync",
		});
		expect(box(view.container, EDGE).disabled).toBe(true);
		const edge = text(card(view.container, EDGE));
		expect(edge).toContain("Update nightly-sync");
		expect(edge).toContain(
			"The device of a one-service update is fixed. Deploy from the app to pick others.",
		);
		// Only devices that run the app are listed.
		expect(
			view.container.querySelector(`[data-device="${STUDIO}"]`),
		).toBeNull();
	});
});

describe("Where · one event (APP §3.15)", () => {
	const event = (options = {}) =>
		where(
			CRM,
			{ event: "evt_crm_webhook", device: [EDGE, STUDIO] },
			{ platform: "desktop", ...options },
		);

	test("event-where: a device that runs the app offers Add to its service", async () => {
		const view = await event();
		const edge = card(view.container, EDGE);
		expect(
			byRole("combobox", "Plan on edge-berlin-01", edge).textContent,
		).toContain("New service crm-webhook");
		expect(text(edge)).toContain(
			"Or add CRM webhook to nightly-sync as an update of that service.",
		);
		// studio-mac-mini doesn't run CRM Sync: a new service, no choice.
		expect(text(card(view.container, STUDIO))).toContain(
			"New service crm-webhook",
		);
		expect(
			card(view.container, STUDIO).querySelector("[role=combobox]"),
		).toBeNull();
		expect(text(view.container)).toContain(
			"The device checks the events again after the copy arrives (step 6).",
		);
	});

	test("Add to a stopped service says it stays stopped", async () => {
		const fake = await createFakeWorkspace(undefined, { platform: "desktop" });
		kit.seedDraft(fake, {
			appId: CRM,
			scope: { kind: "app", appId: CRM },
			route: {
				deviceIds: [EDGE],
				mode: "new",
				eventId: "evt_crm_webhook",
			},
			reached: 2,
			change: (draft) => ({
				...draft,
				targets: [
					{
						deviceId: EDGE,
						choices: { main: { kind: "add", serviceId: "nightly-sync" } },
						serveBoth: [],
						over: {},
					},
				],
			}),
		});
		const view = await where(
			CRM,
			{ event: "evt_crm_webhook", device: EDGE },
			{ fake },
		);
		expect(text(card(view.container, EDGE))).toContain(
			"nightly-sync is stopped as you asked; it stays stopped, so CRM webhook won't answer until you start it.",
		);
	});

	test("an event another service already serves there is left out, or served in both", async () => {
		const view = await where(INVOICE, {
			event: "evt_extract_http",
			device: STUDIO,
		});
		// studio-mac-mini doesn't run Invoice AI; lab does but is locked. Nothing is left out here.
		expect(text(view.container)).toContain(
			"Same on every device: the event can run.",
		);
		expect(text(view.container)).not.toContain("Serve it in both");
	});
});

describe("Where · older agent and older hub", () => {
	test("an agent without the capacity fact shows no storage line and gets no new command", async () => {
		const view = await where(
			VISITOR,
			{ device: [EDGE, STUDIO] },
			{ agentFeatures: {} },
		);
		const edge = text(card(view.container, EDGE));
		// What an older agent doesn't report reads "unknown", never as an error.
		expect(edge).toContain("Linux · agent unknown · sandbox required");
		expect(edge).not.toContain("used by app versions");
		expect(view.container.querySelector('[role="alert"]')).toBeNull();
		expect(
			view.fake.api.commands.filter(
				([, type, command]) =>
					type === "artifact" &&
					(command.request as { kind?: string }).kind === "usage",
			),
		).toEqual([]);
	});

	test("a hub without my-access leaves a shared device pickable", async () => {
		const view = await where(VISITOR, { device: EDGE }, { hubVersion: "old" });
		expect(card(view.container, LAB).getAttribute("data-ok")).toBe("true");
		expect(box(view.container, LAB).disabled).toBe(false);
		expect(view.container.querySelector('[data-kind="error"]')).toBeNull();
		const asked = view.fake.api.calls.filter(([, path]) =>
			path.endsWith("/my-access"),
		);
		expect(asked.length).toBeLessThanOrEqual(1);
	});
});
