import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	byRole,
	byText,
	click,
	clickByText,
	inPortal,
	installDom,
	queryByRole,
} from "../testing/dom-harness";

const dom = installDom();
const { mountDevices, cleanupDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { sampleFleet } = await import(
	"../../../../lib/device-management/model/__fixtures__/sample-fleet"
);
const { deployExitHref } = await import("../routing/devices-href");
const kit = await import("./deploy-test-kit");
const { EDGE, STUDIO, LAB, VISITOR, CRM, INVOICE, text } = kit;
await preloadDevices();

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
	globalThis.sessionStorage.clear();
});
afterAll(dom.restore);

const lastHref = (view: { navigations: { href: string }[] }) =>
	view.navigations.at(-1)?.href ?? "";

describe("entries (APP §3.1)", () => {
	test("app-first: the app is fixed, the whole app is ticked and nothing is sent", async () => {
		const view = await kit.mountApp(mountDevices, VISITOR);
		const page = text(view.container);
		expect(byRole("heading", "Deploy Visitor Check-in").tagName).toBe("H1");
		expect(page).toContain(
			"Visitor Check-in · runs online · v0.4.0 · no devices yet · new service",
		);
		expect(page).toContain(
			"Deploy Visitor Check-in: pick what runs, then where.",
		);
		// The schedule can run too; the summary below still counts the two that Whole app ticks.
		expect(page).toContain("3 of its 4 events can run on a device.");
		expect(page).toContain(
			"Nothing changes on any device until you deploy on step 7.",
		);
		// Step 6 follows the app's mode (A1).
		expect(page).toContain("Access & cost");
		expect(page).not.toContain("Copy & upload");
		expect(page).toContain("Can't run on devices · 1");
		expect(page).toContain("Visitor Check-in · v0.4.0 · 2 events · 1 service");
		// No app picker in an app's own page.
		expect(view.container.querySelector("#deploy-app")).toBeNull();
		expect(kit.primaries(view.container)).toBe(1);
		expect(kit.copyOf(view.container)).not.toMatch(kit.MACHINE_WORDS);
		// Planning reads; it never changes a device.
		expect(
			view.fake.api.commands.filter(([, type]) => type === "apply"),
		).toEqual([]);
	});

	test("the URL gets the step, and Continue moves on with a replace", async () => {
		const view = await kit.mountApp(mountDevices, VISITOR);
		expect(view.navigations.at(-1)).toEqual({
			mode: "replace",
			href: `/library/config/devices?id=${VISITOR}&flow=deploy&mode=new&step=what`,
		});
		await clickByText("Continue", view.container);
		await view.settle();
		expect(view.navigations.at(-1)?.mode).toBe("replace");
		expect(lastHref(view)).toContain("step=how");
		expect(byRole("heading", /How it runs/).tagName).toBe("H2");
	});

	test("the stepper leads back to a visited step, never ahead to one that wasn't reached", async () => {
		const view = await kit.mountApp(mountDevices, VISITOR);
		const stepOf = (name: string) =>
			view.container.querySelector(`ol li[data-s]:nth-child(${name})`);
		// In an app's page the app is given: step 1 is the current one, nothing ahead is a button.
		expect(stepOf("1")?.getAttribute("aria-current")).toBe("step");
		expect(queryByRole("button", /^Go to step/, view.container)).toBeNull();
		await clickByText("Continue", view.container);
		await view.settle();
		expect(stepOf("1")?.getAttribute("data-s")).toBe("done");
		expect(queryByRole("button", /^Go to step 3/, view.container)).toBeNull();
		await click(byRole("button", "Go to step 1: What to run", view.container));
		await view.settle();
		expect(lastHref(view)).toContain("step=what");
		// The step that was reached stays reachable from the stepper.
		expect(
			byRole("button", "Go to step 2: How it runs", view.container),
		).toBeTruthy();
	});

	test("one event from Events: scope One event, the others collapsed, Exit returns to Events", async () => {
		const view = await kit.mountApp(mountDevices, CRM, {
			event: "evt_crm_webhook",
			from: "events",
		});
		const page = text(view.container);
		expect(byRole("heading", "Deploy CRM webhook").tagName).toBe("H1");
		expect(page).toContain("CRM Sync · offline copy · v3.1.0 · no devices yet");
		expect(
			byRole("button", "One event", view.container).getAttribute(
				"aria-pressed",
			),
		).toBe("true");
		expect(page).toContain("Also run other events of CRM Sync · 3");
		expect(page).not.toContain("Nightly CRM sync");
		// The service takes the event's name.
		expect(
			view.container.querySelector<HTMLInputElement>("#deploy-service-id-main")
				?.value,
		).toBe("crm-webhook");
		// A local-only app's step 6 is the copy.
		expect(page).toContain("Copy & upload");
		const exit = byText("Exit deploy", view.container).closest("a");
		const events = `/library/config/events?id=${CRM}&event=evt_crm_webhook`;
		expect(exit?.getAttribute("href")).toBe(events);
		// The top bar says "Events › Deploy" in an app; the frame adds no crumb row of its own.
		expect(queryByRole("link", "Events", view.container)).toBeNull();
		expect(
			view.container.querySelector('nav[aria-label="Breadcrumb"]'),
		).toBeNull();

		await clickByText(/Also run other events/, view.container);
		expect(text(view.container)).toContain("Nightly CRM sync");
	});

	test("device-first without an app: What shows the app picker and Continue waits for a choice", async () => {
		const view = await kit.mountAccount(mountDevices, { device: EDGE });
		const page = text(view.container);
		expect(page).toContain("Deploy to edge-berlin-01");
		expect(page).toContain("Choose an app for edge-berlin-01.");
		expect(
			byRole("link", "edge-berlin-01", view.container).getAttribute("href"),
		).toContain(`device=${EDGE}`);
		expect(byRole("radio", /Visitor Check-in/, view.container)).toBeTruthy();
		expect(kit.footBlocking(view.container)).toBe("Choose an app first.");

		// R7: the gated Continue stays in place and does nothing.
		const before = view.navigations.length;
		const writes = view.fake.api.writes().length;
		await clickByText("Continue", view.container);
		expect(view.navigations.length).toBe(before);
		expect(view.fake.api.writes().length).toBe(writes);

		await click(byRole("radio", /Visitor Check-in/, view.container));
		await view.settle();
		expect(lastHref(view)).toContain(`app=${VISITOR}`);
		expect(text(view.container)).toContain(
			"Deploy Visitor Check-in to edge-berlin-01 as visitor-check-in.",
		);
	});

	test("new-service: a device and an app from the device page", async () => {
		const view = await kit.mountAccount(mountDevices, {
			device: EDGE,
			app: CRM,
		});
		const page = text(view.container);
		expect(page).toContain(
			"CRM Sync · offline copy · v3.1.0 · to edge-berlin-01 · new service",
		);
		expect(page).toContain("Deploy CRM Sync to edge-berlin-01 as crm-sync.");
		expect(page).toContain("4 of its 5 events can run on a device.");
		expect(page).toContain("Runs on 1 device now");
		expect(kit.primaries(view.container)).toBe(1);
	});

	test("update of one service: the title names it, the version is a choice", async () => {
		const view = await kit.mountApp(mountDevices, CRM, {
			mode: undefined,
			device: EDGE,
			service: "nightly-sync",
		});
		const page = text(view.container);
		expect(page).toContain("Update nightly-sync");
		expect(page).toContain(
			"CRM Sync · offline copy · v3.1.0 · to edge-berlin-01 · update",
		);
		expect(page).toContain(
			"Update nightly-sync on edge-berlin-01 to the newest version.",
		);
		expect(byRole("radio", /Keep each service's version/)).toBeTruthy();
		// Its own events are ticked; the app's other ones are offered.
		expect(page).toContain("Served now");
		expect(page).toContain("Not served yet");
		const served = view.container.querySelector(
			"#deploy-event-evt_crm_nightly",
		);
		expect(served?.getAttribute("data-state")).toBe("checked");
		expect(
			view.container
				.querySelector("#deploy-event-evt_crm_webhook")
				?.getAttribute("data-state"),
		).toBe("unchecked");
	});

	test("an update of a service that already runs the newest version starts with Keep", async () => {
		const view = await kit.mountApp(mountDevices, "app_field_notes", {
			mode: undefined,
			device: STUDIO,
			service: "field-notes",
		});
		expect(
			byRole("radio", /Keep each service's version/).getAttribute(
				"aria-checked",
			),
		).toBe("true");
		expect(text(byRole("region", "Your choices"))).toContain(
			"Field Notes · keeps each version · 1 service",
		);
		// The same entry on a service that is behind offers the newest version first.
		await cleanupDevices();
		globalThis.sessionStorage.clear();
		const behind = await kit.mountApp(mountDevices, INVOICE, {
			mode: undefined,
			device: EDGE,
			service: "invoice-extractor",
		});
		expect(
			byRole("radio", /^Newest/, behind.container).getAttribute("aria-checked"),
		).toBe("true");
	});

	test("a Change settings… entry keeps each service's version", async () => {
		const view = await kit.mountApp(mountDevices, CRM, {
			mode: undefined,
			device: EDGE,
			service: "nightly-sync",
			step: "settings",
		});
		expect(text(view.container)).toContain("keeps each service's version");
		expect(text(view.container)).toContain(
			"Change nightly-sync's settings or events on edge-berlin-01.",
		);
		expect(text(byRole("region", "Your choices"))).toContain(
			"CRM Sync · keeps each version · 1 service",
		);
	});

	test("update of every listed service: each device updates its own service", async () => {
		const view = await kit.mountApp(mountDevices, INVOICE, {
			mode: "update",
			device: [EDGE, LAB],
			step: "where",
		});
		const page = text(view.container);
		expect(page).toContain("Update Invoice AI");
		expect(page).toContain("Update invoice-extractor");
		// The locked device can't say which service it runs yet.
		expect(page).toContain("Update: its services load when you unlock.");
		expect(page).toContain("Only devices that run Invoice AI are listed.");
	});
});

describe("frame", () => {
	test("Exit goes where deployExitHref says", async () => {
		const view = await kit.mountApp(mountDevices, VISITOR);
		await clickByText("Exit deploy", view.container);
		expect(view.navigations.at(-1)).toEqual({
			mode: "push",
			href: deployExitHref(
				{ screen: "deploy", deviceIds: [], mode: "new" },
				{ kind: "app", appId: VISITOR },
			),
		});
	});

	test("Exit back to Events goes through the host router, so the page isn't loaded anew", async () => {
		const view = await kit.mountApp(mountDevices, CRM, {
			event: "evt_crm_webhook",
			from: "events",
		});
		await clickByText("Exit deploy", view.container);
		expect(view.navigations.at(-1)).toEqual({
			mode: "push",
			href: `/library/config/events?id=${CRM}&event=evt_crm_webhook`,
		});
	});

	test("Exit of a device-first update returns to the service", async () => {
		const view = await kit.mountAccount(mountDevices, {
			device: EDGE,
			app: CRM,
			service: "nightly-sync",
		});
		await clickByText("Exit deploy", view.container);
		expect(lastHref(view)).toBe(
			deployExitHref(
				{
					screen: "deploy",
					deviceIds: [EDGE],
					appId: CRM,
					serviceId: "nightly-sync",
				},
				{ kind: "account" },
			),
		);
	});

	test("resume: saved choices come back with a banner, on the furthest step", async () => {
		const fake = await createFakeWorkspace();
		kit.seedDraft(fake, {
			appId: VISITOR,
			scope: { kind: "app", appId: VISITOR },
			route: { deviceIds: [], mode: "new" },
			reached: 4,
			change: (draft) => ({
				...draft,
				scope: "events",
				events: ["evt_visitor_page"],
				targets: [{ deviceId: EDGE, choices: {}, serveBoth: [], over: {} }],
			}),
		});
		const view = await kit.mountApp(mountDevices, VISITOR, {}, { fake });
		const page = text(view.container);
		expect(page).toContain(
			"Picked up where you left off: step 5 of 8 · Endpoint & limits.",
		);
		expect(page).toContain("Secrets and access tokens are never kept");
		expect(lastHref(view)).toContain("step=endpoint");
		// The saved choice, not the entry's default.
		expect(page).toContain("Visitor Check-in · v0.4.0 · 1 event");
		await clickByText("Dismiss", view.container);
		expect(text(view.container)).not.toContain("Picked up where you left off");
	});

	test("a visit without a change isn't a resume", async () => {
		const fake = await createFakeWorkspace();
		const first = await kit.mountApp(mountDevices, VISITOR, {}, { fake });
		await first.unmount();
		const again = await kit.mountApp(mountDevices, VISITOR);
		expect(text(again.container)).not.toContain("Picked up where you left off");
	});

	test("Discard… asks, then starts over from the entry", async () => {
		const fake = await createFakeWorkspace();
		const key = kit.seedDraft(fake, {
			appId: VISITOR,
			scope: { kind: "app", appId: VISITOR },
			route: { deviceIds: [], mode: "new" },
			reached: 2,
			change: (draft) => ({
				...draft,
				targets: [{ deviceId: EDGE, choices: {}, serveBoth: [], over: {} }],
			}),
		});
		const view = await kit.mountApp(mountDevices, VISITOR, {}, { fake });
		expect(text(view.container)).toContain("to edge-berlin-01");
		const writes = view.fake.api.writes().length;
		await clickByText("Discard…", view.container);
		const sheet = inPortal("alertdialog");
		expect(text(sheet)).toContain("Discard this deploy?");
		expect(text(sheet)).toContain("Your choices in this window are cleared.");
		await clickByText("Discard deploy", sheet);
		await view.settle();
		expect(text(view.container)).toContain("no devices yet");
		expect(lastHref(view)).toContain("step=what");
		const saved = JSON.parse(globalThis.sessionStorage.getItem(key) ?? "{}");
		expect(saved.draft.targets).toEqual([]);
		expect(view.fake.api.writes().length).toBe(writes);
	});

	test("the summary lines are the way back to a visited step", async () => {
		const view = await kit.mountApp(mountDevices, VISITOR, {
			device: [EDGE, STUDIO],
			step: "settings",
		});
		const summary = byRole("region", "Your choices", view.container);
		expect(text(summary)).toContain("edge-berlin-01, studio-mac-mini");
		// Visited and prefilled steps are buttons; later ones and Rollout are not.
		await click(byRole("button", /Where it runs/, summary));
		await view.settle();
		expect(lastHref(view)).toContain("step=where");
		expect(
			Array.from(summary.querySelectorAll("button")).some((button) =>
				/Rollout|Review/.test(button.textContent ?? ""),
			),
		).toBe(false);
		// Below 960 px the same summary opens from one sticky line.
		expect(
			view.container.querySelector("[data-deploy-summary-bar]")?.textContent,
		).toContain("This deploy · Step 3 of 8 · 2 devices");
	});

	test("on Rollout the frame shows no planning headline and no foot, and offers the run its headline place", async () => {
		const view = await kit.mountApp(mountDevices, VISITOR, {
			device: EDGE,
			step: "rollout",
		});
		// The run's own sentence goes where the planning headline sits on steps 1–7.
		const slot = view.container.querySelector("[data-deploy-headline-slot]");
		expect(slot?.nextElementSibling?.querySelector("ol")).toBeTruthy();
		expect(text(view.container)).not.toContain("until you deploy on step 7");
		expect(view.container.querySelector("[data-wizard-foot]")).toBeNull();
		expect(text(view.container)).not.toContain("Discard…");
	});

	test("the headline place exists on Rollout only", async () => {
		const view = await kit.mountApp(mountDevices, VISITOR, {
			device: EDGE,
			step: "review",
		});
		expect(
			view.container.querySelector("[data-deploy-headline-slot]"),
		).toBeNull();
		expect(view.container.querySelector("[data-headline]")).toBeTruthy();
	});

	test("Review has Back but no Continue: its primary is the step's own, placed after Back", async () => {
		const view = await kit.mountApp(mountDevices, VISITOR, {
			device: EDGE,
			step: "review",
		});
		const foot = view.container.querySelector("[data-wizard-foot]");
		expect(foot?.textContent).toContain("Back");
		expect(foot?.textContent).not.toContain("Continue");
		const slot = foot?.querySelector("[data-deploy-foot-slot]");
		expect(slot?.previousElementSibling?.textContent).toBe("Back");
		expect(kit.primaries(view.container)).toBeLessThanOrEqual(1);
	});

	test("Review waits for an earlier step the plan's own checks can't see", async () => {
		const fake = await createFakeWorkspace();
		kit.seedDraft(fake, {
			appId: VISITOR,
			scope: { kind: "app", appId: VISITOR },
			route: { deviceIds: [EDGE], mode: "new" },
			reached: 6,
			change: (draft) => ({
				...draft,
				isolation: draft.isolation
					? { ...draft.isolation, memoryBytes: 8 * 1024 ** 2 }
					: draft.isolation,
			}),
		});
		const view = await kit.mountApp(
			mountDevices,
			VISITOR,
			{ device: EDGE, step: "review" },
			{ fake },
		);
		const page = text(view.container);
		expect(page).toContain("Endpoint & limits needs your attention first.");
		expect(page).toContain("Set limits the device accepts");
		// The step's own primary isn't offered while the plan can't be sent.
		expect(kit.primaries(view.container)).toBe(0);
		await clickByText("Go to Endpoint & limits", view.container);
		expect(lastHref(view)).toContain("step=endpoint");
	});

	test("the foot offers its primary slot on Review only", async () => {
		const view = await kit.mountApp(mountDevices, VISITOR, {
			device: EDGE,
			step: "endpoint",
		});
		expect(view.container.querySelector("[data-deploy-foot-slot]")).toBeNull();
		expect(kit.primaries(view.container)).toBe(1);
	});
});

describe("gates (APP §3.7, §3.16)", () => {
	test("no-deploy: a device that lacks Deploy for this app is said on every step", async () => {
		const view = await kit.mountAccount(mountDevices, {
			device: LAB,
			app: CRM,
		});
		const page = text(view.container);
		expect(page).toContain("lab-gpu-02 can't take this deploy right now.");
		expect(page).toContain("You can't deploy to lab-gpu-02 right now.");
		expect(page).toContain("Needs Deploy & configure on this app.");
		expect(page).toContain(
			"You can look through the steps, but Deploy stays unavailable until this is fixed.",
		);
		await clickByText("Ask the owner", view.container);
		expect(dom.clipboard.at(-1)).toContain(
			"Deploy & configure on lab-gpu-02 for CRM Sync",
		);
		await clickByText("Change devices", view.container);
		expect(lastHref(view)).toContain("step=where");
	});

	test("local-web: a local-only app can't be deployed from the browser", async () => {
		const view = await kit.mountApp(
			mountDevices,
			"app_warehouse_scan",
			{},
			{ platform: "web" },
		);
		const page = text(view.container);
		expect(page).toContain(
			"Warehouse Scanner can't be deployed from the browser.",
		);
		expect(page).toContain("Local-only apps deploy from the desktop app.");
		expect(text(byRole("region", "Your choices"))).toContain(
			"Needs the desktop app",
		);
		await clickByText("Continue", view.container);
		await view.settle();
		expect(text(view.container)).toContain(
			"Offline copies are prepared by the desktop app.",
		);
		expect(kit.footBlocking(view.container)).toBe(
			"Offline copies are prepared by the desktop app.",
		);
		const before = view.navigations.length;
		await clickByText("Continue", view.container);
		expect(view.navigations.length).toBe(before);
	});

	test("staged-elsewhere: an update that is already staged is said on What", async () => {
		const seed = sampleFleet();
		const live = seed.live[EDGE];
		live.rollouts = [
			...(live.rollouts ?? []),
			{
				rollout_id: "7d3f1e52-2b0a-4c7f-9d11-5f2f1c9a0b01",
				placement_id: "support-bot",
				project_id: "app_support_portal",
				state: "staged",
				created_at: seed.now - 480,
				updated_at: seed.now - 480,
			},
		];
		const view = await kit.mountApp(
			mountDevices,
			"app_support_portal",
			{ mode: undefined, device: EDGE, service: "support-bot" },
			{ seed },
		);
		const page = text(view.container);
		expect(page).toContain(
			"An update is already staged for support-bot on edge-berlin-01.",
		);
		expect(page).toContain("activate or discard it before you deploy");
		await clickByText("Open support-bot", view.container);
		expect(lastHref(view)).toContain("service=support-bot");
	});
});

describe("older hub", () => {
	test("renders its interims, shows no error and asks each new route once", async () => {
		const view = await kit.mountApp(
			mountDevices,
			VISITOR,
			{ device: [EDGE, LAB], step: "where" },
			{ hubVersion: "old" },
		);
		const page = text(view.container);
		expect(page).toContain("Where it runs");
		expect(view.container.querySelector('[role="alert"]')).toBeNull();
		expect(view.container.querySelector('[data-kind="error"]')).toBeNull();
		// Without the hub's answer the shared device stays pickable; it says more once unlocked.
		const lab = view.container.querySelector(`[data-device="${LAB}"]`);
		expect(lab?.getAttribute("data-ok")).toBe("true");
		const routes = view.fake.api.calls
			.map(([method, path]) => `${method} ${path}`)
			.filter((route) => /my-access|device-placements/.test(route));
		expect(new Set(routes).size).toBe(routes.length);
	});
});
