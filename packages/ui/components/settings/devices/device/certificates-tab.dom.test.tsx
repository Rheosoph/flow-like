import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	SAMPLE_IDS,
	SAMPLE_NOW,
	sampleFleet,
} from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import type {
	DeviceRoute,
	InspectionPlus,
} from "../../../../lib/device-management/model/types";
import type { Capability } from "../../../../lib/device-management/types";
import {
	allByRole,
	byRole,
	click,
	inPortal,
	installDom,
	queryByRole,
} from "../testing/dom-harness";
import type { MountDevicesOptions } from "../testing/mount-devices";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { DeviceCertificatesTab } = await import("./certificates-tab");
const { useOverlayStore } = await import("../workspace/overlay-store");

afterEach(async () => {
	await cleanupDevices();
	useOverlayStore.getState().close();
	await dom.cleanup();
});
afterAll(dom.restore);

const IDS = SAMPLE_IDS;
const ACCOUNT = { kind: "account" } as const;
const EDGE_API = "24f6fe22-c6c2-4e15-9d37-7a41a379afb9";
const INTERNAL_MQTT = "93bcc1ef-5f49-4bb3-b90c-7b052822bf02";
const WAREHOUSE_CERT = "82ac7195-73a6-48a1-816f-7e37f7f67602";

/** R3: wire values, gate codes and the banned certificate jargon never reach the screen. */
const MACHINE =
	/[a-z]+_[a-z_]+|\bG\d{1,2}\b|\bCSR\b|\bACME\b|\bSAN\b|\bCA\b|delegat|placement|lets_encrypt/;

interface MountOptions extends MountDevicesOptions {
	certificateId?: string;
}

async function mount(deviceId: string, options: MountOptions = {}) {
	const { certificateId, ...rest } = options;
	const route: DeviceRoute = {
		screen: "device",
		deviceId,
		tab: "certificates",
		...(certificateId ? { certificateId } : {}),
	};
	const view = await mountDevices(
		<DeviceCertificatesTab route={route} scope={ACCOUNT} deviceId={deviceId} />,
		rest,
	);
	await view.settle();
	await view.settle();
	return view;
}

type View = Awaited<ReturnType<typeof mount>>;

const text = (root: ParentNode = document.body) =>
	(root.textContent ?? "").replace(/\s+/g, " ");

const block = (view: View, id: string) => {
	const found = view.container.querySelector<HTMLElement>(`#${id}`);
	if (!found) throw new Error(`No block "${id}"`);
	return found;
};

const row = (view: View, certificateId: string) => {
	const found = view.container.querySelector<HTMLElement>(
		`[data-certificate="${certificateId}"]`,
	);
	if (!found) throw new Error(`No certificate row ${certificateId}`);
	return found;
};

/** What one device was asked, in order (the fleet's other devices load on their own). */
const commandTypes = (view: View, deviceId: string = IDS.edge) =>
	view.fake.api.commands
		.filter(([device]) => device === deviceId)
		.map(([, type]) => type);

const stamps = (root: ParentNode) =>
	Array.from(root.querySelectorAll("[data-stamp]"), (stamp) =>
		stamp.getAttribute("data-src"),
	);

