import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	DAY_S,
	type TestReleaseOptions,
	publishTestRelease,
} from "../hub/release-test-kit";
import {
	allByRole,
	byRole,
	click,
	inPortal,
	installDom,
	queryByRole,
	typeInto,
} from "../testing/dom-harness";
import type { DeviceView, OpenOptions } from "./device-test-kit";

const dom = installDom();
const kit = await import("./device-test-kit");
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { sampleFleet } = await import(
	"../../../../lib/device-management/model/__fixtures__/sample-fleet"
);

const { IDS, MACHINE, commandTypes, openDevice, primaries, text } = kit;

afterEach(async () => {
	await kit.resetDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

const open = (deviceId: string, options: OpenOptions = {}) =>
	openDevice(deviceId, { tab: "settings", ...options });

const layer = (view: DeviceView, id: string) => {
	const found = view.container.querySelector<HTMLElement>(
		`#device-layer-${id}`,
	);
	if (!found) throw new Error(`No layer "${id}"`);
	return found;
};

/** The golden fleet with invoice-extractor settled, so no update blocks the device operations. */
function settledSeed(agentRelease = "0.9.4") {
	const seed = sampleFleet();
	const live = seed.live[IDS.edge];
	live.rollouts = [];
	const inspection = live.inspection?.value;
	if (!inspection) throw new Error("the sample has no live read of edge");
	const invoice = inspection.placements.find(
		(row) => row.id === "invoice-extractor",
	);
	if (!invoice) throw new Error("the sample has no invoice-extractor");
	invoice.observed_state = "running";
	invoice.applied_revision = invoice.config_revision;
	invoice.ready_replicas = 1;
	if (inspection.agent) {
		inspection.agent.release_version = agentRelease;
		inspection.agent.release_sequence = agentRelease === "0.9.4" ? 44 : null;
	}
	inspection.hostOperation = null;
	return seed;
}

/** The first release cut from this work: agent 0.1.1 as release number 3, valid for a year. */
const NEXT_RELEASE: TestReleaseOptions = {
	version: "0.1.1",
	sequence: 3,
	minimum: 1,
};

/** edge-berlin-01 on the given agent, on a hub that serves a release list signed here. */
async function openWithRelease(agent: string, release: TestReleaseOptions) {
	const fake = await createFakeWorkspace(settledSeed(agent));
	await publishTestRelease(fake, release);
	return open(IDS.edge, { fake });
}

const hubWrites = (view: DeviceView, method: string, path: RegExp) =>
	view.fake.api
		.writes()
		.filter(([sent, target]) => sent === method && path.test(target));

describe("layers", () => {
	test("the device as layers, from the host down to the danger zone", async () => {
		const view = await open(IDS.edge);
		const ids = Array.from(
			view.container.querySelectorAll("[id^=device-layer-], #danger-zone"),
			(element) => element.id,
		);
		expect(ids).toEqual([
			"device-layer-host",
			"device-layer-agent",
			"device-layer-isolation",
			"device-layer-capacity",
			"device-layer-subscription",
			"device-layer-identity",
			"danger-zone",
		]);
		expect(primaries()).toBe(1);
		expect(text(view.container)).not.toMatch(MACHINE);
	});

	test("host and agent facts with their source; isolation with the network warning", async () => {
		const view = await open(IDS.edge);
		const host = text(layer(view, "host"));
		expect(host).toContain("PlatformLinux");
		expect(host).toContain("Last boot");
		expect(layer(view, "host").querySelector("[data-stamp]")).not.toBeNull();
		const agent = text(layer(view, "agent"));
		expect(agent).toContain("Running0.9.4live read");
		expect(agent).toContain(
			"Latest verified release0.9.4· release number 44 · signed by",
		);
		expect(agent).toContain("Remote updateAvailable on this device");
		expect(agent).toContain("Background tasks1 running normally");
		const isolation = text(layer(view, "isolation"));
		expect(isolation).toContain("Sandbox required");
		expect(isolation).toContain(
			"Sandboxed services still share the device's network.",
		);
		expect(isolation).toContain("Landlock ABI6");
	});

	test("network addresses: left out of the read for the owner, listed once the device reports them", async () => {
		const leftOut = text(layer(await open(IDS.edge), "host"));
		expect(leftOut).toContain("Network addressesNot included in this read.");
		expect(leftOut).not.toContain("whole-device View status");
		await kit.resetDevices();

		const seed = sampleFleet();
		const inspection = seed.live[IDS.edge].inspection?.value;
		if (!inspection) throw new Error("the sample has no live read of edge");
		inspection.network = {
			interfaces: [
				{ name: "lo", loopback: true, addresses: ["127.0.0.1"] },
				{ name: "eth0", loopback: false, addresses: ["10.20.4.17"] },
			],
		};
		const host = text(layer(await open(IDS.edge, { seed }), "host"));
		expect(host).toContain("Network addresses10.20.4.17");
		expect(host).not.toContain("127.0.0.1");
	});

	test("someone the device is shared with sees no danger zone and can't rename", async () => {
		const view = await open(IDS.lab);
		expect(view.container.querySelector("#danger-zone")).toBeNull();
		expect(text(layer(view, "identity"))).toContain(
			"Only the owner can rename this device.",
		);
		expect(queryByRole("button", "Rename…", view.container)).toBeNull();
	});
});

describe("host operations", () => {
	test("reboot waits for a running update: disabled with the reason, and nothing is sent", async () => {
		const view = await open(IDS.edge);
		const host = layer(view, "host");
		const reboot = byRole("button", "Reboot device…", host);
		expect(reboot.getAttribute("aria-disabled")).toBe("true");
		expect(text(host)).toMatch(
			/Wait for invoice-extractor.s update to finish \(by .+ at the latest\)\./,
		);
		await click(reboot);
		expect(queryByRole("alertdialog")).toBeNull();
		expect(commandTypes(view)).not.toContain("reboot");
	});

	test("reboot: consequence preview with an acknowledgement, one command, tracked in Activity", async () => {
		const view = await open(IDS.edge, { seed: settledSeed() });
		await click(byRole("button", "Reboot device…", layer(view, "host")));
		const sheet = inPortal("alertdialog");
		expect(text(sheet)).toContain("Reboot edge-berlin-01?");
		expect(text(sheet)).toContain(
			"edge-berlin-01 restarts. invoice-extractor and support-bot stop, then start again as requested.",
		);
		expect(text(sheet)).toContain("nightly-sync stays stopped, as you asked.");
		const confirm = sheet.querySelector("[data-confirm]") as HTMLElement;
		expect(confirm.getAttribute("aria-disabled")).toBe("true");
		await click(confirm);
		expect(commandTypes(view)).not.toContain("reboot");
		await click(
			byRole("checkbox", "Interrupt the running services now", sheet),
		);
		await click(sheet.querySelector("[data-confirm]") as HTMLElement);
		await view.settle();
		const sent = view.fake.api.commands.filter(([, type]) => type === "reboot");
		expect(sent).toHaveLength(1);
		expect(sent[0][2].expected_boot_id).toBe(view.fake.agent(IDS.edge).bootId);
		expect(
			view.fake.workspace.activity
				.list()
				.some((item) => item.kind === "reboot"),
		).toBe(true);
		expect(layer(view, "host").querySelector("[data-result]")).not.toBeNull();
	});

	test("the agent is current: Update agent… is disabled with that reason; Check reports the release", async () => {
		const view = await open(IDS.edge, { seed: settledSeed() });
		const agent = layer(view, "agent");
		const update = byRole("button", "Update agent…", agent);
		expect(update.getAttribute("aria-disabled")).toBe("true");
		expect(text(agent)).toContain(
			"Already running the latest verified release, 0.9.4.",
		);
		await click(update);
		expect(commandTypes(view)).not.toContain("update_agent");
		await click(byRole("button", "Check for agent update", agent));
		await view.settle();
		expect(text(agent)).toMatch(
			/Checked at \d\d:\d\d:\d\d: 0\.9\.4 is the latest verified release\./,
		);
	});

	test("an older agent release can be updated: acknowledgement, then the signed release goes to the device", async () => {
		const view = await open(IDS.edge, { seed: settledSeed("0.9.2") });
		const agent = layer(view, "agent");
		await click(byRole("button", "Update agent…", agent));
		const sheet = inPortal("alertdialog");
		expect(text(sheet)).toContain(
			"Installs verified release 0.9.4 (release number 44). If the new agent doesn't start, the device rolls back to 0.9.2.",
		);
		await click(byRole("checkbox", undefined, sheet));
		await click(sheet.querySelector("[data-confirm]") as HTMLElement);
		await view.settle();
		const sent = view.fake.api.commands.filter(
			([, type]) => type === "update_agent",
		);
		expect(sent).toHaveLength(1);
		expect(typeof sent[0][2].release_jws).toBe("string");
		expect(
			view.fake.workspace.activity
				.list()
				.some((item) => item.kind === "agent_update"),
		).toBe(true);
	});

	test("a new release number with the same version can update both bundled engines", async () => {
		const view = await openWithRelease("0.9.4", { sequence: 45 });
		const agent = layer(view, "agent");
		expect(text(agent)).toContain(
			"The agent binary contains the orchestrator and workflow runtime. Updating the agent replaces both.",
		);
		const update = byRole("button", "Update agent…", agent);
		expect(update.getAttribute("aria-disabled")).toBeNull();
		await click(update);
		const sheet = inPortal("alertdialog");
		expect(text(sheet)).toContain("release number 45");
		await click(byRole("checkbox", undefined, sheet));
		await click(sheet.querySelector("[data-confirm]") as HTMLElement);
		await view.settle();
		expect(commandTypes(view)).toContain("update_agent");
	});

	test("a newer version label cannot offer an older signed release number", async () => {
		const view = await openWithRelease("0.9.4", {
			version: "0.10.0",
			sequence: 43,
		});
		const update = byRole("button", "Update agent…", layer(view, "agent"));
		expect(update.getAttribute("aria-disabled")).toBe("true");
		await click(update);
		expect(queryByRole("alertdialog")).toBeNull();
		expect(commandTypes(view)).not.toContain("update_agent");
	});

	test("no verified release in hand: Update agent… is disabled with the reason, never a click into nothing", async () => {
		const seed = settledSeed("0.9.2");
		seed.latestRelease = undefined;
		const view = await open(IDS.edge, { seed });
		const agent = layer(view, "agent");
		expect(text(agent)).toContain(
			"Latest verified releaseCouldn't be read from the hub",
		);
		const update = byRole("button", "Update agent…", agent);
		expect(update.getAttribute("aria-disabled")).toBe("true");
		expect(text(agent)).toContain(
			"The latest verified release couldn't be read from the hub. Check for an agent update again.",
		);
		await click(update);
		expect(queryByRole("alertdialog")).toBeNull();
		expect(commandTypes(view)).not.toContain("update_agent");
	});

	test("a release with a far end date shows no end; inside its last 30 days it says when it runs out", async () => {
		const far = await openWithRelease("0.9.4", {});
		const shown = text(layer(far, "agent"));
		expect(shown).toContain(
			"Latest verified release0.9.4· release number 44 · signed by",
		);
		expect(shown).toMatch(/signed by[^·]+·Hub status/);
		expect(shown).not.toContain("runs out");
		expect(shown).not.toContain("expires");
		await kit.resetDevices();

		const soon = await openWithRelease("0.9.4", { endsInS: 12 * DAY_S });
		expect(text(layer(soon, "agent"))).toMatch(
			/signed by[^·]+· runs out on [^·]+·Hub status/,
		);
	});

	test("an agent before 0.1.1 can't take a release valid for longer than 30 days: disabled with the reason", async () => {
		const year = await openWithRelease("0.1.0", NEXT_RELEASE);
		const agent = layer(year, "agent");
		const update = byRole("button", "Update agent…", agent);
		expect(update.getAttribute("aria-disabled")).toBe("true");
		expect(text(agent)).toContain(
			"This device's agent (0.1.0) only accepts releases valid for 30 days or less. Ask the hub operator to renew the release for 30 days, or set this device up again.",
		);
		await click(update);
		expect(queryByRole("alertdialog")).toBeNull();
		expect(commandTypes(year)).not.toContain("update_agent");
		await kit.resetDevices();

		// The bridge release: signed for exactly 30 days, so the same agent takes it.
		const bridge = await openWithRelease("0.1.0", {
			...NEXT_RELEASE,
			issuedAgoS: 300,
			endsInS: 30 * DAY_S - 300,
		});
		const open = byRole("button", "Update agent…", layer(bridge, "agent"));
		expect(open.getAttribute("aria-disabled")).toBeNull();
		expect(text(layer(bridge, "agent"))).not.toContain("only accepts releases");
		await kit.resetDevices();

		// From 0.1.1 on the lifetime no longer matters.
		const current = await openWithRelease("0.1.1", {
			...NEXT_RELEASE,
			version: "0.1.2",
			sequence: 4,
		});
		expect(
			byRole("button", "Update agent…", layer(current, "agent")).getAttribute(
				"aria-disabled",
			),
		).toBeNull();
	});

	test("a release that fails a check after it was verified is not offered and not sent", async () => {
		const fake = await createFakeWorkspace(settledSeed("0.9.2"));
		const published = await publishTestRelease(fake, {});
		const view = await open(IDS.edge, { fake });
		const agent = layer(view, "agent");
		expect(
			byRole("button", "Update agent…", agent).getAttribute("aria-disabled"),
		).toBeNull();

		const [head, body] = published.jws.split(".");
		published.serve(`${head}.${body}.${"A".repeat(86)}`);
		await click(byRole("button", "Check for agent update", agent));
		await view.settle();

		expect(text(agent)).toContain(
			"Latest verified releaseThe hub's agent release failed a check·Hub status",
		);
		expect(text(agent)).toMatch(
			/Checked at \d\d:\d\d:\d\d\. The hub.s agent release failed a check: it isn.t signed by a key the hub operator pinned\./,
		);
		const update = byRole("button", "Update agent…", agent);
		expect(update.getAttribute("aria-disabled")).toBe("true");
		expect(text(agent)).toContain(
			"The hub's agent release failed a check, so there is nothing to install. Hub status says which.",
		);
		await click(update);
		expect(queryByRole("alertdialog")).toBeNull();
		expect(commandTypes(view)).not.toContain("update_agent");
	});

	test("a release that has run out is named with its date and blocks the update", async () => {
		const view = await openWithRelease("0.9.2", {
			issuedAgoS: 40 * DAY_S,
			endsInS: -DAY_S,
		});
		const agent = layer(view, "agent");
		expect(text(agent)).toMatch(
			/Latest verified releaseThe hub.s agent release ran out on [^·]+·Hub status/,
		);
		expect(
			byRole("button", "Update agent…", agent).getAttribute("aria-disabled"),
		).toBe("true");
		expect(text(agent)).toMatch(
			/The hub.s agent release ran out on .+\. Updates wait until the hub operator publishes or renews a release\./,
		);
	});

	test("the device's last operation shows whoever started it", async () => {
		const view = await open(IDS.edge);
		const line = layer(view, "agent").querySelector<HTMLElement>(
			"[data-host-operation=update_agent]",
		);
		if (!line) throw new Error("no operation line");
		expect(text(line)).toMatch(/^DoneAgent update · .+ · started by you/);
	});

	test("not Linux: a sentence instead of Reboot and Update, and nothing to click", async () => {
		const view = await open(IDS.studio);
		const host = layer(view, "host");
		expect(queryByRole("button", "Reboot device…", host)).toBeNull();
		expect(text(host)).toContain(
			"Remote reboot needs Linux with systemd. Restart this macOS device from the device itself.",
		);
		const agent = layer(view, "agent");
		expect(queryByRole("button", "Update agent…", agent)).toBeNull();
		expect(text(agent)).toContain("Not supported on this system");
		expect(text(layer(view, "isolation"))).toContain(
			"Sandboxing needs Linux. Services here run as the agent with full device access, so use a dedicated account.",
		);
	});

	test("macOS with launchd can update the agent while remote reboot stays unavailable", async () => {
		const seed = settledSeed();
		const inspection = seed.live[IDS.studio].inspection?.value;
		if (!inspection?.agent || !inspection.hostOperations)
			throw new Error("the sample has no studio agent");
		inspection.agent.release_sequence = 43;
		inspection.hostOperations.update_agent = true;
		const view = await open(IDS.studio, { seed });
		const agent = layer(view, "agent");
		expect(text(agent)).toContain("Remote updatemacOS with launchd");
		expect(
			queryByRole("button", "Reboot device…", layer(view, "host")),
		).toBeNull();
		await click(byRole("button", "Update agent…", agent));
		const sheet = inPortal("alertdialog");
		expect(text(sheet)).toContain("The agent restarts on studio-mac-mini.");
		expect(text(sheet)).not.toContain("studio-mac-mini restarts.");
		await click(byRole("checkbox", undefined, sheet));
		await click(sheet.querySelector("[data-confirm]") as HTMLElement);
		await view.settle();
		expect(commandTypes(view)).toContain("update_agent");
	});
});

describe("capacity", () => {
	test("app storage is read only on request and shows what the device keeps", async () => {
		const view = await open(IDS.edge);
		const capacity = layer(view, "capacity");
		expect(text(capacity)).toContain("Services3");
		expect(text(capacity)).toContain("Certificate slots2 of 32");
		expect(text(capacity)).toContain("Not read yet.");
		const usage = () =>
			view.fake.api.commands.filter(
				([, type, command]) =>
					type === "artifact" &&
					(command.request as { kind?: string })?.kind === "usage",
			).length;
		expect(usage()).toBe(0);
		await click(byRole("button", "Check app storage", capacity));
		await view.settle();
		expect(usage()).toBe(1);
		expect(text(capacity)).toContain("App versions kept");
		expect(text(capacity)).toContain("Read from the device at");
		expect(byRole("button", "Check again", capacity)).toBeTruthy();
	});
});

describe("encrypted status subscription", () => {
	test("shows when it expires; Renew signs a new one", async () => {
		const view = await open(IDS.edge);
		const subscription = layer(view, "subscription");
		expect(text(subscription)).toMatch(/Expires.+\(in \d+ months\)/);
		const before = hubWrites(view, "PUT", /fleet\/readers\//).length;
		await click(byRole("button", "Renew", subscription));
		await view.settle();
		expect(hubWrites(view, "PUT", /fleet\/readers\//).length).toBe(before + 1);
		expect(subscription.querySelector("[data-result]")).not.toBeNull();
	});

	test("Stop receiving… asks first, then removes this computer's subscription on the hub", async () => {
		const view = await open(IDS.edge);
		const subscription = layer(view, "subscription");
		await click(byRole("button", "Stop receiving…", subscription));
		const sheet = inPortal("alertdialog");
		expect(text(sheet)).toContain(
			"Stop receiving encrypted status of edge-berlin-01?",
		);
		expect(hubWrites(view, "DELETE", /fleet\/readers\//)).toHaveLength(0);
		await click(sheet.querySelector("[data-confirm]") as HTMLElement);
		await view.settle();
		expect(hubWrites(view, "DELETE", /fleet\/readers\//)).toHaveLength(1);
	});

	test("locked: Renew is disabled with the reason and sends nothing", async () => {
		const view = await open(IDS.edge, { unlock: "none" });
		const subscription = layer(view, "subscription");
		const renew = byRole("button", "Renew", subscription);
		expect(renew.getAttribute("aria-disabled")).toBe("true");
		const before = view.fake.api.writes().length;
		await click(renew);
		expect(view.fake.api.writes().length).toBe(before);
	});
});

describe("rename", () => {
	test("Rename… saves a display name on the hub and the page takes it over", async () => {
		const view = await open(IDS.edge);
		await click(byRole("button", "Rename…", layer(view, "identity")));
		await view.settle();
		const sheet = inPortal("dialog");
		const input = byRole("textbox", "Display name", sheet);
		const save = byRole("button", "Save name", sheet);
		expect(save.getAttribute("aria-disabled")).toBe("true");
		await typeInto(input, "  Berlin edge  ");
		await click(byRole("button", "Save name", sheet));
		await view.settle();
		const sent = hubWrites(view, "PATCH", new RegExp(`devices/${IDS.edge}$`));
		expect(sent).toHaveLength(1);
		expect(sent[0][2]).toEqual({ display_name: "Berlin edge" });
		expect(queryByRole("dialog")).toBeNull();
		expect(byRole("heading", /Berlin edge/, view.container)).toBeTruthy();
		expect(text(layer(view, "identity"))).toContain("set up as edge-berlin-01");
	});

	test("a name that is too long is refused before anything is sent", async () => {
		const view = await open(IDS.edge, { action: "rename" });
		const sheet = inPortal("dialog");
		await typeInto(byRole("textbox", "Display name", sheet), "x".repeat(65));
		expect(text(sheet)).toContain("Use at most 64 characters.");
		await click(byRole("button", "Save name", sheet));
		expect(hubWrites(view, "PATCH", /devices\//)).toHaveLength(0);
	});

	test("older hub: Rename… is disabled and says the name can't be changed", async () => {
		const view = await open(IDS.edge, { hubVersion: "old" });
		const identity = layer(view, "identity");
		const rename = byRole("button", "Rename…", identity);
		expect(rename.getAttribute("aria-disabled")).toBe("true");
		expect(text(identity)).toContain("Set at setup and can't be changed.");
		await click(rename);
		expect(queryByRole("dialog")).toBeNull();
		expect(hubWrites(view, "PATCH", /devices\//)).toHaveLength(0);
		expect(queryByRole("alert", undefined, view.container)).toBeNull();
	});
});

describe("danger zone", () => {
	const revoke = async (view: DeviceView, also?: RegExp) => {
		await click(byRole("button", "Revoke edge-berlin-01…", view.container));
		const sheet = inPortal("alertdialog");
		if (also) await click(byRole("checkbox", also, sheet));
		await click(byRole("button", "Continue", sheet));
		const confirm = () => sheet.querySelector("[data-confirm]") as HTMLElement;
		expect(confirm().getAttribute("aria-disabled")).toBe("true");
		await typeInto(byRole("textbox", undefined, sheet), "edge-berlin-01");
		await click(confirm());
		await view.settle();
	};

	test("revoke is a review and the typed name; cloud access stays unless asked", async () => {
		const view = await open(IDS.edge);
		await click(byRole("button", "Revoke edge-berlin-01…", view.container));
		const sheet = inPortal("alertdialog");
		const review = text(sheet);
		expect(review).toContain("Revoke edge-berlin-01?");
		expect(review).toContain("Changes now");
		expect(review).toContain("Who loses access");
		expect(review).toContain("Keeps running");
		expect(review).toContain("Cloud & credentials");
		expect(review).toContain("No, this is permanent.");
		const cloud = byRole(
			"checkbox",
			"Also revoke its cloud access and spending limits",
			sheet,
		);
		expect(cloud.getAttribute("aria-checked")).toBe("false");
		expect(hubWrites(view, "DELETE", /devices\//)).toHaveLength(0);
		await click(byRole("button", "Continue", sheet));
		const confirm = sheet.querySelector("[data-confirm]") as HTMLElement;
		expect(confirm.getAttribute("aria-disabled")).toBe("true");
		await typeInto(byRole("textbox", undefined, sheet), "edge-berlin-0");
		expect(
			(sheet.querySelector("[data-confirm]") as HTMLElement).getAttribute(
				"aria-disabled",
			),
		).toBe("true");
		await typeInto(byRole("textbox", undefined, sheet), "edge-berlin-01");
		await click(sheet.querySelector("[data-confirm]") as HTMLElement);
		await view.settle();
		const deletes = hubWrites(view, "DELETE", /devices\//).map(
			([, path]) => path,
		);
		expect(deletes).toEqual([`devices/${IDS.edge}`]);
		const page = text(view.container);
		expect(page).toContain("edge-berlin-01 is revoked.");
		expect(
			allByRole("tab", undefined, view.container).map((tab) =>
				text(tab).replace(/\d.*$/, ""),
			),
		).toEqual(["Overview", "Access", "Keys"]);
	});

	test("with the option ticked, the spending limit and the cloud access go first", async () => {
		const view = await open(IDS.edge);
		await revoke(view, /^Also revoke its cloud access/);
		const deletes = hubWrites(view, "DELETE", /devices\//).map(
			([, path]) => path,
		);
		expect(deletes).toHaveLength(3);
		expect(deletes[0]).toMatch(/billing-grants\//);
		expect(deletes[1]).toMatch(/resource-grants\//);
		expect(deletes[2]).toBe(`devices/${IDS.edge}`);
	});

	test("works without unlocking: no password, no command to the device", async () => {
		const view = await open(IDS.edge, { unlock: "none" });
		const commands = view.fake.api.commands.length;
		await revoke(view);
		expect(
			hubWrites(view, "DELETE", new RegExp(`devices/${IDS.edge}$`)),
		).toHaveLength(1);
		expect(view.fake.api.commands.length).toBe(commands);
		expect(text(view.container)).toContain("edge-berlin-01 is revoked.");
	});

	test("the revoke link opens the flow once and drops its one-shot parameter", async () => {
		const view = await open(IDS.edge, { action: "revoke" });
		await view.settle();
		expect(text(inPortal("alertdialog"))).toContain("Revoke edge-berlin-01?");
		expect(view.navigations.at(-1)?.href).not.toContain("action=");
		await click(byRole("button", "Cancel", inPortal("alertdialog")));
		await view.settle();
		expect(queryByRole("alertdialog")).toBeNull();
		expect(hubWrites(view, "DELETE", /devices\//)).toHaveLength(0);
	});
});

describe("older agent", () => {
	test("facts it doesn't report read as not reported, and no newer command is sent", async () => {
		const view = await open(IDS.edge, { agentFeatures: {} });
		expect(text(layer(view, "capacity"))).toContain(
			"Not reported. Update the device agent to see how much storage app versions use.",
		);
		expect(
			queryByRole("button", "Check app storage", layer(view, "capacity")),
		).toBeNull();
		expect(text(layer(view, "agent"))).toContain(
			"Background tasksNot reported by this agent version",
		);
		expect(text(layer(view, "host"))).toContain(
			"Network addressesNot reported by this agent version",
		);
		expect(queryByRole("alert", undefined, view.container)).toBeNull();
		const sent = commandTypes(view);
		for (const newer of ["host_operation", "operations"])
			expect(sent).not.toContain(newer);
		expect(
			view.fake.api.commands.filter(
				([, type, command]) =>
					type === "artifact" &&
					["usage", "prune"].includes(
						(command.request as { kind?: string })?.kind ?? "",
					),
			),
		).toEqual([]);
		expect(text(view.container)).not.toMatch(MACHINE);
	});
});
