import { afterAll, afterEach, describe, expect, test } from "bun:test";
import type { AcmeCertificate } from "../../../../../lib/device-management/certificate-acme";
import type {
	CertificateIssuer,
	CertificateRequest,
} from "../../../../../lib/device-management/certificate-issuance";
import type { DeviceCertificate } from "../../../../../lib/device-management/certificates";
import {
	SAMPLE_IDS,
	SAMPLE_NOW,
	sampleFleet,
} from "../../../../../lib/device-management/model/__fixtures__/sample-fleet";
import type { DeviceRoute } from "../../../../../lib/device-management/model/types";
import {
	allByRole,
	byRole,
	click,
	dropFiles,
	inPortal,
	installDom,
	queryByRole,
	typeInto,
} from "../../testing/dom-harness";
import type { MountDevicesOptions } from "../../testing/mount-devices";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../../testing/mount-devices"
);
await preloadDevices();
const { createFakeWorkspace } = await import("../../testing/fake-workspace");
const { DeviceCertificatesTab } = await import("../../device/certificates-tab");

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

const EDGE = SAMPLE_IDS.edge;
const ACCOUNT = { kind: "account" } as const;
const EDGE_API = "24f6fe22-c6c2-4e15-9d37-7a41a379afb9";
const INTERNAL_MQTT = "93bcc1ef-5f49-4bb3-b90c-7b052822bf02";
const BILLING_REQUEST = "966dddf3-b745-483e-8283-0f33254a480d";
const DAY = 86_400;

const CHAIN = "-----BEGIN CERTIFICATE-----\nYWJj\n-----END CERTIFICATE-----\n";
const SECRET = "never-display-this-secret";
const KEY = `-----BEGIN PRIVATE KEY-----\n${SECRET}\n-----END PRIVATE KEY-----\n`;
const REQUEST_PEM =
	"-----BEGIN CERTIFICATE REQUEST-----\nYWJj\n-----END CERTIFICATE REQUEST-----\n";

type Command = Record<string, unknown>;

/** The fields the tests' device handlers read from the commands they answer. */
interface Wire {
	request_id: string;
	certificate_id: string;
	label?: string;
	expected_revision: number;
	dns_names: string[];
	ip_addresses: string[];
	leaf_lifetime_days: number;
	environment: AcmeCertificate["environment"];
	http_bind: string;
}

const wire = (command: Command) => command as unknown as Wire;

async function mount(options: MountDevicesOptions = {}) {
	const route: DeviceRoute = {
		screen: "device",
		deviceId: EDGE,
		tab: "certificates",
	};
	const view = await mountDevices(
		<DeviceCertificatesTab route={route} scope={ACCOUNT} deviceId={EDGE} />,
		options,
	);
	await view.settle();
	await view.settle();
	return view;
}

type View = Awaited<ReturnType<typeof mount>>;

const text = (root: ParentNode = document.body) =>
	(root.textContent ?? "").replace(/\s+/g, " ");

const pem = (name: string, content: string) =>
	new File([content], name, { type: "application/x-pem-file" });

const sent = (view: View, type: string): Command[] =>
	view.fake.api.commands
		.filter(([device, kind]) => device === EDGE && kind === type)
		.map(([, , command]) => command);

const writes = (view: View) =>
	view.fake.api.commands.filter(
		([device, kind]) =>
			device === EDGE && /^(put|delete|create|install|configure)_/.test(kind),
	);

const row = (view: View, certificateId: string) => {
	const found = view.container.querySelector<HTMLElement>(
		`[data-certificate="${certificateId}"]`,
	);
	if (!found) throw new Error(`No certificate row ${certificateId}`);
	return found;
};

const block = (view: View, id: string) => {
	const found = view.container.querySelector<HTMLElement>(`#${id}`);
	if (!found) throw new Error(`No block "${id}"`);
	return found;
};

async function settled(view: View) {
	await view.settle();
	await view.settle();
}

const SET_UP_LETS_ENCRYPT = "Set up Let's Encrypt…";
const LIFETIME_FIELD = "Each renewed certificate is valid for";

async function chooseFromRowMenu(
	view: View,
	id: string,
	item: string | RegExp,
) {
	await click(byRole("button", /^More for /, row(view, id)));
	await click(byRole("menuitem", item, inPortal("menu")));
	await view.settle();
}

async function chooseFromAddMenu(view: View, item: string | RegExp) {
	await click(byRole("button", "Add certificate", view.container));
	await click(byRole("menuitem", item, inPortal("menu")));
	await view.settle();
}

const dropTarget = (sheet: HTMLElement) => {
	const target = sheet.querySelector('input[type="file"]')?.closest("label");
	if (!target) throw new Error("The sheet has no file drop zone");
	return target;
};

const disabled = (button: HTMLElement) =>
	button.getAttribute("aria-disabled") === "true";

