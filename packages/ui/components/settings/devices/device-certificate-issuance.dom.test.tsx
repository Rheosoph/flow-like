import { afterAll, afterEach, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { StrictMode, act } from "react";
import type { CertificateRequest } from "../../../lib/device-management/certificate-issuance";
import type { DeviceCertificate } from "../../../lib/device-management/certificates";
import type { ManagementCall } from "../../../lib/device-management/telemetry";

const window = new Window({ url: "https://app.test" });
Object.assign(window, { SyntaxError, TypeError, Error });
Object.assign(globalThis, {
	window,
	document: window.document,
	navigator: window.navigator,
	HTMLElement: window.HTMLElement,
	Element: window.Element,
	Node: window.Node,
	HTMLInputElement: window.HTMLInputElement,
	MutationObserver: window.MutationObserver,
	Event: window.Event,
	IS_REACT_ACT_ENVIRONMENT: true,
});
const { createRoot } = await import("react-dom/client");
const { DeviceCertificateIssuance } = await import(
	"./device-certificate-issuance"
);
const { DeviceCertificates } = await import("./device-certificates");
const container = document.createElement("div");
document.body.append(container);
const root = createRoot(container);
const id = "00000000-0000-4000-8000-000000000001";
const now = Math.floor(Date.now() / 1000);
const scope = {
	issuer: "issuer",
	account: "owner",
	apiOrigin: "https://hub.test",
	profileId: "profile",
};
const certificate: DeviceCertificate = {
	certificate_id: id,
	label: "Gateway",
	revision: 3,
	subject: "CN=edge.example.com",
	issuer: "CN=Org",
	dns_names: ["edge.example.com"],
	ip_addresses: [],
	sha256_fingerprint: "a".repeat(64),
	not_before: now - 10,
	not_after: now + 3600,
	bindings: [],
};
const request: CertificateRequest = {
	request_id: id,
	certificate_id: id,
	label: "Gateway",
	expected_revision: 3,
	dns_names: certificate.dns_names,
	ip_addresses: [],
	csr_pem: "public CSR",
	created_at: now,
	expires_at: now + 3600,
	purpose: "service",
};
let requests: CertificateRequest[] = [request];
let commands: Record<string, unknown>[] = [];
const call: ManagementCall = async (command, operationId) => {
	commands.push(command);
	let result: Record<string, unknown> = {};
	if (command.type === "certificate_requests")
		result = { requests, next: null };
	if (command.type === "certificates")
		result = { certificates: [certificate], inventory_revision: 1, next: null };
	if (command.type === "certificate_issuers")
		result = { issuers: [], next: null };
	if (command.type === "install_certificate_request") {
		requests = [];
		result = { certificate: { ...certificate, revision: 4 } };
	}
	if (command.type === "install_certificate_issuer") {
		requests = [];
		result = {
			issuer: {
				certificate_id: id,
				revision: 1,
				dns_names: certificate.dns_names,
				ip_addresses: [],
				leaf_lifetime_days: 30,
				not_after: now + 3600,
				last_renewed_at: null,
				next_renewal_at: now,
				last_error: null,
			},
		};
	}
	if (command.type === "delete_certificate_request") {
		requests = [];
		result = { request_id: id, deleted: true };
	}
	return { operation_id: operationId ?? "read", state: "completed", result };
};
async function render(canDelegate = true) {
	await act(async () =>
		root.render(
			<StrictMode>
				<DeviceCertificateIssuance
					scope={scope}
					run={(operation) => operation(call)}
					certificates={[certificate]}
					onChanged={() => {}}
					canDelegate={canDelegate}
				/>
			</StrictMode>,
		),
	);
}
function button(label: string) {
	const found = [...container.querySelectorAll("button")].find(
		(value) => value.textContent === label,
	);
	if (!found) throw new Error(`Missing ${label}`);
	return found;
}
async function click(label: string) {
	await act(async () => button(label).click());
}
async function chooseChain(
	contents:
		| string
		| Promise<string> = "-----BEGIN CERTIFICATE-----\nYWI=\n-----END CERTIFICATE-----",
) {
	const input = [...container.querySelectorAll("label")]
		.find(
			(value) =>
				value.textContent?.includes("PEM chain") ||
				value.textContent?.includes("constrained issuing chain"),
		)
		?.querySelector("input");
	expect(input).toBeDefined();
	Object.defineProperty(input, "files", {
		configurable: true,
		value: [{ size: 100, text: async () => await contents }],
	});
	return input!.closest("form")!;
}
afterEach(async () => {
	await act(async () => root.render(null));
	requests = [request];
	commands = [];
});
afterAll(async () => {
	await act(async () => root.unmount());
	window.happyDOM.abort();
});

test("enterprise CSR flow shows exact names and installs only a signed chain", async () => {
	await render();
	await click("Refresh requests and renewal");
	expect(container.textContent).toContain("Exact names: edge.example.com");
	expect(button("Download CSR for Gateway").disabled).toBe(false);
	const form = await chooseChain();
	await act(async () =>
		form.dispatchEvent(
			new Event("submit", { bubbles: true, cancelable: true }),
		),
	);
	const command = commands.find(
		(value) => value.type === "install_certificate_request",
	);
	expect(command?.request_id).toBe(id);
	expect(command?.certificate_chain_pem).toContain("BEGIN CERTIFICATE");
	expect(command).not.toHaveProperty("private_key_pem");
	expect(container.textContent).toContain("signed certificate was installed");
});

test("shared certificate managers do not read or configure owner renewal policies", async () => {
	await render(false);
	await click("Refresh requests and renewal");
	expect(commands.some((value) => value.type === "certificate_issuers")).toBe(
		false,
	);
	expect(container.textContent).toContain("Only the device owner");
	expect(container.textContent).not.toContain("Prepare renewal for Gateway");
	expect(button("Install signed service certificate").disabled).toBe(false);
});

test("issuing key installation requires explicit approval of the exact names and lifetime", async () => {
	requests = [{ ...request, purpose: "issuer", leaf_lifetime_days: 30 }];
	await render();
	await click("Refresh requests and renewal");
	expect(button("Install signed issuing certificate").disabled).toBe(true);
	expect(container.textContent).toContain("maximum lifetime of 30 days");
	expect(container.textContent).toContain(
		"Flow-Like constrained issuing chain",
	);
	const checkbox = [...container.querySelectorAll("label")]
		.find((value) => value.textContent?.includes("I authorise this device"))
		?.querySelector<HTMLInputElement>('input[type="checkbox"]');
	await act(async () => checkbox?.click());
	expect(button("Install signed issuing certificate").disabled).toBe(false);
	const form = await chooseChain();
	await act(async () =>
		form.dispatchEvent(
			new Event("submit", { bubbles: true, cancelable: true }),
		),
	);
	expect(
		commands.some((value) => value.type === "install_certificate_issuer"),
	).toBe(true);
	expect(container.textContent).toContain("Renewal is delegated");
});

test("locking while reading a signed chain prevents the upload", async () => {
	await render();
	await click("Refresh requests and renewal");
	let finish!: (value: string) => void;
	const content = new Promise<string>((resolve) => {
		finish = resolve;
	});
	const form = await chooseChain(content);
	await act(async () =>
		form.dispatchEvent(
			new Event("submit", { bubbles: true, cancelable: true }),
		),
	);
	await act(async () => root.render(null));
	await act(async () =>
		finish("-----BEGIN CERTIFICATE-----\nYWI=\n-----END CERTIFICATE-----"),
	);
	expect(
		commands.some((value) => value.type === "install_certificate_request"),
	).toBe(false);
});

test("legacy agents and shared users never see unavailable certificate issuance controls", async () => {
	await act(async () =>
		root.render(
			<DeviceCertificates
				run={(operation) => operation(call)}
				scope={scope}
				canManage
				issuance={false}
				acme={false}
			/>,
		),
	);
	expect(container.textContent).toContain(
		"Update this device's standalone binary",
	);
	expect(container.textContent).not.toContain(
		"Create certificates on this device",
	);
	await act(async () =>
		root.render(
			<DeviceCertificates
				run={(operation) => operation(call)}
				scope={scope}
				canManage={false}
				issuance
				acme
				canDelegate={false}
			/>,
		),
	);
	expect(container.textContent).not.toContain("Automatic public certificates");
	expect(container.textContent).not.toContain(
		"Create certificates on this device",
	);
	await act(async () =>
		root.render(
			<DeviceCertificates
				run={(operation) => operation(call)}
				scope={scope}
				canManage
				issuance
				acme
				canDelegate={false}
			/>,
		),
	);
	expect(container.textContent).toContain("Create certificates on this device");
	expect(container.textContent).not.toContain("Automatic public certificates");
	expect(container.textContent).not.toContain(
		"Owner-approved automatic renewal",
	);
});