describe("certificate list", () => {
	test("the owner sees slots, each certificate with validity, names, renewal and use, and every block states its source", async () => {
		const view = await mount(IDS.edge);
		const list = block(view, "device-certificates");
		expect(text(list)).toContain(
			"3 of 32 certificate slots used (certificates, pending requests and Let's Encrypt policies).",
		);
		const mqtt = text(row(view, INTERNAL_MQTT));
		expect(mqtt).toContain("internal-mqtt");
		expect(mqtt).toContain("Expires in 5 d");
		expect(mqtt).toContain("mqtt.lab.internal, 10.0.4.20");
		expect(mqtt).toContain("Automatic (your authority)");
		expect(mqtt).toContain("failing");
		expect(mqtt).toContain("Not used");
		const api = text(row(view, EDGE_API));
		expect(api).toContain("Valid");
		expect(api).toContain("edge-berlin.rheosoph.example");
		expect(api).toContain("Automatic (Let's Encrypt)");
		expect(api).toContain("support-bot");
		expect(stamps(list)).toEqual(["hub", "live"]);
		for (const id of [
			"device-certificate-requests",
			"device-certificate-renewal",
			"device-certificate-acme",
			"device-certificate-reminders",
		])
			expect(stamps(block(view, id)).length).toBeGreaterThan(0);
		expect(view.container.querySelectorAll("[data-dv-primary]")).toHaveLength(
			0,
		);
		expect(text(view.container)).not.toMatch(MACHINE);
	});

	test("a row opens to issuer, subject, dates, fingerprint, certificate ID and version", async () => {
		const view = await mount(IDS.edge);
		const toggle = byRole("button", "internal-mqtt", row(view, INTERNAL_MQTT));
		expect(toggle.getAttribute("aria-expanded")).toBe("false");
		await click(toggle);
		const detail = view.container.querySelector(
			`[data-certificate-detail="${INTERNAL_MQTT}"]`,
		);
		const shown = text(detail as HTMLElement);
		expect(shown).toContain("IssuerCN=Rheosoph Internal service issuer");
		expect(shown).toContain("SubjectCN=mqtt.lab.internal");
		expect(shown).toContain("Valid from");
		expect(shown).toContain("Fingerprint");
		expect(shown).toContain("Certificate ID93bcc1ef");
		expect(shown).toContain("reminders name this ID");
		expect(shown).toContain("Version2");
		await click(toggle);
		expect(
			view.container.querySelector(
				`[data-certificate-detail="${INTERNAL_MQTT}"]`,
			),
		).toBeNull();
	});

	test("the reminder link's certificate is marked and open; one that is gone is said so", async () => {
		const view = await mount(IDS.edge, { certificateId: EDGE_API });
		expect(row(view, EDGE_API).getAttribute("aria-selected")).toBe("true");
		expect(
			view.container.querySelector(`[data-certificate-detail="${EDGE_API}"]`),
		).not.toBeNull();
		expect(row(view, INTERNAL_MQTT).getAttribute("aria-selected")).toBeNull();
		await view.unmount();
		const gone = await mount(IDS.edge, {
			certificateId: "11111111-2222-4333-8444-555555555555",
		});
		expect(text(gone.container)).toContain(
			"The certificate this link points to isn't on edge-berlin-01 any more.",
		);
	});
});