/** The one-line reason beside a sheet's disabled primary (R7). */
const footReason = (sheet: HTMLElement) =>
	sheet.querySelector("[data-foot-reason]")?.textContent ?? undefined;

function certificateOf(command: Command, revision: number): DeviceCertificate {
	const input = wire(command);
	return {
		certificate_id: input.certificate_id,
		label: input.label ?? "billing-api",
		revision,
		subject: "CN=new.lab.internal",
		issuer: "CN=Test issuer",
		dns_names: ["new.lab.internal"],
		ip_addresses: [],
		sha256_fingerprint: "e".repeat(64),
		not_before: SAMPLE_NOW - 60,
		not_after: SAMPLE_NOW + 90 * DAY,
		bindings: [],
		binding_count: 0,
	};
}

/** The device accepts imports and keeps what it was sent, key included (the command log redacts it). */
function acceptImports(view: View): Command[] {
	const puts: Command[] = [];
	const agent = view.fake.api.agent(EDGE);
	agent.handle("put_certificate", (command) => {
		puts.push(command);
		const revision = wire(command).expected_revision + 1;
		const certificate = certificateOf(command, revision);
		agent.certificates = {
			inventory_revision: agent.certificates.inventory_revision + 1,
			certificates: [
				...agent.certificates.certificates.filter(
					(entry) => entry.certificate_id !== certificate.certificate_id,
				),
				certificate,
			],
		};
		return { state: "completed", result: { certificate } };
	});
	return puts;
}

function acceptInstalls(view: View): Command[] {
	const installs: Command[] = [];
	const agent = view.fake.api.agent(EDGE);
	agent.handle("install_certificate_request", (command) => {
		installs.push(command);
		const request = agent.certificateRequests.find(
			(entry) => entry.request_id === command.request_id,
		);
		if (!request) throw new Error("unknown request");
		agent.certificateRequests = agent.certificateRequests.filter(
			(entry) => entry !== request,
		);
		const certificate = certificateOf(request, request.expected_revision + 1);
		agent.certificates = {
			inventory_revision: agent.certificates.inventory_revision + 1,
			certificates: [...agent.certificates.certificates, certificate],
		};
		return { state: "completed", result: { certificate } };
	});
	return installs;
}

/** The request the device answers a create command with. */
function requestOf(
	command: Command,
	purpose: CertificateRequest["purpose"],
): CertificateRequest {
	const input = wire(command);
	const lifetime =
		purpose === "issuer"
			? { leaf_lifetime_days: input.leaf_lifetime_days }
			: {};
	return {
		request_id: input.request_id,
		certificate_id: input.certificate_id,
		label: input.label ?? "internal-mqtt",
		expected_revision: input.expected_revision,
		dns_names: input.dns_names,
		ip_addresses: input.ip_addresses,
		csr_pem: REQUEST_PEM,
		created_at: SAMPLE_NOW,
		expires_at: SAMPLE_NOW + 30 * DAY,
		purpose,
		...lifetime,
	};
}

/** The device makes signing requests (`create_certificate_request`) or renewal authority requests. */
function acceptRequests(
	view: View,
	purpose: CertificateRequest["purpose"],
): Command[] {
	const created: Command[] = [];
	const agent = view.fake.api.agent(EDGE);
	const type =
		purpose === "issuer"
			? "create_certificate_issuer_request"
			: "create_certificate_request";
	agent.handle(type, (command) => {
		created.push(command);
		const request = requestOf(command, purpose);
		agent.certificateRequests = [...agent.certificateRequests, request];
		return { state: "completed", result: { request } };
	});
	return created;
}

function issuerOf(lifetimeDays: number): CertificateIssuer {
	return {
		certificate_id: INTERNAL_MQTT,
		revision: 2,
		dns_names: ["mqtt.lab.internal"],
		ip_addresses: ["10.0.4.20"],
		leaf_lifetime_days: lifetimeDays,
		not_after: SAMPLE_NOW + 180 * DAY,
		last_renewed_at: null,
		next_renewal_at: SAMPLE_NOW + 3_600,
		last_error: null,
	};
}

/** The device installs a signed renewal authority and renews with it from then on. */
function acceptAuthorities(view: View, lifetimeDays: number): Command[] {
	const installed: Command[] = [];
	const agent = view.fake.api.agent(EDGE);
	agent.handle("install_certificate_issuer", (command) => {
		installed.push(command);
		const issuer = issuerOf(lifetimeDays);
		agent.certificateIssuers = [issuer];
		agent.certificateRequests = agent.certificateRequests.filter(
			(entry) => entry.request_id !== command.request_id,
		);
		return { state: "completed", result: { issuer } };
	});
	return installed;
}

