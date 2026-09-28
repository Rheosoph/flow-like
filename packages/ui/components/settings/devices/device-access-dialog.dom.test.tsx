import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import type { LocalDeviceVault } from "../../../lib/device-management/storage";
import type {
	DeviceReceipt,
	OnboardingManifest,
} from "../../../lib/device-management/types";

const window = new Window({ url: "https://app.test" });
Object.assign(window, { SyntaxError, TypeError, Error });
Object.assign(globalThis, {
	window,
	document: window.document,
	navigator: window.navigator,
	HTMLElement: window.HTMLElement,
	HTMLInputElement: window.HTMLInputElement,
	Element: window.Element,
	DocumentFragment: window.DocumentFragment,
	CustomEvent: window.CustomEvent,
	NodeFilter: window.NodeFilter,
	getComputedStyle: window.getComputedStyle.bind(window),
	requestAnimationFrame: (callback: FrameRequestCallback) =>
		setTimeout(() => callback(0), 0),
	cancelAnimationFrame: clearTimeout,
	Node: window.Node,
	MutationObserver: window.MutationObserver,
	Event: window.Event,
	IS_REACT_ACT_ENVIRONMENT: true,
});
const ownerKey = {
	kty: "OKP" as const,
	crv: "Ed25519" as const,
	x: "o".repeat(43),
};
const sharedKey = {
	kty: "OKP" as const,
	crv: "Ed25519" as const,
	x: "s".repeat(43),
};
const manifest = {
	device_id: "device",
	name: "Workshop",
	owner_id: "owner",
	api_base_url: "https://hub.test/api/v1",
	controller_key: ownerKey,
} as OnboardingManifest;
const receipt = {
	device_id: "device",
	manifest_jws: "manifest",
} as DeviceReceipt;
const scope = {
	issuer: "issuer",
	account: "reader",
	apiOrigin: "https://hub.test",
	profileId: "profile",
};
let existing: LocalDeviceVault | undefined;
const created: LocalDeviceVault[] = [];
const actualCrypto = {
	...(await import("../../../lib/device-management/crypto")),
};
const actualStorage = {
	...(await import("../../../lib/device-management/storage")),
};
mock.module("../../../lib/device-management/crypto", () => ({
	...actualCrypto,
	loadDeviceCrypto: async () => ({
		verifyDeviceReceipt: () => manifest,
	}),
}));
mock.module("../../../lib/device-management/storage", () => ({
	...actualStorage,
	readDeviceVault: async () => existing,
	addDeviceVault: async (_scope: unknown, vault: LocalDeviceVault) => {
		created.push(vault);
	},
}));
const { createRoot } = await import("react-dom/client");
const { DeviceAccessDialog } = await import("./device-access-dialog");
const container = document.createElement("div");
document.body.append(container);
const root = createRoot(container);
afterEach(async () => {
	await act(async () => root.render(null));
	existing = undefined;
	created.length = 0;
});
afterAll(async () => {
	await act(async () => root.unmount());
	mock.module("../../../lib/device-management/crypto", () => actualCrypto);
	mock.module("../../../lib/device-management/storage", () => actualStorage);
	mock.restore();
	await window.happyDOM.close();
});
function vault(update: Partial<LocalDeviceVault>): LocalDeviceVault {
	return {
		deviceId: "device",
		grantId: "lost-request-grant",
		manifestJws: "manifest",
		ownerControllerKey: ownerKey,
		controllerPublic: {
			device_id: "device",
			endpoint_id: "endpoint",
			controller_key: sharedKey,
			archive_key: Array(32).fill(1),
			telemetry_member: { endpoint_id: "endpoint", signing_key: sharedKey },
		},
		controllerVault: new Uint8Array(80),
		...update,
	};
}
async function importBundle() {
	await act(async () =>
		root.render(<DeviceAccessDialog scope={scope} onClose={() => {}} />),
	);
	const input = document.querySelector('input[type="file"]');
	await act(async () => {
		Object.defineProperty(input, "files", {
			configurable: true,
			value: [
				{
					size: 100,
					text: async () =>
						JSON.stringify({
							version: 1,
							receipt,
							owner_controller_key: ownerKey,
						}),
				},
			],
		});
		input?.dispatchEvent(new Event("change", { bubbles: true }));
	});
	await act(async () => new Promise((resolve) => setTimeout(resolve, 10)));
}
function button(name: string): HTMLButtonElement {
	const found = [...document.querySelectorAll("button")].find(
		(item) => item.textContent?.trim() === name,
	);
	if (!found) throw new Error(`Missing button ${name}`);
	return found as unknown as HTMLButtonElement;
}

test("a lost access request is recreated from its saved public controller without a new vault", async () => {
	existing = vault({});
	await importBundle();
	expect(button("Create access request").disabled).toBe(true);
	await act(async () => button("Recreate the access request").click());
	const link = [...document.querySelectorAll("a")].find(
		(item) => item.textContent?.trim() === "Download public access request",
	);
	expect(link).toBeDefined();
	const request = await (await fetch(link?.getAttribute("href") ?? "")).json();
	expect(request).toEqual([
		{
			user_id: "reader",
			controller_key: sharedKey,
			grant_id: "lost-request-grant",
		},
	]);
	expect(created).toHaveLength(0);
});

test("keys for another owner or role block new requests with an explicit reason", async () => {
	existing = vault({
		grantId: "owner",
		ownerControllerKey: undefined,
		invitationVault: new Uint8Array(80),
	});
	await importBundle();
	expect(document.body.textContent).toContain(
		"This app already holds other keys for this device",
	);
	expect(button("Create access request").disabled).toBe(true);
	expect(document.body.textContent).not.toContain(
		"Recreate the access request",
	);
});
