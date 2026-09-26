import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";

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
const storage = { ...(await import("../../../lib/device-management/storage")) };
const cryptography = {
	...(await import("../../../lib/device-management/crypto")),
};
let failed = false;
let saved: unknown[] = [];
let envelopes = 0;
mock.module("../../../lib/device-management/storage", () => ({
	...storage,
	addCertificateAuthority: async (_scope: unknown, value: unknown) => {
		saved.push(value);
	},
	readCertificateAuthorities: async () => saved,
}));
mock.module("../../../lib/device-management/crypto", () => ({
	...cryptography,
	loadDeviceCrypto: async () => ({
		createCertificateAuthorityVault(spec: Record<string, unknown>) {
			if (failed) throw new Error("private-key-material-must-not-be-rendered");
			envelopes++;
			return {
				public_bundle: {
					...spec,
					root_certificate_pem: "root",
					issuer_certificate_pem: "issuer",
					sha256_fingerprint: "a".repeat(64),
					not_before: 100,
					not_after: 200,
					issuer_not_after: 150,
				},
				vault: Array(80).fill(1),
				root_vault: Array(80).fill(2),
			};
		},
		signServiceCertificate() {
			throw new Error("private-signing-key-must-not-be-rendered");
		},
	}),
}));
const { createRoot } = await import("react-dom/client");
const { DeviceCertificateAuthorities } = await import(
	"./device-certificate-authorities"
);
const { DeviceCertificateAcme } = await import("./device-certificate-acme");
const { DeviceCertificateIssuance } = await import(
	"./device-certificate-issuance"
);
const container = document.createElement("div");
document.body.append(container);
const root = createRoot(container);
const scope = {
	issuer: "issuer",
	account: "owner",
	apiOrigin: "https://hub.test",
	profileId: "profile",
};
function button(label: string) {
	const found = [...container.querySelectorAll("button")].find(
		(value) => value.textContent === label,
	);
	if (!found) throw new Error(`Missing ${label}`);
	return found;
}
function input(label: string) {
	const found = [...container.querySelectorAll("label")]
		.find((value) => value.textContent?.trim() === label)
		?.querySelector<HTMLInputElement>("input");
	if (!found) throw new Error(`Missing input ${label}`);
	return found;
}
async function fill(label: string, value: string) {
	const field = input(label);
	await act(async () => {
		Object.getOwnPropertyDescriptor(
			window.HTMLInputElement.prototype,
			"value",
		)?.set?.call(field, value);
		field.dispatchEvent(new Event("input", { bubbles: true }));
	});
}
async function create() {
	await act(async () =>
		root.render(
			<DeviceCertificateAuthorities
				scope={scope}
				authorities={[]}
				onChanged={() => {}}
			/>,
		),
	);
	await fill("Authority label", "Organisation");
	await fill("Permitted DNS suffixes", "example.com");
	await fill("New authority password", "a strong authority password");
	await fill("Repeat authority password", "a strong authority password");
	expect(button("Create certificate authority").disabled).toBe(false);
	await act(async () =>
		button("Create certificate authority")
			.closest("form")
			?.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })),
	);
}
afterEach(async () => {
	await act(async () => root.render(null));
	saved = [];
	failed = false;
	envelopes = 0;
});
afterAll(async () => {
	await act(async () => root.unmount());
	mock.module("../../../lib/device-management/storage", () => storage);
	mock.module("../../../lib/device-management/crypto", () => cryptography);
	window.happyDOM.abort();
});

test("the issuing key is persisted only after the user acknowledges the offline root backup", async () => {
	await create();
	expect(envelopes).toBe(1);
	expect(saved).toEqual([]);
	expect(button("Use this authority").disabled).toBe(true);
	expect(button("Download encrypted authority backup").disabled).toBe(false);
	const checkbox = [...container.querySelectorAll("label")]
		.find((value) =>
			value.textContent?.includes("I saved the encrypted root backup"),
		)
		?.querySelector<HTMLInputElement>('input[type="checkbox"]');
	await act(async () => checkbox?.click());
	await act(async () => button("Use this authority").click());
	expect(saved).toHaveLength(1);
	expect(saved[0]).not.toHaveProperty("root_vault");
	expect(input("New authority password").value).toBe("");
	expect(input("Repeat authority password").value).toBe("");
});