describe("states", () => {
	test("locked: the hub's IDs and expiry with 'Unlock to see names and details', and nothing is asked of the device", async () => {
		const view = await mount(IDS.edge, { unlock: "none" });
		const list = block(view, "device-certificates");
		expect(text(list)).toContain(
			"Unlock edge-berlin-01 to see names and details. The hub knows only IDs, fingerprints and expiry dates.",
		);
		const mqtt = text(row(view, INTERNAL_MQTT));
		// Without a name the ID is the row's title, once.
		expect(mqtt.match(/93bcc1ef/g)).toHaveLength(1);
		expect(mqtt).toContain("Expires in 5 d");
		expect(mqtt).toContain("Name shows after a live read");
		expect(mqtt).not.toContain("internal-mqtt");
		expect(stamps(list)).toEqual(["hub"]);
		expect(view.container.querySelector("[data-certificate-slots]")).toBeNull();
		expect(
			view.container.querySelector("#device-certificate-requests"),
		).toBeNull();
		const renew = byRole("button", "Renew…", row(view, INTERNAL_MQTT));
		expect(renew.getAttribute("aria-disabled")).toBe("true");
		await click(renew);
		expect(queryByRole("dialog")).toBeNull();
		expect(commandTypes(view)).toEqual([]);
		await click(byRole("button", "Unlock…", list));
		expect(useOverlayStore.getState().overlay).toMatchObject({
			kind: "unlock",
			deviceId: IDS.edge,
		});
	});

	test("offline: last known IDs, the reason on the disabled control, and no command", async () => {
		const view = await mount(IDS.warehouse);
		const list = block(view, "device-certificates");
		expect(text(list)).toContain(
			"warehouse-pi is offline, so names and details can't be read.",
		);
		const expired = row(view, WAREHOUSE_CERT);
		expect(text(expired)).toContain("Expired");
		const before = commandTypes(view, IDS.warehouse).length;
		const renew = byRole("button", "Renew…", expired);
		expect(renew.getAttribute("aria-disabled")).toBe("true");
		expect(text(expired)).toMatch(/offline.*needs a live connection/);
		await click(renew);
		expect(commandTypes(view, IDS.warehouse)).toHaveLength(before);
		expect(queryByRole("button", /More for/, expired)).toBeNull();
	});

	test("never reported and no certificates are told apart", async () => {
		const never = await mount(IDS.cold, { unlock: "none" });
		expect(text(never.container)).toContain(
			"The device hasn't reported certificates",
		);
		expect(text(never.container)).toContain("It hasn't checked in yet.");
		await never.unmount();
		const empty = await mount(IDS.studio);
		expect(text(empty.container)).toContain("No certificates on this device");
		expect(text(empty.container)).toContain("0 of 32 certificate slots used");
		expect(
			empty.container.querySelector("#device-certificate-requests"),
		).toBeNull();
		expect(
			empty.container.querySelector("#device-certificate-acme"),
		).toBeNull();
	});

	test("access to one app only: no certificates, the permission that is missing, and no request for them", async () => {
		const view = await mount(IDS.lab, { unlock: "none" });
		expect(text(view.container)).toContain(
			"Needs whole-device View status or Manage certificates.",
		);
		expect(text(view.container)).toContain(
			"You can see certificates used by your services only",
		);
		expect(
			view.container.querySelector("#device-certificate-reminders"),
		).toBeNull();
		expect(allByRole("button", /Add certificate|Refresh/)).toHaveLength(0);
		expect(commandTypes(view, IDS.lab)).toEqual([]);
	});

	test("a failing hub keeps the list and says the data is older", async () => {
		const view = await mount(IDS.edge, { unlock: "none" });
		const list = block(view, "device-certificates");
		// An answer the app can't read is a failed refresh that isn't retried, unlike a 503.
		view.fake.api.on("GET", "devices/:id/certificate-inventory", () => ({
			certificates: "none",
		}));
		await click(byRole("button", "Refresh", list));
		await view.settle();
		const stamp = list.querySelector("[data-stamp]");
		expect(stamp?.getAttribute("data-age")).toBe("error");
		expect(text(stamp as HTMLElement)).toContain("couldn't refresh");
		expect(text(row(view, INTERNAL_MQTT))).toContain("Expires in 5 d");
		expect(queryByRole("alert", undefined, view.container)).toBeNull();
	});

	test("a live read that fails says so, keeps the hub's list and can be tried again", async () => {
		const fake = await createFakeWorkspace(undefined, { unlock: "none" });
		const restore = fake.api
			.agent(IDS.edge)
			.reject("certificates", "failed", "storage error");
		await fake.unlock(IDS.edge, { connectLive: true });
		const view = await mount(IDS.edge, { fake });
		const list = block(view, "device-certificates");
		expect(text(list)).toContain(
			"Names and details couldn't be read from edge-berlin-01.",
		);
		expect(text(row(view, INTERNAL_MQTT))).toContain("93bcc1ef");
		expect(text(list)).not.toContain("storage error");
		restore();
		await click(byRole("button", "Try again", list));
		await view.settle();
		await view.settle();
		expect(text(row(view, INTERNAL_MQTT))).toContain("internal-mqtt");
	});
});