function acceptSetups(view: View): Command[] {
	const configured: Command[] = [];
	const agent = view.fake.api.agent(EDGE);
	agent.handle("configure_acme_certificate", (command) => {
		configured.push(command);
		const input = wire(command);
		const acme: AcmeCertificate = {
			certificate_id: input.certificate_id,
			label: input.label ?? "",
			revision: input.expected_revision + 1,
			dns_names: input.dns_names,
			environment: input.environment,
			http_bind: input.http_bind,
			next_attempt_at: SAMPLE_NOW,
			last_renewed_at: null,
			last_error: null,
		};
		agent.acme = [
			...agent.acme.filter(
				(entry) => entry.certificate_id !== acme.certificate_id,
			),
			acme,
		];
		return { state: "completed", result: { acme } };
	});
	return configured;
}

const terms = (sheet: HTMLElement) =>
	byRole("checkbox", /subscriber agreement/, sheet);

async function openSign(view: View) {
	await click(
		byRole(
			"button",
			"Sign with organisation authority…",
			block(view, "device-certificate-requests"),
		),
	);
	return byRole("dialog");
}

function passwordField(sheet: HTMLElement) {
	const input = sheet.querySelector<HTMLInputElement>('input[type="password"]');
	if (!input) throw new Error("The sheet has no password field");
	return input;
}

describe("import and replace", () => {
	test("replacing keeps the certificate's ID and version, sends the key once and never shows it", async () => {
		const view = await mount();
		const puts = acceptImports(view);
		await chooseFromRowMenu(view, INTERNAL_MQTT, /Replace with file/);
		const sheet = byRole("dialog");
		expect(text(sheet)).toContain("Replace internal-mqtt with a file");
		expect(text(sheet)).toContain(
			"Replacing this certificate stops its automatic renewal.",
		);
		await dropFiles(dropTarget(sheet), [
			pem("fullchain.pem", CHAIN),
			pem("key.pem", KEY),
		]);
		expect(text(sheet)).toContain("fullchain.pem");
		expect(text(sheet)).toContain("key.pem");
		const submit = byRole("button", "Replace certificate", sheet);
		expect(disabled(submit)).toBe(true);
		expect(footReason(sheet)).toBe("Tick the confirmation above first.");
		await click(submit);
		expect(puts).toHaveLength(0);
		await click(
			byRole("checkbox", /Automatic renewal of internal-mqtt stops/, sheet),
		);
		await click(byRole("button", "Replace certificate", sheet));
		await settled(view);
		expect(puts).toHaveLength(1);
		expect(puts[0]).toMatchObject({
			certificate_id: INTERNAL_MQTT,
			expected_revision: 2,
			label: "internal-mqtt",
		});
		expect(`${puts[0]?.private_key_pem}`).toContain(SECRET);
		expect(text()).not.toContain(SECRET);
		expect(queryByRole("dialog")).toBeNull();
		expect(text(row(view, INTERNAL_MQTT))).toContain(
			"Replace internal-mqtt: done.",
		);
	});

	test("a new certificate gets a fresh ID and starts at version 0", async () => {
		const view = await mount();
		const puts = acceptImports(view);
		await chooseFromAddMenu(view, /Import from files/);
		const sheet = byRole("dialog");
		expect(text(sheet)).toContain(
			"Uses 1 certificate slot: 4 of 32 after this.",
		);
		await dropFiles(dropTarget(sheet), [
			pem("api.crt", CHAIN),
			pem("api.key", KEY),
		]);
		await click(byRole("button", "Import certificate", sheet));
		expect(text(sheet)).toContain("Give it a label.");
		expect(puts).toHaveLength(0);
		await typeInto(byRole("textbox", "Label", sheet), "orders-api");
		await click(byRole("button", "Import certificate", sheet));
		await settled(view);
		expect(puts).toHaveLength(1);
		expect(puts[0]).toMatchObject({
			label: "orders-api",
			expected_revision: 0,
		});
		expect(puts[0]?.certificate_id).toMatch(/^[0-9a-f-]{36}$/);
		expect([EDGE_API, INTERNAL_MQTT]).not.toContain(puts[0]?.certificate_id);
		expect(text(block(view, "device-certificates"))).toContain(
			"Import certificate orders-api: done.",
		);
		expect(text(block(view, "device-certificates"))).toContain("orders-api");
	});

	test("a refused import quotes the device, never shows the key and keeps the chosen files", async () => {
		const view = await mount();
		view.fake.api
			.agent(EDGE)
			.reject(
				"put_certificate",
				"invalid",
				"Private key does not match the certificate",
			);
		await chooseFromRowMenu(view, INTERNAL_MQTT, /Replace with file/);
		const sheet = byRole("dialog");
		await dropFiles(dropTarget(sheet), [
			pem("fullchain.pem", CHAIN),
			pem("key.pem", KEY),
		]);
		await click(
			byRole("checkbox", /Automatic renewal of internal-mqtt stops/, sheet),
		);
		await click(byRole("button", "Replace certificate", sheet));
		await settled(view);
		const open = byRole("dialog");
		expect(text(open)).toContain("was refused by the device");
		expect(text(open)).toContain("Private key does not match the certificate");
		expect(text(open)).toContain("fullchain.pem");
		expect(text(open)).toContain("key.pem");
		expect(text()).not.toContain(SECRET);
	});

	test("files that can't be a chain and a key are named as such, and nothing is sent", async () => {
		const view = await mount();
		const puts = acceptImports(view);
		await chooseFromAddMenu(view, /Import from files/);
		const sheet = byRole("dialog");
		await typeInto(byRole("textbox", "Label", sheet), "orders-api");
		await click(byRole("button", "Import certificate", sheet));
		expect(text(sheet)).toContain(
			"Choose both the certificate chain and its private key.",
		);
		await dropFiles(dropTarget(sheet), [pem("notes.txt", "hello")]);
		expect(text(sheet)).toContain(
			"neither a PEM certificate chain nor a PEM private key",
		);
		await dropFiles(dropTarget(sheet), [
			pem(
				"locked.key",
				"-----BEGIN ENCRYPTED PRIVATE KEY-----\nabc\n-----END ENCRYPTED PRIVATE KEY-----\n",
			),
		]);
		expect(text(sheet)).toContain("This private key is encrypted.");
		await dropFiles(dropTarget(sheet), [
			pem("big.crt", `${CHAIN}${"A".repeat(13 * 1024)}`),
			pem("api.key", KEY),
		]);
		await click(byRole("button", "Import certificate", sheet));
		expect(text(sheet)).toContain("larger than 12 KiB together");
		expect(puts).toHaveLength(0);
		expect(writes(view)).toHaveLength(0);
	});

	test("leaving while the key file is still being read sends nothing", async () => {
		const view = await mount();
		const puts = acceptImports(view);
		await chooseFromAddMenu(view, /Import from files/);
		const sheet = byRole("dialog");
		await typeInto(byRole("textbox", "Label", sheet), "orders-api");
		let finish: (value: string) => void = () => undefined;
		const slow = pem("api.key", KEY);
		Object.defineProperty(slow, "text", {
			value: () =>
				new Promise<string>((resolve) => {
					finish = resolve;
				}),
		});
		await dropFiles(dropTarget(sheet), [pem("api.crt", CHAIN), slow]);
		await click(byRole("button", "Import certificate", sheet));
		await view.unmount();
		finish(KEY);
		await new Promise((resolve) => setTimeout(resolve, 0));
		expect(puts).toHaveLength(0);
	});
});

