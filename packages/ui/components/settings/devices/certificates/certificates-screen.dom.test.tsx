import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { ApiResponseError } from "../../../../lib/api-error";
import { SAMPLE_IDS } from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import type { CertificatesRoute } from "../../../../lib/device-management/model/types";
import { byRole, click, installDom, queryByRole } from "../testing/dom-harness";
import type { MountDevicesOptions } from "../testing/mount-devices";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { useDevicesRoute } = await import("../routing/use-devices-route");
const { useOverlayStore } = await import("../workspace/overlay-store");
const { CertificatesScreen } = await import("./certificates-screen");

afterEach(async () => {
	await cleanupDevices();
	useOverlayStore.getState().close();
	await dom.cleanup();
});
afterAll(dom.restore);

const ACCOUNT = { kind: "account" } as const;
const HOME: CertificatesRoute = { screen: "certificates" };
const EDGE_API = "24f6fe22-c6c2-4e15-9d37-7a41a379afb9";
const INTERNAL_MQTT = "93bcc1ef-5f49-4bb3-b90c-7b052822bf02";
const WAREHOUSE_CERT = "82ac7195-73a6-48a1-816f-7e37f7f67602";
/** R3: wire values, condition keys and gate codes never reach the screen. */
const MACHINE_WORDS =
	/[a-z]+_[a-z_]+|\bG\d{1,2}\b|\bD\d{1,2}\b|lets_encrypt|acme|csr|vault\b/;

function mount(
	options: MountDevicesOptions = {},
	route: CertificatesRoute = HOME,
) {
	return mountDevices(
		<CertificatesScreen route={route} scope={ACCOUNT} />,
		options,
	);
}

type View = Awaited<ReturnType<typeof mount>>;

/** The screen on the area's own route, so links and tabs really switch. */
function Routed() {
	const { route, scope } = useDevicesRoute();
	return <CertificatesScreen route={route} scope={scope} />;
}

function certificateRow(container: HTMLElement, id: string): HTMLElement {
	const row = container.querySelector(`tr[data-certificate="${id}"]`);
	if (!row) throw new Error(`No certificate row ${id}`);
	return row as HTMLElement;
}

function silentRows(container: HTMLElement): HTMLElement[] {
	return Array.from(container.querySelectorAll<HTMLElement>("tr[data-silent]"));
}

function windowOf(container: HTMLElement, name: string): HTMLElement {
	const node = container.querySelector(`[data-window="${name}"]`);
	if (!node) throw new Error(`No summary window ${name}`);
	return node as HTMLElement;
}

function linkIn(root: HTMLElement, name: string): HTMLAnchorElement {
	return byRole("link", name, root) as HTMLAnchorElement;
}

/** Hub writes other than the workspace's own status subscriptions. */
function ourWrites(view: View) {
	return view.fake.api
		.writes()
		.filter(([, path]) => path.includes("certificate"));
}

