import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { ApiResponseError } from "../../../../lib/api-error";
import type { CertificateNotice } from "../../../../lib/device-management/hub/endpoints";
import { SAMPLE_IDS } from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import type { CertificatesRoute } from "../../../../lib/device-management/model/types";
import {
	byRole,
	click,
	inPortal,
	installDom,
	queryByRole,
	typeInto,
} from "../testing/dom-harness";
import type { FakeWorkspace } from "../testing/fake-workspace";
import type { MountDevicesOptions } from "../testing/mount-devices";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { CertificatesScreen } = await import("./certificates-screen");
const { CertificateReminderHistory } = await import("./reminder-history");

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

const EDGE = SAMPLE_IDS.edge;
const INTERNAL_MQTT = "93bcc1ef-5f49-4bb3-b90c-7b052822bf02";
const ACCOUNT = { kind: "account" } as const;
const REMINDERS: CertificatesRoute = {
	screen: "certificates",
	tab: "reminders",
};
const NOTICES = /certificate-notices(\?|$)/;
const MUTE = /certificate-notices\/mute/;
const TEST = /certificate-notices\/test$/;
/** R3: wire values, condition keys and gate codes never reach the screen. */
const MACHINE_WORDS =
	/[a-z]+_[a-z_]+|\bG\d{1,2}\b|\bD\d{1,2}\b|not_a_recipient|\busr_/;

function notice(patch: Partial<CertificateNotice>): CertificateNotice {
	return {
		certificate_id: INTERNAL_MQTT,
		certificate_revision: 2,
		not_after: 1_791_201_600,
		stage: "week",
		channel: "push",
		status: "sent",
		attempts: 1,
		completed_at: 1_790_596_800,
		next_attempt_at: null,
		...patch,
	};
}

async function seeded(
	options: Parameters<typeof createFakeWorkspace>[1] = {},
): Promise<FakeWorkspace> {
	const fake = await createFakeWorkspace(undefined, options);
	fake.hub.notices.set(EDGE, [
		notice({ channel: "email", completed_at: 1_790_596_860 }),
		notice({}),
		notice({
			stage: "three_days",
			status: "pending",
			completed_at: null,
			next_attempt_at: 1_790_942_400,
			attempts: 0,
		}),
	]);
	return fake;
}

function mountHistory(
	fake: FakeWorkspace,
	props: { certificateId?: string; compact?: boolean } = {},
) {
	return mountDevices(
		<CertificateReminderHistory deviceId={EDGE} {...props} />,
		{ fake },
	);
}

function noticeRows(container: HTMLElement): HTMLElement[] {
	return Array.from(container.querySelectorAll<HTMLElement>("tr[data-notice]"));
}