describe("signing requests", () => {
	test("a request shows its exact names; installing takes a signed chain only, never a key", async () => {
		const view = await mount();
		const installs = acceptInstalls(view);
		const requests = block(view, "device-certificate-requests");
		expect(text(requests)).toContain("billing-api");
		expect(text(requests)).toContain("Certificate request");
		expect(text(requests)).toContain("new certificate");
		expect(text(requests)).toContain("billing.lab.internal");
		expect(text(requests)).toContain("Waiting for signed certificate");
		expect(
			disabled(byRole("button", "Download signing request", requests)),
		).toBe(false);
		await click(byRole("button", "Install signed certificate…", requests));
		const sheet = byRole("dialog");
		expect(text(sheet)).toContain("Exact namesbilling.lab.internal");
		const chain = byRole("textbox", "Signed chain (PEM)", sheet);
		await typeInto(chain, `${CHAIN}${KEY}`);
		await click(byRole("button", "Install certificate", sheet));
		expect(text(sheet)).toContain("This contains a private key.");
		await typeInto(chain, "not a certificate");
		await click(byRole("button", "Install certificate", sheet));
		expect(text(sheet)).toContain("This isn't a PEM certificate chain.");
		expect(installs).toHaveLength(0);
		await typeInto(chain, CHAIN);
		await click(byRole("button", "Install certificate", sheet));
		await settled(view);
		expect(installs).toHaveLength(1);
		expect(installs[0]).toEqual({
			type: "install_certificate_request",
			request_id: BILLING_REQUEST,
			certificate_chain_pem: CHAIN,
		});
		expect(queryByRole("dialog")).toBeNull();
		expect(text(view.container)).toContain(
			"Install the signed certificate for billing-api: done.",
		);
		expect(text(block(view, "device-certificates"))).toContain("billing-api");
	});

	test("a renewal authority is installed only after the names and the lifetime were approved", async () => {
		const seed = sampleFleet();
		const request = seed.live[EDGE]?.certificateRequests?.[0];
		if (!request) throw new Error("the sample fleet has no signing request");
		Object.assign(request, {
			purpose: "issuer",
			certificate_id: INTERNAL_MQTT,
			expected_revision: 2,
			label: "internal-mqtt",
			dns_names: ["mqtt.lab.internal"],
			ip_addresses: ["10.0.4.20"],
			leaf_lifetime_days: 30,
		});
		const view = await mount({ seed });
		const installs = acceptAuthorities(view, 30);
		const requests = block(view, "device-certificate-requests");
		expect(text(requests)).toContain("Renewal authority request");
		expect(text(requests)).toContain("replaces internal-mqtt (version 2)");
		await click(byRole("button", "Install signed authority…", requests));
		const sheet = byRole("dialog");
		expect(text(sheet)).toContain("mqtt.lab.internal, 10.0.4.20");
		expect(text(sheet)).toContain("valid for at most 30 days each");
		await typeInto(byRole("textbox", "Signed chain (PEM)", sheet), CHAIN);
		const submit = byRole("button", "Install renewal authority", sheet);
		expect(disabled(submit)).toBe(true);
		// The reason sits beside the button while the approval isn't ticked.
		expect(footReason(sheet)).toBe("Tick the confirmation above first.");
		await click(submit);
		expect(installs).toHaveLength(0);
		await click(byRole("checkbox", /I authorise this device/, sheet));
		expect(footReason(sheet)).toBeUndefined();
		await click(byRole("button", "Install renewal authority", sheet));
		await settled(view);
		expect(installs).toHaveLength(1);
		expect(installs[0]).toMatchObject({
			request_id: BILLING_REQUEST,
			certificate_chain_pem: CHAIN,
		});
	});

	test("leaving while a chosen chain file is read installs nothing", async () => {
		const view = await mount();
		const installs = acceptInstalls(view);
		await click(
			byRole(
				"button",
				"Install signed certificate…",
				block(view, "device-certificate-requests"),
			),
		);
		const sheet = byRole("dialog");
		let finish: (value: string) => void = () => undefined;
		const slow = pem("signed.pem", CHAIN);
		Object.defineProperty(slow, "text", {
			value: () =>
				new Promise<string>((resolve) => {
					finish = resolve;
				}),
		});
		await dropFiles(dropTarget(sheet), [slow]);
		await view.unmount();
		finish(CHAIN);
		await new Promise((resolve) => setTimeout(resolve, 0));
		expect(installs).toHaveLength(0);
	});

	test("creating a request carries exactly the names typed", async () => {
		const view = await mount();
		const created = acceptRequests(view, "service");
		await chooseFromAddMenu(view, /Create on the device/);
		const sheet = byRole("dialog");
		await click(byRole("button", "Create signing request", sheet));
		expect(text(sheet)).toContain("Give it a label.");
		expect(text(sheet)).toContain("Add at least one DNS name or IP address.");
		expect(created).toHaveLength(0);
		await typeInto(byRole("textbox", "Label", sheet), "orders-api");
		await typeInto(
			byRole("textbox", "Names", sheet),
			"orders.lab.internal\n10.0.4.21",
		);
		await click(byRole("button", "Create signing request", sheet));
		await settled(view);
		expect(created).toHaveLength(1);
		expect(created[0]).toMatchObject({
			label: "orders-api",
			expected_revision: 0,
			dns_names: ["orders.lab.internal"],
			ip_addresses: ["10.0.4.21"],
		});
		expect(text(block(view, "device-certificate-requests"))).toContain(
			"orders.lab.internal, 10.0.4.21",
		);
	});

	test("a second request for a certificate whose request still waits is blocked", async () => {
		const seed = sampleFleet();
		seed.live[EDGE]?.certificateRequests?.push({
			request_id: "aaaaaaaa-0000-4000-8000-000000000001",
			certificate_id: EDGE_API,
			label: "edge-api",
			expected_revision: 4,
			dns_names: ["edge-berlin.rheosoph.example"],
			ip_addresses: [],
			csr_pem: REQUEST_PEM,
			created_at: SAMPLE_NOW,
			expires_at: SAMPLE_NOW + 30 * DAY,
			purpose: "service",
		});
		const view = await mount({ seed });
		const created = acceptRequests(view, "service");
		await chooseFromRowMenu(view, EDGE_API, /Create signing request/);
		const sheet = byRole("dialog");
		expect(text(sheet)).toContain(
			"A signing request for edge-api is already waiting.",
		);
		const submit = byRole("button", "Create signing request", sheet);
		expect(disabled(submit)).toBe(true);
		await click(submit);
		await settled(view);
		expect(created).toHaveLength(0);
	});

	test("discarding a request asks first, then sends one command", async () => {
		const view = await mount();
		const requests = block(view, "device-certificate-requests");
		await click(byRole("button", "Discard request…", requests));
		const sheet = byRole("alertdialog");
		expect(text(sheet)).toContain(
			"The device destroys the key it made for this request.",
		);
		expect(sent(view, "delete_certificate_request")).toHaveLength(0);
		await click(
			byRole("button", "Discard the signing request for billing-api", sheet),
		);
		await settled(view);
		expect(sent(view, "delete_certificate_request")).toEqual([
			{ type: "delete_certificate_request", request_id: BILLING_REQUEST },
		]);
		expect(text(block(view, "device-certificate-requests"))).toContain(
			"Discard the signing request for billing-api: done.",
		);
		expect(text(block(view, "device-certificates"))).toContain(
			"2 of 32 certificate slots used",
		);
	});
});

