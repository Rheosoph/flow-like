import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	byRole,
	click,
	clickByText,
	inPortal,
	installDom,
	typeInto,
} from "../../testing/dom-harness";

const dom = installDom();
const { mountDevices, cleanupDevices, preloadDevices } = await import(
	"../../testing/mount-devices"
);
const { createFakeWorkspace } = await import("../../testing/fake-workspace");
const { sampleFleet } = await import(
	"../../../../../lib/device-management/model/__fixtures__/sample-fleet"
);
const kit = await import("../deploy-test-kit");
const { EDGE, STUDIO, LAB, VISITOR, CRM, text } = kit;
await preloadDevices();

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
	globalThis.sessionStorage.clear();
});
afterAll(dom.restore);

const summary = () => text(byRole("region", "Your choices"));
const endpoint = (appId: string, params = {}, options = {}) =>
	kit.mountApp(mountDevices, appId, { step: "endpoint", ...params }, options);
const exceptions = (
	root: ParentNode,
	table = "Endpoint differences by device",
) =>
	Array.from(
		root.querySelectorAll(
			`[data-exceptions] table[aria-label="${table}"] tbody tr`,
		),
	).map((row) => text(row as HTMLElement));
const APP_SCOPE = { kind: "app", appId: VISITOR } as const;
/** studio-mac-mini can't sandbox: its acknowledgement, so a test about something else isn't blocked by it. */
const trustStudio = (root: ParentNode) =>
	click(root.querySelector(`#deploy-trust-${STUDIO}`) as Element);

/** Visitor Check-in with only its page: one hosted event, so instances aren't limited. */
async function pageOnly(devices: string[]) {
	const fake = await createFakeWorkspace();
	kit.seedDraft(fake, {
		appId: VISITOR,
		scope: APP_SCOPE,
		route: { deviceIds: [], mode: "new" },
		reached: 4,
		change: (draft) => ({
			...draft,
			scope: "events",
			events: ["evt_visitor_page"],
			targets: devices.map((deviceId) => ({
				deviceId,
				choices: {},
				serveBoth: [],
				over: {},
			})),
		}),
	});
	return fake;
}