describe("Reminder history of one device (BG26)", () => {
	test("lists what the hub sent, by stage and channel, and links each certificate to its device tab", async () => {
		const fake = await seeded();
		const { container } = await mountHistory(fake);
		const rows = noticeRows(container);
		expect(rows).toHaveLength(3);
		expect(rows[0]?.textContent).toContain("internal-mqtt");
		expect(rows[0]?.textContent).toContain("7 days before");
		expect(rows[0]?.textContent).toContain("Email");
		expect(rows[0]?.textContent).toContain("Sent");
		expect(rows[0]?.textContent).toContain("1 attempt");
		expect(rows[2]?.textContent).toContain("3 days before");
		expect(rows[2]?.textContent).toContain("Scheduled");
		expect(rows[2]?.textContent).toContain("next try");
		const link = byRole("link", "internal-mqtt", rows[0]) as HTMLAnchorElement;
		expect(link.href).toContain(`device=${EDGE}`);
		expect(link.href).toContain("tab=certificates");
		expect(link.href).toContain(`certificate=${INTERNAL_MQTT}`);
		expect(container.textContent).not.toMatch(/three_days|pending|week\b/);
		expect(fake.api.writes().filter(([, path]) => MUTE.test(path))).toEqual([]);
	});

	test("with no reminder yet it says so and when the first one goes out, never an error", async () => {
		const fake = await createFakeWorkspace();
		const { container } = await mountHistory(fake);
		expect(container.querySelector("[data-kind=empty]")?.textContent).toContain(
			"No reminders sent to you for edge-berlin-01 yet",
		);
		expect(container.textContent).toContain(
			"The first one goes out 7 days before a certificate expires.",
		);
		expect(queryByRole("alert", undefined, container)).toBeNull();
	});

	test("muting goes through a confirmation with an undo row, and cancelling sends nothing", async () => {
		const fake = await seeded();
		const view = await mountHistory(fake);
		const { container } = view;
		await click(byRole("button", "Mute device…", container));
		let confirm = inPortal("alertdialog");
		expect(confirm.textContent).toContain(
			"You stop getting expiry reminders for every certificate on edge-berlin-01",
		);
		expect(confirm.textContent).toContain("Only you.");
		expect(confirm.textContent).toContain("Unmute here at any time.");
		await click(byRole("button", "Cancel", confirm));
		await view.settle();
		expect(fake.api.sent("PUT", MUTE)).toEqual([]);

		await click(byRole("button", "Mute device…", container));
		confirm = inPortal("alertdialog");
		await click(byRole("button", "Mute reminders for edge-berlin-01", confirm));
		await view.settle();
		const [put] = fake.api.sent("PUT", MUTE);
		expect(put?.[2]).toEqual({ certificate_id: null, until: null });
		expect(container.textContent).toContain("Muted until you unmute");
		expect(container.textContent).toContain(
			"You won't be reminded about edge-berlin-01 until you unmute it.",
		);

		await click(byRole("button", "Unmute device", container));
		await view.settle();
		const [removed] = fake.api.sent("DELETE", MUTE);
		expect(removed?.[1]).toContain("certificate=*");
		expect(queryByRole("button", "Unmute device", container)).toBeNull();
		expect(byRole("button", "Mute device…", container)).toBeTruthy();
	});

	test("a test reminder says which channels it went out on, and the hub's ten-minute wait reads as a sentence", async () => {
		const fake = await seeded();
		const view = await mountHistory(fake);
		const { container } = view;
		await click(byRole("button", "Send test", container));
		await view.settle();
		const [post] = fake.api.sent("POST", TEST);
		expect(post?.[2]).toEqual({ channel: "both" });
		expect(container.textContent).toContain(
			"Test reminder sent by Push and Email at",
		);
		expect(
			noticeRows(container).filter((row) => row.dataset.notice === "test"),
		).toHaveLength(2);
		expect(container.textContent).toContain("No certificate");

		await click(byRole("button", "Send test", container));
		await view.settle();
		expect(fake.api.sent("POST", TEST)).toHaveLength(2);
		expect(container.textContent).toContain(
			"The hub sends one test reminder per device every 10 minutes. Try again in 10 minutes.",
		);
		expect(container.textContent).not.toMatch(/429|TOO_MANY|rate_limited/);
	});

	test("a wait the hub doesn't state still reads as a sentence", async () => {
		const fake = await seeded();
		fake.api.fail(
			{ method: "POST", path: TEST },
			new ApiResponseError({
				status: 429,
				code: "TOO_MANY_REQUESTS",
				message: "A test reminder was sent less than ten minutes ago",
			}),
		);
		const view = await mountHistory(fake);
		await click(byRole("button", "Send test", view.container));
		await view.settle();
		expect(view.container.textContent).toContain(
			"The hub sends one test reminder per device every 10 minutes. Try again a little later.",
		);
	});

	test("a channel the hub skipped is named with its reason", async () => {
		const fake = await seeded();
		fake.hub.noticeChannels.email = false;
		const view = await mountHistory(fake);
		await click(byRole("button", "Send test", view.container));
		await view.settle();
		expect(view.container.textContent).toContain(
			"Test reminder sent by Push at",
		);
		expect(view.container.textContent).toContain(
			"Not sent: Email: this hub doesn't send it.",
		);
		fake.api.on("POST", "devices/:id/certificate-notices/test", () => ({
			sent: [],
			skipped: [
				{ channel: "push", reason: "not_a_recipient" },
				{ channel: "email", reason: "no_email" },
			],
		}));
		await click(byRole("button", "Send test", view.container));
		await view.settle();
		expect(view.container.textContent).toContain(
			"No test reminder was sent. Push: you haven't added this device's keys, so its reminders don't reach you. Email: your account has no email address.",
		);
	});

	test("a hub that lists reminders but has no mute or test route says nothing changed, never 'muted' or 'sent'", async () => {
		const fake = await seeded();
		const missing = new ApiResponseError({
			status: 405,
			code: "METHOD_NOT_ALLOWED",
			message: "Method not allowed",
		});
		fake.api.fail({ method: "PUT", path: MUTE }, missing);
		fake.api.fail({ method: "POST", path: TEST }, missing);
		const view = await mountHistory(fake);
		const { container } = view;
		await click(byRole("button", "Mute device…", container));
		const confirm = inPortal("alertdialog");
		// The confirmation carries the bell icon in its head, like the other sheets.
		expect(confirm.querySelector("svg.lucide-bell-off")).toBeTruthy();
		await click(byRole("button", "Mute reminders for edge-berlin-01", confirm));
		await view.settle();
		expect(fake.api.sent("PUT", MUTE)).toHaveLength(1);
		expect(container.textContent).toContain(
			"This hub can't do that yet. Nothing changed.",
		);
		expect(container.textContent).not.toContain("Muted at");
		expect(container.textContent).not.toContain("Muted until you unmute");
		expect(byRole("button", "Mute device…", container)).toBeTruthy();

		await click(byRole("button", "Send test", container));
		await view.settle();
		expect(fake.api.sent("POST", TEST)).toHaveLength(1);
		expect(container.textContent).toContain(
			"This hub can't do that yet. Nothing changed.",
		);
		expect(container.textContent).not.toContain("Test reminder sent");
		expect(container.textContent).not.toMatch(/405|METHOD_NOT_ALLOWED/);
	});

	test("for one certificate it asks for that certificate's reminders and mutes only it", async () => {
		const fake = await seeded();
		const view = await mountHistory(fake, {
			certificateId: INTERNAL_MQTT,
			compact: true,
		});
		const { container } = view;
		const [get] = fake.api.sent("GET", NOTICES);
		expect(get?.[1]).toContain(`certificate=${INTERNAL_MQTT}`);
		expect(noticeRows(container)).toHaveLength(3);
		expect(queryByRole("columnheader", "Certificate", container)).toBeNull();
		await click(byRole("button", "Mute this certificate…", container));
		await click(
			byRole(
				"button",
				"Mute reminders for this certificate",
				inPortal("alertdialog"),
			),
		);
		await view.settle();
		const [put] = fake.api.sent("PUT", MUTE);
		expect(put?.[2]).toEqual({ certificate_id: INTERNAL_MQTT, until: null });
		expect(byRole("button", "Unmute this certificate", container)).toBeTruthy();
	});

	test("an older hub: a fixed explanation instead of history, mute and test; each route is asked at most once and no error shows", async () => {
		const fake = await seeded({ hubVersion: "old" });
		const view = await mountHistory(fake);
		const { container } = view;
		await view.settle();
		expect(
			container.querySelector("[data-reminders=interim]")?.textContent,
		).toContain(
			"This hub can't list the reminders it already sent, mute a device or send a test reminder yet.",
		);
		expect(queryByRole("button", "Mute device…", container)).toBeNull();
		expect(queryByRole("button", "Send test", container)).toBeNull();
		expect(queryByRole("alert", undefined, container)).toBeNull();
		expect(container.querySelector("[data-kind=error]")).toBeNull();
		expect(fake.api.sent("GET", NOTICES).length).toBeLessThanOrEqual(1);
		expect(fake.api.sent("GET", MUTE)).toEqual([]);
		expect(fake.api.writes().filter(([, path]) => MUTE.test(path))).toEqual([]);
	});

	test("a person whose access doesn't cover certificates is told so, not shown an empty list", async () => {
		const fake = await seeded();
		fake.api.fail(
			{ method: "GET", path: NOTICES },
			new ApiResponseError({
				status: 403,
				code: "FORBIDDEN",
				message:
					"Certificates require device-wide Status or ManageCertificates access",
			}),
		);
		const { container } = await mountHistory(fake);
		expect(
			container.querySelector("[data-kind=noaccess]")?.textContent,
		).toContain("Reminders for edge-berlin-01 aren't yours to see");
		expect(container.querySelector("[data-kind=empty]")).toBeNull();
		expect(container.textContent).not.toContain("ManageCertificates");
	});
});