describe("signing with the organisation authority", () => {
	test("only the signed chain goes to the device, and the password is dropped", async () => {
		const view = await mount();
		const installs = acceptInstalls(view);
		const sheet = await openSign(view);
		expect(text(sheet)).toContain("Exact namesbilling.lab.internal");
		expect(text(sheet)).toContain("Rheosoph Internal");
		await click(byRole("button", "Sign and install", sheet));
		expect(text(sheet)).toContain("Enter the authority's password.");
		expect(installs).toHaveLength(0);
		await typeInto(passwordField(sheet), view.fake.password);
		await click(byRole("button", "Sign and install", sheet));
		await settled(view);
		expect(installs).toHaveLength(1);
		expect(installs[0]?.request_id).toBe(BILLING_REQUEST);
		expect(`${installs[0]?.certificate_chain_pem}`).toContain(
			"BEGIN CERTIFICATE",
		);
		expect(JSON.stringify(installs[0])).not.toContain(view.fake.password);
		expect(queryByRole("dialog")).toBeNull();
		expect(text(view.container)).toContain(
			"Sign and install the certificate for billing-api: done.",
		);
	});

	test("a failed signing clears the password, hides the cause and sends nothing", async () => {
		const view = await mount();
		const installs = acceptInstalls(view);
		view.fake.crypto.signServiceCertificate = () => {
			throw new Error("private-signing-key: wrong padding");
		};
		const sheet = await openSign(view);
		await typeInto(passwordField(sheet), "not the authority password");
		await click(byRole("button", "Sign and install", sheet));
		await settled(view);
		const open = byRole("dialog");
		expect(passwordField(open).value).toBe("");
		expect(text(open)).toContain("The request couldn't be signed.");
		expect(text(open)).toContain("Nothing was sent to the device.");
		expect(text()).not.toContain("private-signing-key");
		expect(installs).toHaveLength(0);
		expect(writes(view)).toHaveLength(0);
	});

	test("without an authority on this computer the sheet says so and leads to creating one", async () => {
		const seed = sampleFleet();
		seed.local = { ...seed.local, authorities: [] };
		const view = await mount({ seed });
		const installs = acceptInstalls(view);
		const sheet = await openSign(view);
		expect(text(sheet)).toContain(
			"There's no organisation authority on this computer.",
		);
		const create = byRole("link", "Create an organisation authority", sheet);
		expect(create.getAttribute("href")).toContain("view=certificates");
		const submit = byRole("button", "Sign and install", sheet);
		expect(disabled(submit)).toBe(true);
		await click(submit);
		expect(installs).toHaveLength(0);
	});
});