/** lab-gpu-02 shared for the whole device, unlocked and connected, with the given permissions. */
function sharedSeed(
	capabilities: Capability[],
	inspection: Partial<InspectionPlus> = {},
) {
	const seed = sampleFleet();
	const edge = seed.live[IDS.edge];
	const value = edge?.inspection?.value;
	const grant = seed.myAccess?.[IDS.lab]?.grants[0];
	if (!edge?.certificates || !value || !grant)
		throw new Error("the sample fleet no longer has what this test builds on");
	grant.scope = { kind: "device" };
	grant.capabilities = capabilities;
	seed.live[IDS.lab] = {
		state: {
			kind: "live",
			transport: "webrtc",
			expiresAt: SAMPLE_NOW + 240,
			bootId: "a1b2c3d4e5f60718",
			connectedAt: SAMPLE_NOW - 60,
		},
		inspection: {
			readAt: SAMPLE_NOW - 10,
			value: {
				...structuredClone(value),
				device_id: IDS.lab,
				boot_id: "a1b2c3d4e5f60718",
				placements: [],
				can_manage_certificates: capabilities.includes("manage_certificates"),
				can_delegate_certificate_renewal: false,
				hostOperation: null,
				...inspection,
			},
		},
		certificates: structuredClone(edge.certificates),
		certificateRequests: structuredClone(edge.certificateRequests ?? []),
	};
	seed.certInventory[IDS.lab] = structuredClone(seed.certInventory[IDS.edge]);
	return seed;
}

/** The owner's edge-berlin-01 with an agent that reports other certificate abilities. */
function agentSeed(change: (value: InspectionPlus) => void) {
	const seed = sampleFleet();
	const value = seed.live[IDS.edge]?.inspection?.value;
	if (!value) throw new Error("the sample fleet has no live read of edge");
	change(value);
	return seed;
}

async function openMenu(view: View, certificateId: string) {
	await click(byRole("button", /^More for /, row(view, certificateId)));
	return inPortal("menu");
}

const menuItems = (menu: HTMLElement) =>
	allByRole("menuitem", undefined, menu).map((item) =>
		item.getAttribute("data-menu-item"),
	);