describe("Certificates: the golden fleet", () => {
	test("says in one headline what expired, what expires and what it can't see, and sends nothing", async () => {
		const view = await mount();
		const { container } = view;
		expect(byRole("heading", "Certificates")).toBeTruthy();
		const headline = container.querySelector("[data-headline]");
		expect(headline?.textContent).toContain(
			"One certificate has expired and one expires in 5 days and can't renew itself.",
		);
		expect(headline?.textContent).toContain(
			"2 devices haven't reported certificates, and 1 is outside your access.",
		);
		expect(container.textContent).toContain(
			"3 certificates reported by 2 of 5 devices on",
		);
		const counts = ["expired", "week", "month", "errors", "silent", "noaccess"]
			.map((name) =>
				windowOf(container, name).firstElementChild?.textContent?.trim(),
			)
			.join(" ");
		expect(counts).toBe("1 1 1 1 2 1");
		// Nothing uses the expired certificate as far as a live read knows: warning, not critical.
		expect(windowOf(container, "expired").dataset.tone).toBe("warning");
		expect(
			container.querySelectorAll("[data-dv-primary]").length,
		).toBeLessThanOrEqual(1);
		expect(container.textContent).not.toMatch(MACHINE_WORDS);
		expect(ourWrites(view)).toEqual([]);
	});

	test("every block head states its source and age", async () => {
		const { container } = await mount();
		const block = container.querySelector("#certificates-expiry");
		const stamps = Array.from(
			block?.querySelectorAll("header [data-stamp]") ?? [],
		).map((stamp) => stamp.getAttribute("data-src"));
		expect(stamps).toEqual(["hub", "local"]);
		expect(block?.querySelector("header")?.textContent).toContain(
			"names from live reads",
		);
	});

	test("an expired certificate on an offline device: renewing is gated with the reason, and the click sends nothing", async () => {
		const view = await mount();
		const row = certificateRow(view.container, WAREHOUSE_CERT);
		expect(row.textContent).toContain("warehouse-pi");
		expect(row.textContent).toContain("82ac7195");
		expect(row.textContent).toContain("name shows after a live read");
		expect(row.textContent).toContain("Expired");
		expect(row.textContent).toContain(
			"Warning · which services use it is unknown",
		);
		expect(row.textContent).toContain("shows after a live read");
		const renew = byRole("button", "Renew", row);
		expect(renew.getAttribute("aria-disabled")).toBe("true");
		expect(row.textContent).toContain(
			"warehouse-pi is offline. Renewing needs a live connection.",
		);
		const calls = view.fake.api.calls.length;
		const navigations = view.navigations.length;
		await click(renew);
		expect(view.fake.api.calls).toHaveLength(calls);
		expect(view.navigations).toHaveLength(navigations);
		await click(byRole("button", "Diagnose connection…", row));
		expect(useOverlayStore.getState().overlay).toEqual({
			kind: "diagnose",
			deviceId: SAMPLE_IDS.warehouse,
		});
	});

	test("a certificate that can't renew itself links to its device's Certificates tab, focused", async () => {
		const view = await mount();
		const row = certificateRow(view.container, INTERNAL_MQTT);
		expect(row.textContent).toContain("internal-mqtt");
		expect(row.textContent).toContain("mqtt.lab.internal, 10.0.4.20");
		expect(row.textContent).toContain("Expires in 5 d");
		expect(row.textContent).toContain("can't renew itself");
		expect(row.textContent).toContain("Automatic (your authority)");
		expect(row.querySelector("[data-renewal=failing]")?.textContent).toContain(
			"Failing · its authority expired",
		);
		expect(row.textContent).toContain("Not used");
		const fix = linkIn(row, "Fix renewal");
		expect(fix.href).toContain(`device=${SAMPLE_IDS.edge}`);
		expect(fix.href).toContain("tab=certificates");
		expect(fix.href).toContain(`certificate=${INTERNAL_MQTT}`);
		await click(fix);
		expect(view.navigations.at(-1)?.href).toContain(
			`certificate=${INTERNAL_MQTT}`,
		);
	});

	test("a healthy certificate shows how it renews and which service uses it", async () => {
		const { container } = await mount();
		const row = certificateRow(container, EDGE_API);
		expect(row.textContent).toContain("Valid");
		expect(row.textContent).toContain("Automatic (Let's Encrypt)");
		expect(row.textContent).toContain("production · next try");
		const service = linkIn(row, "support-bot");
		expect(service.href).toContain("service=support-bot");
		expect(service.href).toContain("tab=endpoint");
		expect(linkIn(row, "Open").href).toContain(`certificate=${EDGE_API}`);
		expect(row.querySelector("[data-stamp]")?.textContent).toContain(
			"confirmed",
		);
	});

	test("devices without certificates say why: never reported, never checked in, no access", async () => {
		const { container } = await mount();
		const rows = Object.fromEntries(
			silentRows(container).map((row) => [
				row.querySelector("a")?.textContent ?? "",
				row,
			]),
		);
		expect(Object.keys(rows).sort()).toEqual([
			"cold-storage-nas",
			"lab-gpu-02",
			"studio-mac-mini",
		]);
		expect(rows["studio-mac-mini"]?.textContent).toContain(
			"The device hasn't reported certificates",
		);
		expect(rows["studio-mac-mini"]?.textContent).toContain("never reported");
		expect(rows["cold-storage-nas"]?.textContent).toContain(
			"Hasn't reported (never checked in)",
		);
		expect(rows["lab-gpu-02"]?.textContent).toContain(
			"No access · needs whole-device View status",
		);
		expect(rows["lab-gpu-02"]?.textContent).toContain(
			"Certificates belong to the whole device.",
		);
		expect(rows["lab-gpu-02"]?.dataset.silent).toBe("noaccess");
		// The app the access covers is named and linked to that app's Devices page.
		expect(
			linkIn(rows["lab-gpu-02"] as HTMLElement, "Invoice AI").href,
		).toContain("id=app_invoice_ai");
		// Never "empty": a device that can't be read is not a device without certificates.
		expect(container.textContent).not.toContain("No certificates listed 0");
	});

	test("a summary window filters the list to its devices, and All brings everything back", async () => {
		const { container } = await mount();
		await click(windowOf(container, "silent"));
		expect(container.querySelectorAll("tr[data-certificate]")).toHaveLength(0);
		expect(silentRows(container)).toHaveLength(2);
		expect(container.textContent).toContain("2 devices that haven't reported");
		expect(windowOf(container, "silent").getAttribute("aria-pressed")).toBe(
			"true",
		);
		await click(byRole("button", /^Expired\s*1$/, container));
		expect(container.querySelectorAll("tr[data-certificate]")).toHaveLength(1);
		expect(silentRows(container)).toHaveLength(0);
		expect(container.textContent).toContain("Showing 1 of 3");
		await click(byRole("button", "All", container));
		expect(container.querySelectorAll("tr[data-certificate]")).toHaveLength(3);
		expect(silentRows(container)).toHaveLength(3);
	});

	test("opening a certificate shows the ID reminders name and where each fact comes from", async () => {
		const { container } = await mount();
		const row = certificateRow(container, INTERNAL_MQTT);
		await click(byRole("button", "internal-mqtt: show details", row));
		const details = container.querySelector("tr[data-expand]");
		expect(details?.textContent).toContain("Certificate ID");
		expect(details?.textContent).toContain("reminders name this ID");
		expect(details?.textContent).toContain(
			"CN=Rheosoph Internal service issuer",
		);
		expect(details?.textContent).toContain("Each renewal lasts");
		expect(details?.textContent).toContain("30 days");
		expect(details?.textContent).toContain(
			"ID, fingerprint and expiry come from the hub: edge-berlin-01 reported them",
		);
		expect(
			linkIn(details as HTMLElement, "Open on edge-berlin-01").href,
		).toContain(`certificate=${INTERNAL_MQTT}`);
	});

	test("with nothing read live, certificates show IDs and the page offers to read the names", async () => {
		const { container } = await mount({ unlock: "none" });
		expect(container.textContent).toContain(
			"No device was read live yet, so certificates show IDs only.",
		);
		expect(
			container.querySelector("#certificates-expiry header")?.textContent,
		).toContain("no names read yet");
		const row = certificateRow(container, INTERNAL_MQTT);
		expect(row.textContent).toContain("93bcc1ef");
		expect(row.textContent).not.toContain("internal-mqtt");
		expect(row.textContent).toContain("renewal unknown until a live read");
		await click(byRole("button", "Read names from edge-berlin-01", container));
		expect(useOverlayStore.getState().overlay).toEqual({
			kind: "unlock",
			deviceId: SAMPLE_IDS.edge,
			connectLive: true,
		});
	});

	test("tabs replace the URL, and the create-authority link opens the wizard once", async () => {
		const view = await mount(
			{
				search: "view=certificates&tab=authorities&action=create-authority",
			},
			{ ...HOME, tab: "authorities", action: "create-authority" },
		);
		expect(view.container.textContent).toContain(
			"Create an organisation authority",
		);
		expect(view.navigations.at(-1)?.mode).toBe("replace");
		expect(view.navigations.at(-1)?.href).not.toContain("action=");
		await click(byRole("tab", /Reminders/, view.container));
		expect(view.navigations.at(-1)).toMatchObject({ mode: "replace" });
		expect(view.navigations.at(-1)?.href).toContain("tab=reminders");
	});

	test("a create-authority link without a tab lands on the authorities tab with the wizard open", async () => {
		const view = await mountDevices(<Routed />, {
			search: "view=certificates&action=create-authority",
		});
		await view.settle();
		expect(view.navigations.at(-1)?.mode).toBe("replace");
		expect(view.navigations.at(-1)?.href).toContain("tab=authorities");
		expect(view.navigations.at(-1)?.href).not.toContain("action=");
		expect(view.container.textContent).toContain(
			"What may this authority sign for?",
		);
	});
});