describe("automatic renewal with your authority", () => {
	test("a failing renewal names the expired authority, and stopping asks first", async () => {
		const view = await mount();
		const renewal = block(view, "device-certificate-renewal");
		expect(text(renewal)).toContain(
			"Automatic renewal of internal-mqtt is failing.",
		);
		expect(text(renewal)).toContain("The authority that renews it expired");
		expect(text(renewal)).toContain(
			"Approved namesmqtt.lab.internal, 10.0.4.20",
		);
		expect(text(renewal)).toContain("Certificate lifetime30 d");
		expect(text(renewal)).toContain("Next renewalwas due");
		expect(renewal.querySelector("[data-authority-expired]")).not.toBeNull();
		await click(byRole("button", "Stop renewal…", renewal));
		const sheet = byRole("alertdialog");
		expect(text(sheet)).toContain("Stop automatic renewal of internal-mqtt?");
		expect(text(sheet)).toContain("Set up automatic renewal again.");
		expect(sent(view, "delete_certificate_issuer")).toHaveLength(0);
		await click(
			byRole("button", "Stop automatic renewal of internal-mqtt", sheet),
		);
		await settled(view);
		expect(sent(view, "delete_certificate_issuer")).toEqual([
			{
				type: "delete_certificate_issuer",
				certificate_id: INTERNAL_MQTT,
				expected_revision: 1,
			},
		]);
		expect(text(view.container)).toContain(
			"Stop automatic renewal of internal-mqtt: done.",
		);
		expect(text(row(view, INTERNAL_MQTT))).toContain("Manual");
	});

	test("fixing it, step 1: the device makes the request with the lifetime, and the signing step opens", async () => {
		const view = await mount();
		const prepared = acceptRequests(view, "issuer");
		await click(byRole("button", "Fix renewal…", row(view, INTERNAL_MQTT)));
		const sheet = byRole("dialog");
		expect(text(sheet)).toContain(
			"Install a new renewal authority for internal-mqtt",
		);
		expect(text(sheet)).toContain("Step 1 of 2");
		const days = byRole("textbox", LIFETIME_FIELD, sheet);
		await typeInto(days, "500");
		await click(byRole("button", "Continue", sheet));
		expect(text(sheet)).toContain("Enter a whole number from 1 to 397.");
		expect(prepared).toHaveLength(0);
		await typeInto(days, "45");
		await click(byRole("button", "Continue", sheet));
		await settled(view);
		expect(prepared).toHaveLength(1);
		expect(prepared[0]).toMatchObject({
			certificate_id: INTERNAL_MQTT,
			expected_revision: 2,
			dns_names: ["mqtt.lab.internal"],
			ip_addresses: ["10.0.4.20"],
			leaf_lifetime_days: 45,
		});
		expect(text(byRole("dialog"))).toContain(
			"Sign the renewal authority for internal-mqtt",
		);
	});

	test("fixing it, step 2: the authority signs only after the approval, and the device renews again", async () => {
		const view = await mount();
		const prepared = acceptRequests(view, "issuer");
		const installed = acceptAuthorities(view, 45);
		await click(byRole("button", "Fix renewal…", row(view, INTERNAL_MQTT)));
		await typeInto(byRole("textbox", LIFETIME_FIELD, byRole("dialog")), "45");
		await click(byRole("button", "Continue", byRole("dialog")));
		await settled(view);
		const sheet = byRole("dialog");
		expect(text(sheet)).toContain("Exact namesmqtt.lab.internal, 10.0.4.20");
		await typeInto(passwordField(sheet), view.fake.password);
		const submit = byRole("button", "Sign and install", sheet);
		expect(disabled(submit)).toBe(true);
		expect(footReason(sheet)).toBe("Tick the confirmation above first.");
		await click(submit);
		expect(installed).toHaveLength(0);
		await click(byRole("checkbox", /I authorise this device/, sheet));
		await typeInto(passwordField(byRole("dialog")), view.fake.password);
		await click(byRole("button", "Sign and install", byRole("dialog")));
		await settled(view);
		expect(installed).toHaveLength(1);
		expect(installed[0]?.request_id).toBe(prepared[0]?.request_id);
		expect(`${installed[0]?.certificate_chain_pem}`).toContain(
			"BEGIN CERTIFICATE",
		);
		expect(queryByRole("dialog")).toBeNull();
		const renewal = block(view, "device-certificate-renewal");
		expect(text(renewal)).not.toContain("is failing");
		expect(text(renewal)).toContain("Certificate lifetime45 d");
	});
});

