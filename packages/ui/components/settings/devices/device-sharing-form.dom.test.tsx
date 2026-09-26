import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import type {
	DeviceReceipt,
	ManagementPolicy,
	OnboardingManifest,
} from "../../../lib/device-management/types";
import type { IProfile } from "../../../types";

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
	HTMLTextAreaElement: window.HTMLTextAreaElement,
	MutationObserver: window.MutationObserver,
	Event: window.Event,
	IS_REACT_ACT_ENVIRONMENT: true,
});
const key = { kty: "OKP" as const, crv: "Ed25519" as const, x: "a".repeat(43) };
const previous: ManagementPolicy = {
	version: 1,
	device_id: "device",
	policy_version: 1,
	previous_policy_digest: null,
	issued_at: 1,
	expires_at: Math.floor(Date.now() / 1000) + 86400,
	grants: [
		{
			grant_id: "existing",
			user_id: "existing-user",
			controller_key: key,
			scope: { kind: "device" },
			capabilities: ["status", "manage_certificates"],
			expires_at: Math.floor(Date.now() / 1000) + 3600,
			group_id: null,
			group_version: null,
		},
	],
};
const digest = Buffer.from(
	await crypto.subtle.digest(
		"SHA-256",
		new TextEncoder().encode("previous-policy"),
	),
).toString("base64url");
let signed: ManagementPolicy[] = [];
let puts = 0;
let pendingRead: Promise<void> | undefined;
mock.module("../../../state/backend-state", () => ({
	useBackend: () => ({
		apiState: {
			get: async () => {
				await pendingRead;
				return { version: 1, digest, policy_jws: "previous-policy" };
			},
			put: async () => {
				puts++;
				return { version: 2, digest: "new", policy_jws: "new-policy" };
			},
		},
	}),
}));
mock.module("../../../lib/device-management/crypto", () => ({
	base64url: (value: Uint8Array) => Buffer.from(value).toString("base64url"),
	withPassword: async <T,>(
		password: string,
		operation: (value: Uint8Array) => T,
	) => {
		const bytes = new TextEncoder().encode(password);
		try {
			return operation(bytes);
		} finally {
			bytes.fill(0);
		}
	},
	loadDeviceCrypto: async () => ({
		verifyManagementPolicy: () => structuredClone(previous),
		signManagementPolicy: (policy: ManagementPolicy) => {
			signed.push(structuredClone(policy));
			return "new-policy";
		},
	}),
}));
const { createRoot } = await import("react-dom/client");
const { DeviceSharingForm } = await import("./device-sharing-form");
const container = document.createElement("div");
document.body.append(container);
const root = createRoot(container);
const manifest = {
	device_id: "device",
	controller_key: key,
	owner_invitation_key: key,
} as OnboardingManifest;
const receipt = { device_id: "device" } as DeviceReceipt;
async function render(supported?: boolean) {
	await act(async () =>
		root.render(
			<DeviceSharingForm
				profile={{ id: "profile" } as IProfile}
				manifest={manifest}
				receipt={receipt}
				invitationVault={new Uint8Array([1])}
				{...(supported === undefined
					? {}
					: { certificateManagement: supported })}
			/>,
		),
	);
}
function permission() {
	const input = [...container.querySelectorAll("label")]
		.find((label) => label.textContent === "Manage service certificates")
		?.querySelector("input");
	if (!input) throw new Error("Missing certificate permission");
	return input;
}
async function fields() {
	await act(async () => {
		const recipients = container.querySelector("textarea");
		Object.getOwnPropertyDescriptor(
			window.HTMLTextAreaElement.prototype,
			"value",
		)?.set?.call(
			recipients,
			JSON.stringify([{ user_id: "new-user", controller_key: key }]),
		);
		recipients?.dispatchEvent(new Event("input", { bubbles: true }));
		const password = container.querySelector('input[type="password"]');
		Object.getOwnPropertyDescriptor(
			window.HTMLInputElement.prototype,
			"value",
		)?.set?.call(password, "owner password");
		password?.dispatchEvent(new Event("input", { bubbles: true }));
	});
}
async function submit() {
	await act(async () =>
		container
			.querySelector("form")
			?.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })),
	);
	await act(async () => new Promise((resolve) => setTimeout(resolve, 10)));
}
afterEach(async () => {
	await act(async () => root.render(null));
	signed = [];
	puts = 0;
	pendingRead = undefined;
});
afterAll(async () => {
	await act(async () => root.unmount());
	mock.restore();
	await window.happyDOM.close();
});

test("unknown and unsupported agents disable certificate sharing while ordinary sharing preserves existing grants", async () => {
	await render();
	expect(permission().disabled).toBe(true);
	expect(container.textContent).toContain(
		"Update the device's standalone binary and reconnect",
	);
	await render(false);
	expect(permission().disabled).toBe(true);
	await fields();
	await submit();
	expect(signed).toHaveLength(1);
	expect(puts).toBe(1);
	expect(signed[0].grants[0]).toEqual(previous.grants[0]);
	expect(signed[0].grants[1].capabilities).toEqual([
		"status",
		"metrics",
		"logs",
	]);
});

test("supported agents allow granting device certificate management", async () => {
	await render(true);
	expect(permission().disabled).toBe(false);
	await act(async () => permission().click());
	await fields();
	await submit();
	expect(signed[0].grants[1].capabilities).toContain("manage_certificates");
	expect(puts).toBe(1);
});

test("a previously selected certificate permission is rejected before signing after support is lost", async () => {
	await render(true);
	await act(async () => permission().click());
	await fields();
	await render(false);
	expect(permission().checked).toBe(true);
	expect(permission().disabled).toBe(false);
	await submit();
	expect(container.querySelector('[role="alert"]')?.textContent).toContain(
		"before sharing certificate management permission",
	);
	expect(signed).toHaveLength(0);
	expect(puts).toBe(0);
	await act(async () => permission().click());
	expect(permission().checked).toBe(false);
	expect(permission().disabled).toBe(true);
	await fields();
	await submit();
	expect(signed).toHaveLength(1);
	expect(signed[0].grants[1].capabilities).not.toContain("manage_certificates");
	expect(puts).toBe(1);
});

test("support is rechecked at signing if a pending policy read spans a device capability change", async () => {
	let finish!: () => void;
	pendingRead = new Promise<void>((resolve) => {
		finish = resolve;
	});
	await render(true);
	await act(async () => permission().click());
	await fields();
	await submit();
	await render(false);
	await act(async () => finish());
	await act(async () => new Promise((resolve) => setTimeout(resolve, 10)));
	expect(signed).toHaveLength(0);
	expect(puts).toBe(0);
	expect(container.querySelector('[role="alert"]')?.textContent).toContain(
		"before sharing certificate management permission",
	);
});