describe("Endpoint (APP §3.9)", () => {
	test("app-endpoint: exposing it lists each device without a certificate, with its acknowledgement", async () => {
		const view = await endpoint(VISITOR, { device: [EDGE, STUDIO] });
		expect(text(view.container)).toContain("Web endpoint for Check-in page");
		expect(summary()).toContain(
			"127.0.0.1:8080 · 1 instance · sandboxed on 1 of 2",
		);
		await click(byRole("radio", /All networks/, view.container));
		expect(text(view.container)).toContain(
			"Exposed on every network the device is on.",
		);
		expect(exceptions(view.container)).toEqual([
			expect.stringContaining(
				"edge-berlin-01No certificate · unencryptedNo certificate is picked for edge-berlin-01. Anyone on its network can read the traffic.Serve unencrypted on edge-berlin-01",
			),
			expect.stringContaining("Serve unencrypted on studio-mac-mini"),
		]);
		expect(kit.footBlocking(view.container)).toBe(
			"Confirm serving unencrypted on edge-berlin-01, or pick a certificate there.",
		);
		const before = view.navigations.length;
		await clickByText("Continue", view.container);
		expect(view.navigations.length).toBe(before);

		await click(
			view.container.querySelector(`#deploy-unencrypted-${EDGE}`) as Element,
		);
		await click(
			view.container.querySelector(`#deploy-unencrypted-${STUDIO}`) as Element,
		);
		// Next in line: the device that runs it as the agent.
		expect(kit.footBlocking(view.container)).toBe(
			"Confirm that studio-mac-mini runs Visitor Check-in with the agent's full access.",
		);
		await click(
			view.container.querySelector(`#deploy-trust-${STUDIO}`) as Element,
		);
		expect(kit.footBlocking(view.container)).toBeNull();
		expect(summary()).toContain("0.0.0.0:8080");
		expect(kit.primaries(view.container)).toBe(1);
		expect(kit.copyOf(view.container)).not.toMatch(kit.MACHINE_WORDS);
	});

	test("certificates are per device, and only where the device lets this viewer choose", async () => {
		const view = await endpoint(VISITOR, { device: [EDGE, STUDIO, LAB] });
		const select = (deviceId: string) =>
			view.container.querySelector<HTMLButtonElement>(
				`#deploy-certificate-${deviceId}`,
			) as HTMLButtonElement;
		expect(select(EDGE).disabled).toBe(false);
		expect(select(EDGE).textContent).toContain("No certificate (unencrypted)");
		expect(text(view.container)).toContain("2 certificates on edge-berlin-01.");
		expect(text(view.container)).toContain(
			"studio-mac-mini has reported no certificates.",
		);
		// A locked device hasn't said anything yet: the choice waits, visible and disabled.
		expect(select(LAB).disabled).toBe(true);
		expect(text(view.container)).toContain(
			"Certificates on lab-gpu-02 load once it's connected live.",
		);
	});

	test("a device without certificate rights keeps the choice disabled with the reason", async () => {
		const seed = sampleFleet();
		const inspection = seed.live[STUDIO].inspection;
		if (!inspection) throw new Error("studio-mac-mini has no live inspection");
		inspection.value.can_manage_certificates = false;
		const view = await endpoint(VISITOR, { device: STUDIO }, { seed });
		const select = view.container.querySelector<HTMLButtonElement>(
			`#deploy-certificate-${STUDIO}`,
		);
		expect(select?.disabled).toBe(true);
		expect(text(view.container)).toContain(
			"Needs Manage certificates on the whole device.",
		);
	});

	test("a port in use on one device moves there, or gets its own value", async () => {
		const view = await endpoint(VISITOR, { device: [EDGE, STUDIO] });
		await trustStudio(view.container);
		await typeInto(
			view.container.querySelector("#deploy-port") as Element,
			"8081",
		);
		await view.settle();
		expect(exceptions(view.container)).toEqual([
			expect.stringContaining(
				"edge-berlin-01Port 8082Port 8081 is used by invoice-extractor there, so it uses 8082 instead.Change for this device",
			),
		]);
		expect(kit.footBlocking(view.container)).toBeNull();

		await clickByText("Change for this device", view.container);
		const sheet = inPortal("dialog");
		expect(text(sheet)).toContain("Endpoint on edge-berlin-01");
		expect(text(sheet)).toContain("8081 (invoice-extractor)");
		await typeInto(sheet.querySelector("#deploy-own-port") as Element, "8443");
		await clickByText("Done", sheet);
		// A port typed for one device that is taken there is an error, not a silent move.
		expect(exceptions(view.container)[0]).toContain(
			"Port 8443 is used by support-bot on edge-berlin-01.",
		);
		expect(kit.footBlocking(view.container)).toBe(
			"Port 8443 is used by support-bot on edge-berlin-01.",
		);

		await clickByText("Change for this device", view.container);
		await typeInto(
			inPortal("dialog").querySelector("#deploy-own-port") as Element,
			"9000",
		);
		await clickByText("Done", inPortal("dialog"));
		expect(exceptions(view.container)).toEqual([
			expect.stringContaining(
				"edge-berlin-01Port 9000You set this port for this device.",
			),
		]);
		expect(kit.footBlocking(view.container)).toBeNull();
	});

	test("the access token: generated per device, shared, or typed; never kept", async () => {
		const view = await endpoint(VISITOR, { device: [EDGE, STUDIO] });
		await trustStudio(view.container);
		expect(
			byRole("button", "One token per device", view.container).getAttribute(
				"aria-pressed",
			),
		).toBe("true");
		await clickByText("Same token on every device", view.container);
		expect(kit.footBlocking(view.container)).toBeNull();
		expect(kit.savedText()).toContain('"token":"same","tokenValue":""');

		await clickByText("Set my own", view.container);
		expect(kit.footBlocking(view.container)).toBe(
			"The access token needs at least 32 printable characters without spaces.",
		);
		const token = "0123456789abcdef0123456789abcdef-own";
		await typeInto(
			view.container.querySelector("#deploy-token") as Element,
			token,
		);
		expect(kit.footBlocking(view.container)).toBeNull();
		expect(kit.savedText()).not.toContain(token);
	});

	test("hosting values the plan doesn't carry are shown, not edited", async () => {
		const view = await endpoint(VISITOR, { device: EDGE });
		expect(text(view.container)).toContain(
			"Advanced64 parallel requests · 300 s timeout",
		);
		await clickByText("Advanced", view.container);
		expect(text(view.container)).toContain(
			"Change these after deploy in Configuration › Edit as JSON…",
		);
	});

	test("without a served event step 5 is only limits", async () => {
		const fake = await createFakeWorkspace();
		kit.seedDraft(fake, {
			appId: VISITOR,
			scope: APP_SCOPE,
			route: { deviceIds: [], mode: "new" },
			reached: 4,
			change: (draft) => ({
				...draft,
				scope: "events",
				events: ["evt_badge_printer"],
			}),
		});
		const view = await endpoint(VISITOR, { device: EDGE }, { fake });
		expect(byRole("heading", /Limits/, view.container).textContent).toBe(
			"Step 5 of 8: Limits",
		);
		expect(text(view.container)).toContain("No web endpoint needed");
		expect(text(view.container)).toContain(
			"Badge printer run on their own, so the service doesn't listen on a port.",
		);
		expect(summary()).toContain("LimitsNo endpoint · 1 instance");
	});
});

