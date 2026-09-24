import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { type ReactNode, act } from "react";
import type { LocalDeviceVault } from "../../../lib/device-management/storage";

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
const scope = {
	issuer: "issuer",
	account: "reader",
	apiOrigin: "https://hub.test",
	profileId: "profile",
};
const key = { kty: "OKP" as const, crv: "Ed25519" as const, x: "controller" };
const record: LocalDeviceVault = {
	deviceId: "device",
	grantId: "owner",
	manifestJws: "signed-manifest",
	controllerPublic: {
		device_id: "device",
		endpoint_id: "endpoint",
		controller_key: key,
		archive_key: Array(32).fill(1),
		telemetry_member: { endpoint_id: "endpoint", signing_key: key },
	},
	controllerVault: new Uint8Array(80).fill(2),
	invitationVault: new Uint8Array(80).fill(3),
};
const next = {
	...record,
	controllerVault: new Uint8Array(80).fill(4),
	invitationVault: new Uint8Array(80).fill(5),
};
let outcome: Promise<LocalDeviceVault>;
let calls: unknown[][] = [];
mock.module("../../../lib/device-management/crypto", () => ({
	loadDeviceCrypto: async () => ({}),
}));
mock.module("../../../lib/device-management/password", () => ({
	changeDevicePassword: (...args: unknown[]) => {
		calls.push(args);
		return outcome;
	},
}));
const { createRoot } = await import("react-dom/client");
const { DevicePasswordChange } = await import("./device-password-change");
const container = document.createElement("div");
document.body.append(container);
const root = createRoot(container);
afterEach(async () => {
	await act(async () => root.render(null));
	calls = [];
});
afterAll(async () => {
	await act(async () => root.unmount());
	mock.restore();
	await window.happyDOM.close();
});
async function render(element: ReactNode) {
	await act(async () => root.render(element));
}
const screen = {
	queryByRole(role: string, options?: { name: string }): HTMLElement | null {
		const selector =
			role === "status"
				? "output"
				: role === "link"
					? "a"
					: role === "button"
						? "button"
						: `[role="${role}"]`;
		return (
			[...container.querySelectorAll<HTMLElement>(selector)].find(
				(item) => !options || item.textContent?.trim() === options.name,
			) ?? null
		);
	},
	getByRole(role: string, options?: { name: string }): HTMLElement {
		const element = this.queryByRole(role, options);
		if (!element) throw new Error(`Missing ${role}`);
		return element;
	},
};
const fireEvent = { click: (element: HTMLElement) => element.click() };
async function fill(confirm = "replacement password") {
	await act(async () => {
		const inputs = container.querySelectorAll<HTMLInputElement>(
			"input[type=password]",
		);
		["previous password", "replacement password", confirm].forEach(
			(value, index) => {
				const input = inputs[index];
				Object.getOwnPropertyDescriptor(
					window.HTMLInputElement.prototype,
					"value",
				)?.set?.call(input, value);
				input?.dispatchEvent(new Event("input", { bubbles: true }));
			},
		);
	});
}
function emptyInputs() {
	for (const input of document.querySelectorAll<HTMLInputElement>(
		"input[type=password]",
	))
		expect(input.value).toBe("");
}

test("mismatched confirmation clears password inputs without changing storage", async () => {
	await render(
		<DevicePasswordChange
			scope={scope}
			record={record}
			disabled={false}
			onChanged={() => {
				throw new Error("unexpected commit");
			}}
		/>,
	);
	await fill("different confirmation");
	await act(async () =>
		fireEvent.click(
			screen.getByRole("button", { name: "Change local password" }),
		),
	);
	expect(screen.getByRole("alert").textContent).toContain("do not match");
	emptyInputs();
	expect(calls).toHaveLength(0);
});

test("commit success offers only the updated encrypted backup and explains old copies", async () => {
	let finish!: (record: LocalDeviceVault) => void;
	outcome = new Promise((resolve) => {
		finish = resolve;
	});
	const committed: LocalDeviceVault[] = [];
	await render(
		<DevicePasswordChange
			scope={scope}
			record={record}
			disabled={false}
			onChanged={(value) => committed.push(value)}
		/>,
	);
	await fill();
	await act(async () =>
		fireEvent.click(
			screen.getByRole("button", { name: "Change local password" }),
		),
	);
	emptyInputs();
	expect(committed).toHaveLength(0);
	expect(
		screen
			.getByRole("button", { name: "Changing password…" })
			.hasAttribute("disabled"),
	).toBe(true);
	expect(calls[0]?.slice(0, 4)).toEqual([
		scope,
		record,
		"previous password",
		"replacement password",
	]);
	await act(async () => {
		finish(next);
		await outcome;
	});
	expect(committed).toEqual([next]);
	expect(screen.getByRole("status").textContent).toContain("Password changed");
	const backup = screen.getByRole("link", {
		name: "Save updated encrypted backup",
	});
	expect(backup.getAttribute("download")).toBe(
		"flow-like-controller-device.json",
	);
	const saved = await (await fetch(backup.getAttribute("href") ?? "")).json();
	expect(saved.controllerVault).toEqual(Array.from(next.controllerVault));
	expect(saved.invitationVault).toEqual(Array.from(next.invitationVault));
	expect(JSON.stringify(saved)).not.toContain("password");
	expect(document.body.textContent).toContain(
		"Old backups still use the old password",
	);
});

test("failed persistence does not report success or offer an uncommitted backup", async () => {
	let fail!: (error: Error) => void;
	outcome = new Promise((_resolve, reject) => {
		fail = reject;
	});
	const committed: LocalDeviceVault[] = [];
	await render(
		<DevicePasswordChange
			scope={scope}
			record={record}
			disabled={false}
			onChanged={(value) => committed.push(value)}
		/>,
	);
	await fill();
	await act(async () =>
		fireEvent.click(
			screen.getByRole("button", { name: "Change local password" }),
		),
	);
	await act(async () => {
		fail(new Error("Encrypted device storage was not committed."));
		await outcome.catch(() => {});
	});
	expect(screen.getByRole("alert").textContent).toContain("not committed");
	expect(committed).toHaveLength(0);
	expect(screen.queryByRole("link")).toBeNull();
	emptyInputs();
});
