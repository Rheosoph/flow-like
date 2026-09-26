import { afterAll, afterEach, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { StrictMode, act } from "react";
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
const { DeviceCertificates } = await import("./device-certificates");
const container = document.createElement("div");
document.body.append(container);
const root = createRoot(container);
const id = "00000000-0000-4000-8000-000000000001";
const now = Math.floor(Date.now() / 1000);
const certificate: DeviceCertificate = {
	certificate_id: id,
	label: "Edge API",
	revision: 3,
	subject: "CN=edge.test",
	issuer: "CN=Private CA",
	dns_names: ["edge.test"],
	ip_addresses: ["10.0.0.1"],
	sha256_fingerprint: "a".repeat(64),
	not_before: now - 100,
	not_after: now + 3600,
	bindings: [
		{ placement_id: "placement", project_id: "project", service: "https" },
	],
};
let commands: Record<string, unknown>[] = [];
let installed: DeviceCertificate[] = [certificate];
let rejectImport = false;
const call: ManagementCall = async (command, operationId) => {
	commands.push(command);
	if (command.type === "put_certificate" && rejectImport)
		throw new Error(String(command.private_key_pem));
	if (command.type === "put_certificate")
		installed = installed.map((value) =>
			value.certificate_id === command.certificate_id
				? {
						...value,
						revision: Number(command.expected_revision) + 1,
						label: String(command.label),
					}
				: value,
		);
	return {
		operation_id: operationId ?? "read",
		state: "completed",
		result:
			command.type === "certificates"
				? { certificates: installed, inventory_revision: 1, next: null }
				: command.type === "put_certificate"
					? { certificate: installed[0], inventory_revision: 2 }
					: { certificate_id: command.certificate_id, deleted: true },
	};
};
async function render(canManage = true) {
	await act(async () =>
		root.render(
			<StrictMode>
				<DeviceCertificates
					canManage={canManage}
					run={(operation) => operation(call)}
				/>
			</StrictMode>,
		),
	);
}
function button(text: string) {
	const value = [...container.querySelectorAll("button")].find(
		(element) => element.textContent === text,
	);
	if (!value) throw new Error(`Missing button ${text}`);
	return value;
}
async function click(text: string) {
	await act(async () => button(text).click());
}
async function submitReplacement() {
	expect(button("Send replacement to device").disabled).toBe(false);
	await act(async () =>
		container
			.querySelector("form")
			?.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })),
	);
}
async function files(keyContents?: Promise<string>) {
	const [chainInput, keyInput] =
		container.querySelectorAll<HTMLInputElement>('input[type="file"]');
	const chain = "-----BEGIN CERTIFICATE-----\nYWJj\n-----END CERTIFICATE-----";
	const key =
		"-----BEGIN PRIVATE KEY-----\nnever-display-this-secret\n-----END PRIVATE KEY-----";
	await act(async () => {
		for (const [input, name, content] of [
			[chainInput, "chain.pem", chain],
			[keyInput, "key.pem", key],
		] as const) {
			Object.defineProperty(input, "files", {
				configurable: true,
				value: [
					{
						name,
						size: content.length,
						text: async () =>
							name === "key.pem" && keyContents ? await keyContents : content,
					},
				],
			});
			input.dispatchEvent(new Event("change", { bubbles: true }));
		}
	});
}
afterEach(async () => {
	await act(async () => root.render(null));
	commands = [];
	installed = [certificate];
	rejectImport = false;
});
afterAll(async () => {
	await act(async () => root.unmount());
	window.happyDOM.abort();
});

test("device view shows certificate expiration, names, fingerprint and placement use and guards deletion", async () => {
	await render();
	await click("Refresh certificates");
	expect(container.textContent).toContain("expiring within 7 days");
	expect(container.textContent).toContain("edge.test");
	expect(container.textContent).toContain("10.0.0.1");
	expect(container.textContent).toContain("CN=Private CA");
	expect(container.textContent).toContain("a".repeat(64));
	expect(container.textContent).toContain("project / placement (https)");
	expect(button("Delete Edge API").disabled).toBe(true);
	expect(commands).toEqual([{ type: "certificates", limit: 4 }]);
});

test("shared read-only access can refresh certificates but cannot import, replace or delete", async () => {
	installed = [{ ...certificate, bindings: [] }];
	await render(false);
	expect(button("Refresh certificates").disabled).toBe(false);
	await click("Refresh certificates");
	expect(container.textContent).toContain("edge.test");
	expect(button("Replace Edge API").disabled).toBe(true);
	expect(button("Delete Edge API").disabled).toBe(true);
	expect(container.querySelector('input[type="file"]')).toBeNull();
	expect(container.textContent).toContain(
		"certificate management permission is required",
	);
	expect(commands).toEqual([{ type: "certificates", limit: 4 }]);
});

test("replacement keeps certificate identity and revision and never renders the submitted key", async () => {
	await render();
	await click("Refresh certificates");
	await click("Replace Edge API");
	await files();
	await submitReplacement();
	const put = commands.find((command) => command.type === "put_certificate");
	expect(put?.certificate_id).toBe(id);
	expect(put?.expected_revision).toBe(3);
	expect(put?.private_key_pem).toContain("never-display-this-secret");
	expect(container.textContent).not.toContain("never-display-this-secret");
	expect(container.textContent).toContain("Certificate replaced");
	for (const input of container.querySelectorAll<HTMLInputElement>(
		'input[type="file"]',
	))
		expect(input.value).toBe("");
});

test("failed import sanitizes transport errors and clears the file selections", async () => {
	rejectImport = true;
	await render();
	await click("Refresh certificates");
	await click("Replace Edge API");
	await files();
	await submitReplacement();
	expect(container.textContent).toContain(
		"Certificate import was not confirmed",
	);
	expect(container.textContent).not.toContain("never-display-this-secret");
	for (const input of container.querySelectorAll<HTMLInputElement>(
		'input[type="file"]',
	))
		expect(input.value).toBe("");
	expect(button("Send replacement to device").disabled).toBe(true);
});

test("unassigned certificate deletion requires explicit confirmation", async () => {
	installed = [{ ...certificate, bindings: [] }];
	await render();
	await click("Refresh certificates");
	await click("Delete Edge API");
	expect(commands).toHaveLength(1);
	await click("Confirm certificate deletion");
	expect(commands[1]).toEqual({
		type: "delete_certificate",
		certificate_id: id,
		expected_revision: 3,
	});
});

test("locking during private key file reading prevents any import from reaching the device", async () => {
	await render();
	await click("Refresh certificates");
	await click("Replace Edge API");
	let finish!: (value: string) => void;
	const key = new Promise<string>((resolve) => {
		finish = resolve;
	});
	await files(key);
	await submitReplacement();
	await act(async () => root.render(null));
	await act(async () =>
		finish("-----BEGIN PRIVATE KEY-----\nsecret\n-----END PRIVATE KEY-----"),
	);
	expect(commands.some((command) => command.type === "put_certificate")).toBe(
		false,
	);
});