test("failed authority creation clears passwords and hides cryptography exception contents", async () => {
	failed = true;
	await create();
	expect(saved).toEqual([]);
	expect(input("New authority password").value).toBe("");
	expect(input("Repeat authority password").value).toBe("");
	expect(container.textContent).toContain("authority could not be created");
	expect(container.textContent).not.toContain("private-key-material");
});

test("public certificate issuance requires explicit ACME terms consent after settings have been read", async () => {
	let configured = 0;
	await act(async () =>
		root.render(
			<DeviceCertificateAcme
				certificates={[]}
				onChanged={() => {}}
				run={(operation) =>
					operation(async (command, operationId) => {
						if (command.type === "configure_acme_certificate") configured++;
						return {
							operation_id: operationId ?? "read",
							state: "completed",
							result:
								command.type === "acme_certificates"
									? { policies: [], next: null }
									: { certificates: [], next: null, inventory_revision: 1 },
						};
					})
				}
			/>,
		),
	);
	await act(async () => button("Refresh public certificate settings").click());
	await fill("Public certificate label", "Public API");
	await fill("Public DNS names", "api.example.com");
	expect(button("Enable public certificate issuance").disabled).toBe(true);
	expect(container.textContent).toContain(
		"public certificate transparency logs",
	);
	const checkbox = [...container.querySelectorAll("label")]
		.find((value) => value.textContent?.includes("I accept the"))
		?.querySelector<HTMLInputElement>('input[type="checkbox"]');
	await act(async () => checkbox?.click());
	expect(button("Enable public certificate issuance").disabled).toBe(false);
	expect(configured).toBe(0);
});

test("failed certificate signing clears the authority password and hides key-bearing exceptions", async () => {
	const id = "00000000-0000-4000-8000-000000000001";
	saved = [
		{
			public_bundle: {
				account_binding: storage.accountStorageKey(scope),
				authority_id: id,
				label: "Org",
				dns_suffixes: ["example.com"],
				ip_addresses: [],
				root_certificate_pem: "root",
				issuer_certificate_pem: "issuer",
				sha256_fingerprint: "a".repeat(64),
				not_before: 100,
				not_after: 200,
				issuer_not_after: 150,
			},
			vault: new Uint8Array(80).fill(1),
		},
	];
	let installs = 0;
	await act(async () =>
		root.render(
			<DeviceCertificateIssuance
				scope={scope}
				certificates={[]}
				onChanged={() => {}}
				run={(operation) =>
					operation(async (command, operationId) => {
						if (command.type === "install_certificate_request") installs++;
						return {
							state: "completed",
							operation_id: operationId ?? "read",
							result:
								command.type === "certificate_requests"
									? {
											requests: [
												{
													request_id: id,
													certificate_id: id,
													label: "Gateway",
													expected_revision: 0,
													dns_names: ["edge.example.com"],
													ip_addresses: [],
													csr_pem: "public csr",
													created_at: 100,
													expires_at: 200,
													purpose: "service",
												},
											],
											next: null,
										}
									: { certificates: [], inventory_revision: 1, next: null },
						};
					})
				}
			/>,
		),
	);
	await act(async () => button("Refresh requests and renewal").click());
	const select = [...container.querySelectorAll("label")]
		.find((value) =>
			value.textContent?.startsWith("Sign with organisation authority"),
		)
		?.querySelector("select");
	expect(select).toBeDefined();
	await act(async () => {
		select!.value = id;
		select!.dispatchEvent(new Event("change", { bubbles: true }));
	});
	await fill("Authority password", "an authority password");
	expect(button("Sign and install service certificate").disabled).toBe(false);
	await act(async () =>
		button("Sign and install service certificate")
			.closest("form")
			?.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })),
	);
	expect(input("Authority password").value).toBe("");
	expect(container.textContent).toContain("request could not be signed");
	expect(container.textContent).not.toContain("private-signing-key");
	expect(installs).toBe(0);
});