describe("Let's Encrypt", () => {
	test("the agreement can be ticked only after the device's settings were read, and nothing is set up before it", async () => {
		const fake = await createFakeWorkspace();
		const release = fake.api.agent(EDGE).hold("acme_certificates");
		const view = await mount({ fake });
		const configured = acceptSetups(view);
		await chooseFromAddMenu(view, SET_UP_LETS_ENCRYPT);
		let sheet = byRole("dialog");
		expect(text(sheet)).toContain(
			"Reading the device's current Let's Encrypt settings before you can agree",
		);
		expect(terms(sheet).hasAttribute("disabled")).toBe(true);
		expect(footReason(sheet)).toBe(
			"The device's current settings aren't read yet.",
		);
		release();
		await settled(view);
		sheet = byRole("dialog");
		expect(terms(sheet).hasAttribute("disabled")).toBe(false);
		expect(footReason(sheet)).toBe("Tick the confirmation above first.");
		await typeInto(byRole("textbox", "Label", sheet), "public-api");
		await typeInto(
			byRole("textbox", "Public DNS names", sheet),
			"api.rheosoph.example",
		);
		const submit = byRole("button", "Set up Let's Encrypt", sheet);
		expect(disabled(submit)).toBe(true);
		await click(submit);
		expect(configured).toHaveLength(0);
		await click(terms(sheet));
		expect(configured).toHaveLength(0);
		expect(footReason(sheet)).toBeUndefined();
		await click(byRole("button", "Set up Let's Encrypt", sheet));
		await settled(view);
		expect(configured).toHaveLength(1);
		expect(configured[0]).toMatchObject({
			label: "public-api",
			expected_revision: 0,
			expected_certificate_revision: 0,
			dns_names: ["api.rheosoph.example"],
			environment: "lets_encrypt_staging",
			http_bind: "0.0.0.0:80",
			terms_of_service_agreed: true,
		});
		expect(queryByRole("dialog")).toBeNull();
		expect(text(block(view, "device-certificate-acme"))).toContain(
			"public-api",
		);
	});

	test("a service that already listens on the challenge port blocks the setup and is named", async () => {
		const seed = sampleFleet();
		const placements = seed.live[EDGE]?.placements;
		if (!placements?.["support-bot"])
			throw new Error("the sample fleet changed");
		placements["support-bot"] = { ...placements["support-bot"], port: 80 };
		const view = await mount({ seed });
		const configured = acceptSetups(view);
		await chooseFromAddMenu(view, SET_UP_LETS_ENCRYPT);
		const sheet = byRole("dialog");
		await typeInto(byRole("textbox", "Label", sheet), "public-api");
		await typeInto(
			byRole("textbox", "Public DNS names", sheet),
			"api.rheosoph.example",
		);
		await click(terms(sheet));
		expect(text(sheet)).toContain("support-bot already listens on port 80.");
		const submit = byRole("button", "Set up Let's Encrypt", sheet);
		expect(disabled(submit)).toBe(true);
		expect(footReason(sheet)).toBe(
			"The challenge port is taken by a service on this device.",
		);
		await click(submit);
		expect(configured).toHaveLength(0);
		await typeInto(
			byRole("textbox", "Challenge address", sheet),
			"0.0.0.0:8081",
		);
		expect(text(sheet)).toContain(
			"invoice-extractor already listens on port 8081.",
		);
		await typeInto(
			byRole("textbox", "Challenge address", sheet),
			"0.0.0.0:8088",
		);
		expect(text(sheet)).not.toContain("already listens");
		await click(byRole("button", "Set up Let's Encrypt", sheet));
		await settled(view);
		expect(configured).toHaveLength(1);
		expect(configured[0]?.http_bind).toBe("0.0.0.0:8088");
	});

	test("names are checked, changing them withdraws the agreement, and IP names are dropped only with a tick", async () => {
		const view = await mount();
		const configured = acceptSetups(view);
		view.fake.api.agent(EDGE).certificateIssuers = [];
		await chooseFromRowMenu(view, INTERNAL_MQTT, SET_UP_LETS_ENCRYPT);
		const sheet = byRole("dialog");
		expect(text(sheet)).toContain("Set up Let's Encrypt for internal-mqtt");
		expect(text(sheet)).toContain(
			"Clients that connect by 10.0.4.20 get certificate errors",
		);
		const names = byRole("textbox", "Public DNS names", sheet);
		await click(terms(sheet));
		await typeInto(names, "mqtt\n10.0.4.20");
		expect(terms(sheet).getAttribute("aria-checked")).toBe("false");
		await click(terms(sheet));
		await click(byRole("checkbox", /10\.0\.4\.20 is dropped/, sheet));
		await click(byRole("button", "Set up Let's Encrypt", sheet));
		expect(text(sheet)).toContain(
			"Let's Encrypt can't issue for IP addresses.",
		);
		await typeInto(names, "mqtt");
		await click(terms(sheet));
		await click(byRole("button", "Set up Let's Encrypt", sheet));
		expect(text(sheet)).toContain("Every name must be a public DNS name");
		expect(configured).toHaveLength(0);
		await typeInto(names, "mqtt.rheosoph.example");
		await click(terms(sheet));
		await click(byRole("button", "Set up Let's Encrypt", sheet));
		await settled(view);
		expect(configured).toHaveLength(1);
		expect(configured[0]).toMatchObject({
			certificate_id: INTERNAL_MQTT,
			label: "internal-mqtt",
			expected_certificate_revision: 2,
			dns_names: ["mqtt.rheosoph.example"],
		});
	});

	test("the setup shows what Let's Encrypt needs, and stopping asks first", async () => {
		const view = await mount();
		const acme = block(view, "device-certificate-acme");
		expect(text(acme)).toContain("Challenge address0.0.0.0:80");
		expect(text(acme)).toContain("EnvironmentProduction");
		expect(text(acme)).toContain("Last errorNone");
		expect(
			allByRole("listitem", undefined, acme).map((item) =>
				item.getAttribute("data-state"),
			),
		).toEqual(["pass", "pass", "pass"]);
		await click(byRole("button", "Stop Let's Encrypt…", acme));
		const sheet = byRole("alertdialog");
		expect(text(sheet)).toContain("Stop Let's Encrypt for edge-api?");
		expect(sent(view, "delete_acme_certificate")).toHaveLength(0);
		await click(byRole("button", "Stop Let's Encrypt for edge-api", sheet));
		await settled(view);
		expect(sent(view, "delete_acme_certificate")).toEqual([
			{
				type: "delete_acme_certificate",
				certificate_id: EDGE_API,
				expected_revision: 3,
			},
		]);
		expect(text(view.container)).toContain(
			"Stop Let's Encrypt for edge-api: done.",
		);
	});
});