describe("Instances and isolation", () => {
	test("a background event limits the service to one instance, with the reason", async () => {
		const view = await endpoint(VISITOR, { device: EDGE });
		expect(
			byRole("button", "More", view.container).hasAttribute("disabled"),
		).toBe(true);
		expect(text(view.container)).toContain(
			"Badge printer runs on its own, so this service runs 1 instance.",
		);
	});

	test("served events alone can run several instances", async () => {
		const fake = await pageOnly([EDGE]);
		const view = await endpoint(VISITOR, { device: EDGE }, { fake });
		await click(byRole("button", "More", view.container));
		await click(byRole("button", "More", view.container));
		expect(summary()).toContain("3 instances");
		await click(byRole("button", "Fewer", view.container));
		expect(summary()).toContain("2 instances");
	});

	test("a device that requires sandboxed services leaves one choice", async () => {
		const view = await endpoint(VISITOR, { device: EDGE });
		expect(text(view.container)).toContain(
			"edge-berlin-01 requires sandboxed services.",
		);
		expect(
			byRole("radio", /Runs as the agent/, view.container).hasAttribute(
				"disabled",
			),
		).toBe(true);
		expect(text(view.container)).toContain(
			"edge-berlin-01 requires sandboxed services, so Sandboxed is the only choice.",
		);
		expect(view.container.querySelector("[data-trust]")).toBeNull();
		expect(kit.footBlocking(view.container)).toBeNull();

		// Limits are the service's own.
		await typeInto(
			view.container.querySelector("#deploy-memory") as Element,
			"2048",
		);
		expect(kit.savedText()).toContain(`"memoryBytes":${2048 * 1024 ** 2}`);
	});

	test("a device that can't sandbox runs it as the agent, after one acknowledgement", async () => {
		const view = await endpoint(VISITOR, { device: STUDIO });
		expect(text(view.container)).toContain(
			"studio-mac-mini can't sandbox services.",
		);
		expect(
			byRole("radio", /Sandboxed with limits/, view.container).hasAttribute(
				"disabled",
			),
		).toBe(true);
		const trust = view.container.querySelector("[data-trust]") as HTMLElement;
		expect(text(trust)).toContain(
			"It can read and change everything the agent can",
		);
		expect(kit.footBlocking(view.container)).toBe(
			"Confirm that studio-mac-mini runs Visitor Check-in with the agent's full access.",
		);
		await click(trust.querySelector(`#deploy-trust-${STUDIO}`) as Element);
		expect(kit.footBlocking(view.container)).toBeNull();
		expect(summary()).toContain("sandboxed on 0 of 1");
	});

	test("several devices: the shared way to run first, then the device that runs it as the agent", async () => {
		const view = await endpoint(VISITOR, { device: [EDGE, STUDIO] });
		expect(text(view.container)).toContain(
			"Same on every device: sandboxed · 1 core · 1.0 GiB · 256 processes · 4.0 GiB disk",
		);
		expect(
			exceptions(view.container, "Isolation differences by device"),
		).toEqual([
			"studio-mac-miniRuns as the agentstudio-mac-mini can't sandbox services, so it has the agent's full access there.",
		]);
		expect(text(view.container)).toContain(
			"Network boundary. Sandboxed services still share the device's network and can reach local and cloud metadata addresses.",
		);

		await click(byRole("radio", /Runs as the agent/, view.container));
		expect(text(view.container)).toContain(
			"Same on every device: runs as the agent",
		);
		expect(
			exceptions(view.container, "Isolation differences by device"),
		).toEqual([]);
		expect(text(view.container)).not.toContain("Network boundary.");
	});

	test("limits a device would refuse keep Continue off, with the range", async () => {
		const view = await endpoint(VISITOR, { device: EDGE });
		expect(kit.footBlocking(view.container)).toBeNull();
		await typeInto(
			view.container.querySelector("#deploy-memory") as Element,
			"8",
		);
		expect(kit.footBlocking(view.container)).toBe(
			"Set limits the device accepts: CPU above 0, memory of at least 64 MiB, 16 to 65,536 processes and at least 16 MiB of disk.",
		);
		const before = view.navigations.length;
		await clickByText("Continue", view.container);
		expect(view.navigations.length).toBe(before);
		await typeInto(
			view.container.querySelector("#deploy-memory") as Element,
			"64",
		);
		expect(kit.footBlocking(view.container)).toBeNull();
	});

	test("an update keeps each service's address and limits until asked", async () => {
		const view = await endpoint(
			CRM,
			{ mode: undefined, device: EDGE, service: "nightly-sync" },
			{ platform: "desktop" },
		);
		expect(text(view.container)).toContain(
			"Each service keeps how it runs and its limits.",
		);
		expect(summary()).toContain("keeps each service's limits");
		await clickByText("Change how it runs…", view.container);
		expect(
			byRole("radio", /Sandboxed with limits/, view.container),
		).toBeTruthy();
		expect(text(view.container)).toContain(
			"A service that is updated keeps the number it runs now.",
		);
	});
});