describe("who may do what", () => {
	test("delete is disabled while a service uses the certificate, with the reason, and sends nothing", async () => {
		const view = await mount(IDS.edge);
		const menu = await openMenu(view, EDGE_API);
		const remove = byRole("menuitem", /Delete…/, menu);
		expect(remove.getAttribute("aria-disabled")).toBe("true");
		expect(text(remove)).toContain(
			"Used by 1 service. Assign another certificate first.",
		);
		const before = commandTypes(view).length;
		await click(remove);
		expect(queryByRole("alertdialog")).toBeNull();
		expect(commandTypes(view)).toHaveLength(before);
		const acme = menu.querySelector<HTMLElement>('[data-menu-item="acme"]');
		expect(text(acme ?? menu)).toContain("Already renews with Let's Encrypt");
		expect(acme?.getAttribute("aria-disabled")).toBe("true");
	});

	test("deleting an unused certificate asks first; cancel sends nothing, confirm sends one command and reports it", async () => {
		const view = await mount(IDS.edge);
		const sent = () =>
			view.fake.api.commands.filter(
				([, type]) => type === "delete_certificate",
			);
		await click(
			byRole("menuitem", /Delete…/, await openMenu(view, INTERNAL_MQTT)),
		);
		let sheet = byRole("alertdialog");
		expect(text(sheet)).toContain("Delete certificate internal-mqtt?");
		expect(text(sheet)).toContain("Its ID can never be reused.");
		expect(text(sheet)).toContain("Immediately. Automatic renewal stops.");
		expect(text(sheet)).toContain("No, this is permanent.");
		await click(byRole("button", "Cancel", sheet));
		await view.settle();
		expect(sent()).toHaveLength(0);

		await click(
			byRole("menuitem", /Delete…/, await openMenu(view, INTERNAL_MQTT)),
		);
		sheet = byRole("alertdialog");
		await click(byRole("button", "Delete certificate internal-mqtt", sheet));
		await view.settle();
		await view.settle();
		expect(sent()).toHaveLength(1);
		expect(sent()[0]?.[2]).toMatchObject({
			certificate_id: INTERNAL_MQTT,
			expected_revision: 2,
		});
		const list = block(view, "device-certificates");
		expect(text(list)).toContain("Delete certificate internal-mqtt: done.");
		expect(
			view.container.querySelector(`[data-certificate="${INTERNAL_MQTT}"]`),
		).toBeNull();
		expect(text(list)).toContain("2 of 32 certificate slots used");
	});

	test("the device's refusal is quoted as the reason", async () => {
		const view = await mount(IDS.edge);
		view.fake.api
			.agent(IDS.edge)
			.reject(
				"delete_certificate",
				"revision_conflict",
				"Certificate revision changed",
			);
		await click(
			byRole("menuitem", /Delete…/, await openMenu(view, INTERNAL_MQTT)),
		);
		await click(
			byRole(
				"button",
				"Delete certificate internal-mqtt",
				byRole("alertdialog"),
			),
		);
		await view.settle();
		await view.settle();
		const shown = text(block(view, "device-certificates"));
		expect(shown).toContain("was refused by the device");
		expect(shown).toContain("Certificate revision changed");
		expect(row(view, INTERNAL_MQTT)).toBeDefined();
	});

	test("someone who manages certificates on a shared device never reads or sees the owner's renewal settings", async () => {
		const view = await mount(IDS.lab, {
			seed: sharedSeed(["status", "manage_certificates"]),
			unlock: [IDS.lab],
		});
		expect(text(row(view, INTERNAL_MQTT))).toContain("internal-mqtt");
		expect(text(row(view, INTERNAL_MQTT))).toContain(
			"Only the owner sees how it renews",
		);
		const types = commandTypes(view, IDS.lab);
		expect(types).toContain("certificates");
		expect(types).toContain("certificate_requests");
		expect(types).not.toContain("certificate_issuers");
		expect(types).not.toContain("acme_certificates");
		expect(text(view.container)).toContain(
			"Only the owner can set up automatic renewal.",
		);
		expect(
			view.container.querySelector("#device-certificate-renewal"),
		).toBeNull();
		expect(view.container.querySelector("#device-certificate-acme")).toBeNull();
		expect(text(block(view, "device-certificate-requests"))).toContain(
			"billing-api",
		);
		const menu = await openMenu(view, INTERNAL_MQTT);
		expect(menuItems(menu)).toEqual(["replace", "request", "all", "delete"]);
		expect(
			queryByRole(
				"button",
				"Sign with organisation authority…",
				view.container,
			),
		).not.toBeNull();
		expect(text(view.container)).toContain(
			"used by what you can see. Pending requests and Let's Encrypt policies count too.",
		);
	});

	test("someone who may only look sees the list and no control that changes anything", async () => {
		const view = await mount(IDS.lab, {
			seed: sharedSeed(["status"]),
			unlock: [IDS.lab],
		});
		expect(text(row(view, EDGE_API))).toContain("edge-berlin.rheosoph.example");
		expect(queryByRole("button", "Add certificate", view.container)).toBeNull();
		expect(queryByRole("button", "Renew…", view.container)).toBeNull();
		expect(queryByRole("button", /^More for /, view.container)).toBeNull();
		expect(
			view.container.querySelector("#device-certificate-requests"),
		).toBeNull();
		const types = commandTypes(view, IDS.lab);
		expect(types).toContain("certificates");
		for (const type of [
			"certificate_requests",
			"certificate_issuers",
			"acme_certificates",
		])
			expect(types).not.toContain(type);
		expect(byRole("button", "Refresh", view.container)).toBeDefined();
	});

	test("an agent from before signing requests and renewal: one sentence, none of those controls, none of those reads", async () => {
		const view = await mount(IDS.edge, {
			seed: agentSeed((value) => {
				value.certificate_issuance = undefined;
				value.certificate_acme = undefined;
			}),
		});
		expect(text(view.container)).toContain(
			"This device's agent can't create signing requests or renew certificates by itself yet.",
		);
		for (const id of [
			"device-certificate-requests",
			"device-certificate-renewal",
			"device-certificate-acme",
		])
			expect(view.container.querySelector(`#${id}`)).toBeNull();
		expect(menuItems(await openMenu(view, INTERNAL_MQTT))).toEqual([
			"replace",
			"all",
			"delete",
		]);
		const types = commandTypes(view);
		for (const type of [
			"certificate_requests",
			"certificate_issuers",
			"acme_certificates",
		])
			expect(types).not.toContain(type);
	});

	test("an agent from before remote certificate management: the hub's list and the way out, no certificate command", async () => {
		const view = await mount(IDS.edge, {
			seed: agentSeed((value) => {
				value.certificate_management = undefined;
				value.certificate_issuance = undefined;
				value.certificate_acme = undefined;
			}),
		});
		const list = block(view, "device-certificates");
		expect(text(list)).toContain(
			"Update the agent on edge-berlin-01 to manage certificates remotely.",
		);
		expect(text(row(view, INTERNAL_MQTT))).toContain("93bcc1ef");
		expect(queryByRole("button", "Renew…", view.container)).toBeNull();
		expect(queryByRole("button", "Add certificate", view.container)).toBeNull();
		expect(
			commandTypes(view).filter((type) => /certificate/.test(type)),
		).toEqual([]);
	});
});

