import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import type { LocalDeviceVault } from "../../../lib/device-management/storage";
import type { IProfile } from "../../../types";

const window = new Window({ url: "https://app.test" });
Object.assign(window, { SyntaxError, TypeError, Error });
Object.assign(globalThis, {
	window,
	document: window.document,
	navigator: window.navigator,
	HTMLElement: window.HTMLElement,
	HTMLInputElement: window.HTMLInputElement,
	Element: window.Element,
	Node: window.Node,
	MutationObserver: window.MutationObserver,
	Event: window.Event,
	IS_REACT_ACT_ENVIRONMENT: true,
});
const api = {};
mock.module("../../../state/backend-state", () => ({
	useBackend: () => ({ apiState: api }),
	useBackendReady: () => true,
}));
function deferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (error: Error) => void;
	const promise = new Promise<T>((yes, no) => {
		resolve = yes;
		reject = no;
	});
	return { promise, resolve, reject };
}
let saves: {
	signal: AbortSignal;
	password: string;
	request: ReturnType<typeof deferred<number>>;
}[] = [];
let restores: {
	signal: AbortSignal;
	password: string;
	scope: { account: string };
	request: ReturnType<typeof deferred<LocalDeviceVault>>;
}[] = [];
mock.module("../../../lib/device-management/recovery", () => ({
	saveAccountRecovery: (input: { signal: AbortSignal; password: string }) => {
		const request = deferred<number>();
		saves.push({ ...input, request });
		return request.promise;
	},
	restoreAccountRecovery: (input: {
		signal: AbortSignal;
		password: string;
		scope: { account: string };
	}) => {
		const request = deferred<LocalDeviceVault>();
		restores.push({ ...input, request });
		return request.promise;
	},
}));
const { createRoot } = await import("react-dom/client");
const { DeviceAccountRecovery } = await import("./device-account-recovery");
const profile = { id: "profile" } as IProfile;
const scope = {
	issuer: "issuer",
	account: "account",
	apiOrigin: "https://api.test",
	profileId: "profile",
};
const record = { deviceId: "device" } as LocalDeviceVault;
let cleanup: (() => Promise<void>) | undefined;
afterEach(async () => {
	await cleanup?.();
	cleanup = undefined;
	saves = [];
	restores = [];
});
afterAll(async () => {
	mock.restore();
	await window.happyDOM.close();
});
async function fixture(local?: LocalDeviceVault) {
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	const restored: LocalDeviceVault[] = [];
	const render = async (account = "account") => {
		await act(async () =>
			root.render(
				<DeviceAccountRecovery
					profile={profile}
					scope={{ ...scope, account }}
					deviceId="device"
					record={local}
					disabled={false}
					onRestored={(value) => restored.push(value)}
				/>,
			),
		);
	};
	await render();
	cleanup = async () => {
		await act(async () => root.unmount());
		container.remove();
	};
	const fill = async () => {
		await act(async () => {
			const input = container.querySelector<HTMLInputElement>("input");
			if (!input) throw new Error("Password input missing");
			Object.getOwnPropertyDescriptor(
				window.HTMLInputElement.prototype,
				"value",
			)?.set?.call(input, "device recovery password");
			input.dispatchEvent(new Event("input", { bubbles: true }));
		});
	};
	const submit = async () => {
		await act(async () =>
			container
				.querySelector("form")
				?.dispatchEvent(
					new Event("submit", { bubbles: true, cancelable: true }),
				),
		);
	};
	const input = () => container.querySelector<HTMLInputElement>("input");
	const alert = () =>
		container.querySelector('[role="alert"]')?.textContent ?? null;
	return { container, render, fill, submit, restored, input, alert };
}
test("saving clears the password immediately, blocks duplicate submits, and reports the committed revision", async () => {
	const f = await fixture(record);
	await f.fill();
	await f.submit();
	expect(f.input()?.value).toBe("");
	expect(f.input()?.disabled).toBe(true);
	expect(saves).toHaveLength(1);
	expect(saves[0].password).toBe("device recovery password");
	expect(saves[0].signal.aborted).toBe(false);
	await f.submit();
	expect(saves).toHaveLength(1);
	await act(async () => saves[0].request.resolve(4));
	expect(f.container.textContent).toContain("backup saved, revision 4");
	expect(f.input()?.disabled).toBe(false);
	expect(f.input()?.value).toBe("");
});
test("restoring clears the password immediately and reports only the completed account result", async () => {
	const f = await fixture();
	await f.fill();
	await f.submit();
	expect(restores).toHaveLength(1);
	expect(restores[0].password).toBe("device recovery password");
	expect(f.container.querySelector<HTMLInputElement>("input")?.value).toBe("");
	expect(f.restored).toHaveLength(0);
	await act(async () => restores[0].request.resolve(record));
	expect(f.restored).toEqual([record]);
	expect(f.container.textContent).toContain("Device keys restored");
	expect(f.input()?.value).toBe("");
});
for (const action of ["save", "restore"] as const) {
	test(`${action} errors stay visible without retaining the submitted password`, async () => {
		const f = await fixture(action === "save" ? record : undefined);
		await f.fill();
		await f.submit();
		const request = action === "save" ? saves[0].request : restores[0].request;
		await act(async () =>
			request.reject(new Error("Encrypted backup could not be verified.")),
		);
		expect(f.alert()).toContain("could not be verified");
		expect(f.input()?.value).toBe("");
		expect(f.input()?.disabled).toBe(false);
		expect(f.restored).toEqual([]);
		expect(f.container.textContent).not.toContain("backup saved,");
	});
}
for (const completion of ["success", "failure"] as const) {
	test(`changing accounts ignores stale ${completion} without unlocking the next account's pending form`, async () => {
		const f = await fixture();
		await f.fill();
		await f.submit();
		await f.render("another-account");
		expect(restores[0].signal.aborted).toBe(true);
		expect(f.input()?.value).toBe("");
		expect(f.input()?.disabled).toBe(false);
		await f.fill();
		await f.submit();
		expect(restores[1].scope.account).toBe("another-account");
		await act(async () => {
			if (completion === "success") restores[0].request.resolve(record);
			else
				restores[0].request.reject(
					new Error("Old account failure must stay hidden."),
				);
		});
		expect(f.input()?.disabled).toBe(true);
		expect(f.alert()).toBeNull();
		expect(f.restored).toEqual([]);
		const restored = { ...record, deviceId: "new-account-device" };
		await act(async () => restores[1].request.resolve(restored));
		expect(f.restored).toEqual([restored]);
		expect(f.input()?.disabled).toBe(false);
	});
}
test("unmount aborts recovery and ignores a late successful restore", async () => {
	const f = await fixture();
	await f.fill();
	await f.submit();
	await cleanup?.();
	cleanup = undefined;
	expect(restores[0].signal.aborted).toBe(true);
	await act(async () => restores[0].request.resolve(record));
	expect(f.restored).toEqual([]);
});
test("unmount aborts pending backup and stale failure cannot reach another account", async () => {
	const f = await fixture(record);
	await f.fill();
	await f.submit();
	expect(saves).toHaveLength(1);
	await cleanup?.();
	cleanup = undefined;
	expect(saves[0].signal.aborted).toBe(true);
	await act(async () => saves[0].request.reject(new Error("old failure")));
	expect(f.restored).toHaveLength(0);
});