describe("Reminders tab", () => {
	function mount(options: MountDevicesOptions = {}) {
		return mountDevices(
			<CertificatesScreen route={REMINDERS} scope={ACCOUNT} />,
			{ search: "view=certificates&tab=reminders", ...options },
		);
	}

	test("explains the stages, who receives them, and what comes next for each certificate", async () => {
		const view = await mount();
		const { container } = view;
		const stages = byRole("list", "Reminder stages", container);
		expect(stages.textContent).toContain("7 days before");
		expect(stages.textContent).toContain("On expiry");
		expect(container.textContent).toContain("Who receives them");
		expect(container.textContent).toContain("Recipients by device");
		expect(container.textContent).toContain(
			"You don't receive them: your access covers part of the device only.",
		);
		expect(container.textContent).toContain("Coming up");
		expect(container.textContent).toContain("3 days before");
		expect(container.textContent).toContain(
			"every stage has passed; it expired",
		);
		expect(container.textContent).toContain("worked out from expiry dates");
		expect(container.textContent).toContain("Sent reminders");
		expect(
			container.querySelectorAll("[data-dv-primary]").length,
		).toBeLessThanOrEqual(1);
		expect(container.textContent).not.toMatch(MACHINE_WORDS);
	});

	test("recipients: an end today reads as a time, and rules are read only for devices the hub says are shared", async () => {
		const view = await mount();
		await view.settle();
		const { container, fake } = view;
		expect(container.textContent).toMatch(
			/whole-device access · until \d{1,2}:\d{2}/,
		);
		expect(container.textContent).not.toContain(
			"Unlock to see who else receives them",
		);
		const asked = fake.api
			.sent("GET", /management\/policy$/)
			.map(([, path]) => path);
		expect(asked.some((path) => path.includes(SAMPLE_IDS.edge))).toBe(true);
		// cold-storage-nas is locked and shared with nobody: nothing reads its rules.
		expect(asked.some((path) => path.includes(SAMPLE_IDS.cold))).toBe(false);
	});

	test("finds the certificate a reminder names by its ID", async () => {
		const view = await mount();
		const { container } = view;
		const input = byRole(
			"textbox",
			"Find a certificate from a reminder",
			container,
		);
		await typeInto(input, "93bc");
		await click(byRole("button", "Look up", container));
		expect(container.textContent).toContain(
			"Paste at least the first 8 characters of the certificate ID from the reminder.",
		);
		await typeInto(input, "93bcc1ef-5f49");
		await click(byRole("button", "Look up", container));
		const open = byRole("link", "Open it", container) as HTMLAnchorElement;
		expect(open.href).toContain(`certificate=${INTERNAL_MQTT}`);
		await typeInto(input, "00000000-0000-0000");
		await click(byRole("button", "Look up", container));
		expect(container.textContent).toContain(
			"No certificate with that ID on the devices you can see. It may be on lab-gpu-02",
		);
	});

	test("an older hub: the tab keeps explaining reminders and offers no mute or test", async () => {
		const view = await mount({ hubVersion: "old" });
		const { container } = view;
		await view.settle();
		expect(container.textContent).toContain("How expiry reminders work");
		expect(container.textContent).toContain("Muting and test reminders");
		expect(container.textContent).toContain("not available on this hub");
		expect(queryByRole("button", "Send test", container)).toBeNull();
		expect(queryByRole("alert", undefined, container)).toBeNull();
	});
});