describe("Certificates: when the hub or a device is older or failing", () => {
	test("a failed refresh keeps the reports and says since when they are shown", async () => {
		const view = await mount();
		const { container, fake } = view;
		// A refusal to answer that isn't retried, so the refresh settles at once.
		fake.api.fail(
			{ path: /certificate-inventory/ },
			new ApiResponseError({
				status: 429,
				code: "RATE_LIMITED",
				message: "Too many requests",
			}),
		);
		await click(byRole("button", "Refresh", container));
		await view.settle();
		expect(container.querySelectorAll("tr[data-certificate]")).toHaveLength(3);
		expect(container.textContent).toContain("Couldn't refresh at");
		expect(
			container
				.querySelector("#certificates-expiry header [data-src=hub]")
				?.getAttribute("data-age"),
		).toBe("error");
	});

	test("the hub fails before anything was read: an error with a retry, never zeros or an empty list", async () => {
		const fake = await createFakeWorkspace();
		fake.api.fail(
			{ method: "GET", path: "devices/certificate-inventory" },
			new ApiResponseError({
				status: 429,
				code: "RATE_LIMITED",
				message: "Too many requests",
			}),
		);
		const view = await mount({ fake });
		const { container } = view;
		await view.settle();
		expect(container.querySelector("[data-kind=error]")?.textContent).toContain(
			"Couldn't read the certificate reports",
		);
		expect(byRole("button", "Try again", container)).toBeTruthy();
		expect(container.querySelector("[data-window]")).toBeNull();
		expect(container.querySelector("[data-headline]")).toBeNull();
		expect(container.querySelector("[data-kind=empty]")).toBeNull();
	});

	test("an older hub: each device's report is read on its own, nothing new is asked twice and no error shows", async () => {
		const fake = await createFakeWorkspace(undefined, { hubVersion: "old" });
		// As the hub answers a person whose access doesn't cover certificates.
		fake.api.fail(
			{
				method: "GET",
				path: `devices/${SAMPLE_IDS.lab}/certificate-inventory`,
			},
			new ApiResponseError({
				status: 403,
				code: "FORBIDDEN",
				message:
					"Certificates require device-wide Status or ManageCertificates access",
			}),
		);
		const view = await mount({ fake });
		const { container } = view;
		await view.settle();
		expect(container.querySelectorAll("tr[data-certificate]")).toHaveLength(3);
		expect(
			silentRows(container).find((row) => row.dataset.silent === "noaccess")
				?.textContent,
		).toContain("lab-gpu-02");
		expect(container.textContent).toContain(
			"This hub can't list every device's certificates at once",
		);
		expect(container.querySelector("[data-kind=error]")).toBeNull();
		expect(container.querySelector("[role=alert]")).toBeNull();
		expect(
			fake.api.sent("GET", "devices/certificate-inventory").length,
		).toBeLessThanOrEqual(1);
		for (const deviceId of [SAMPLE_IDS.edge, SAMPLE_IDS.warehouse])
			expect(
				fake.api.sent("GET", `devices/${deviceId}/certificate-inventory`),
			).toHaveLength(1);
	});

	test("an older agent: a failing renewal still reads as failing, and no command is sent", async () => {
		const view = await mount({ agentFeatures: {} });
		const { container, fake } = view;
		const commands = fake.api.commands.length;
		const row = certificateRow(container, INTERNAL_MQTT);
		expect(row.querySelector("[data-renewal=failing]")?.textContent).toContain(
			"Failing",
		);
		expect(container.querySelector("[role=alert]")).toBeNull();
		await click(byRole("button", "internal-mqtt: show details", row));
		expect(fake.api.commands).toHaveLength(commands);
		expect(queryByRole("alert", undefined, container)).toBeNull();
	});
});