describe("older hub and older agent", () => {
	test("a hub without reminder history: the tab works, the reminders block explains, and the missing route isn't retried", async () => {
		const view = await mount(IDS.edge, { hubVersion: "old" });
		expect(text(row(view, INTERNAL_MQTT))).toContain("internal-mqtt");
		const reminders = block(view, "device-certificate-reminders");
		expect(text(reminders)).toContain(
			"This hub can't list the reminders it already sent",
		);
		expect(queryByRole("alert", undefined, reminders)).toBeNull();
		expect(view.container.querySelector("[data-result=critical]")).toBeNull();
		await view.settle();
		await view.settle();
		const asked = (path: RegExp) =>
			view.fake.api.calls.filter(([, target]) => path.test(target)).length;
		expect(asked(/certificate-notices(\?|$)/)).toBeLessThanOrEqual(1);
		expect(asked(/certificate-notices\/mute/)).toBeLessThanOrEqual(1);
	});

	test("an agent without failure details: the fixed sentence, the device's own words, and no newer command", async () => {
		const view = await mount(IDS.edge, { agentFeatures: {} });
		const renewal = block(view, "device-certificate-renewal");
		expect(text(renewal)).toContain(
			"This device's agent doesn't report how often renewal failed or why.",
		);
		expect(renewal.querySelector("[data-failures]")).toBeNull();
		expect(
			queryByRole("alert", undefined, block(view, "device-certificate-acme")),
		).toBeNull();
		expect([
			...new Set(
				commandTypes(view).filter((type) => /certificate|acme/.test(type)),
			),
		]).toEqual([
			"certificates",
			"certificate_requests",
			"certificate_issuers",
			"acme_certificates",
		]);
		expect(view.container.querySelector("[data-result=critical]")).toBeNull();
	});

	test("failure details from the device: how often, and the cause in plain words instead of the device's", async () => {
		const seed = sampleFleet();
		const live = seed.live[IDS.edge];
		const policy = live?.acme?.[0];
		const issuer = live?.certificateIssuers?.[0];
		if (!policy || !issuer) throw new Error("the sample fleet changed");
		Object.assign(policy, {
			failures: 3,
			error_category: "dns",
			last_error: "NXDOMAIN looking up A for edge-berlin.rheosoph.example",
		});
		Object.assign(issuer, {
			not_after: SAMPLE_NOW + 40 * 86_400,
			failures: 2,
			last_error: "signing backend crashed (errno 5)",
		});
		const view = await mount(IDS.edge, { seed });
		const acme = text(block(view, "device-certificate-acme"));
		expect(acme).toContain("Let's Encrypt couldn't renew edge-api.");
		expect(acme).toContain("3 failed attempts in a row");
		expect(acme).toContain("The certificate authority couldn't find the name");
		expect(acme).not.toContain("NXDOMAIN");
		const renewal = text(block(view, "device-certificate-renewal"));
		expect(renewal).toContain("2 failed attempts in a row");
		expect(renewal).toContain("The cause isn't known yet.");
		expect(renewal).toContain(
			"The device says: “signing backend crashed (errno 5)”",
		);
		expect(text(row(view, EDGE_API))).toContain("failing");
		expect(byRole("button", "Fix renewal…", row(view, EDGE_API))).toBeDefined();
	});
});
