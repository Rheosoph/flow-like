import { afterAll, beforeAll, describe, expect, mock, test } from "bun:test";
import { act, createElement } from "react";
import type { IBotScreenPairing } from "../../state/backend-state/bot-state";
import {
	fakeI18n,
	installDomGlobals,
	restoreDomGlobals,
	settle,
	translate,
	domWindow as window,
} from "./testing/dom-harness";

const actual = { locales: { ...(await import("@flow-like/locales")) } };
mock.module("@flow-like/locales", () => ({
	...actual.locales,
	useTranslation: () => ({ t: translate, i18n: fakeI18n }),
}));

const { createRoot } = await import("react-dom/client");
const { ComputerPairingDialog } = await import("./computer-pairing-dialog");

let container: HTMLElement;
let root: ReturnType<typeof createRoot>;

beforeAll(() => {
	installDomGlobals();
});
afterAll(() => {
	mock.module("@flow-like/locales", () => actual.locales);
	restoreDomGlobals();
});

function fakePairing(overrides: Partial<IBotScreenPairing> = {}) {
	let paired: ((device: { key_id: string; label: string }) => void) | null =
		null;
	let devices = [
		{
			key_id: "ik_1",
			label: "Felix's phone",
			fingerprint: "AB12 CD34",
			paired_at: 1763337600,
		},
	];
	const port: IBotScreenPairing = {
		start: mock(async () => ({
			link: "flowlike://pair-screen?v=1&h=x",
			expires_at: Date.now() / 1000 + 300,
			host_label: "Studio Mac",
		})),
		list: mock(async () => devices),
		remove: mock(async (keyId: string) => {
			devices = devices.filter((device) => device.key_id !== keyId);
		}),
		onPaired: (listener) => {
			paired = listener;
			return () => {
				paired = null;
			};
		},
		...overrides,
	};
	return {
		port,
		firePaired: (label: string) => paired?.({ key_id: "ik_2", label }),
	};
}

async function open(port: IBotScreenPairing) {
	container = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	root = createRoot(container);
	await act(async () => {
		root.render(createElement(ComputerPairingDialog, { pairing: port }));
	});
	await act(async () =>
		(container.querySelector("button") as HTMLElement).click(),
	);
	await settle();
}

const body = () => window.document.body;

describe("the Watch on another device dialog", () => {
	test("shows the code, its countdown and the paired devices", async () => {
		const { port } = fakePairing();
		await open(port);
		expect(body().querySelector('[aria-label^="Pairing code"]')).not.toBeNull();
		expect(body().textContent).toContain("Code valid for 5:00");
		expect(body().textContent).toContain("Felix's phone");
		expect(body().textContent).toContain("AB12 CD34");
		expect(port.start).toHaveBeenCalledTimes(1);
		await act(async () => root.unmount());
	});

	test("removes a device and asks for a new code on request", async () => {
		const { port } = fakePairing();
		await open(port);
		const remove = [...body().querySelectorAll("button")].find(
			(b) => b.getAttribute("aria-label") === "Remove Felix's phone",
		) as unknown as HTMLElement;
		await act(async () => remove.click());
		await settle();
		expect(port.remove).toHaveBeenCalledWith("ik_1");
		expect(body().textContent).toContain("No device is paired yet.");
		const fresh = [...body().querySelectorAll("button")].find((b) =>
			b.textContent?.includes("New code"),
		) as unknown as HTMLElement;
		await act(async () => fresh.click());
		await settle();
		expect(port.start).toHaveBeenCalledTimes(2);
		await act(async () => root.unmount());
	});

	test("announces a completed pairing and fetches a fresh code", async () => {
		const { port, firePaired } = fakePairing();
		await open(port);
		await act(async () => firePaired("Tablet"));
		await settle();
		expect(body().textContent).toContain("Tablet is paired.");
		expect(port.start).toHaveBeenCalledTimes(2);
		await act(async () => root.unmount());
	});

	test("a failing native command shows an inline error instead of crashing", async () => {
		const { port } = fakePairing({
			start: async () => {
				throw "Command bot_screens_pairing_start not found";
			},
		});
		await open(port);
		expect(body().querySelector('[role="alert"]')?.textContent).toContain(
			"bot_screens_pairing_start not found",
		);
		expect(body().querySelector('[aria-label^="Pairing code"]')).toBeNull();
		await act(async () => root.unmount());
	});
});
