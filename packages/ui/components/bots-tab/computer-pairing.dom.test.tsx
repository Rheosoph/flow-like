import { afterAll, beforeAll, describe, expect, mock, test } from "bun:test";
import { act, createElement } from "react";
import {
	fakeI18n,
	installDomGlobals,
	restoreDomGlobals,
	translate,
	domWindow as window,
} from "./testing/dom-harness";

const actual = { locales: { ...(await import("@flow-like/locales")) } };
mock.module("@flow-like/locales", () => ({
	...actual.locales,
	useTranslation: () => ({ t: translate, i18n: fakeI18n }),
}));
const scannerPath = "../interfaces/chat-default/chatbox/barcode-scanner-dialog";
const actualScanner = { ...(await import(scannerPath)) };
mock.module(scannerPath, () => ({
	...actualScanner,
	BarcodeScannerDialog: ({
		open,
		onCapture,
	}: {
		open: boolean;
		onCapture: (capture: unknown) => void;
	}) =>
		open
			? createElement(
					"button",
					{
						type: "button",
						onClick: () =>
							onCapture({
								photo: null,
								barcodes: [
									{
										text: "https://example.com",
										format: "qr_code",
										points: [],
									},
									{
										text: "flowlike://pair-screen?v=1",
										format: "qr_code",
										points: [],
									},
								],
							}),
					},
					"shutter",
				)
			: null,
}));

const { createRoot } = await import("react-dom/client");
const { ComputerPairingStage } = await import("./computer-pairing");

let container: HTMLElement;
let root: ReturnType<typeof createRoot>;

beforeAll(() => {
	installDomGlobals();
});
afterAll(() => {
	mock.module("@flow-like/locales", () => actual.locales);
	mock.module(scannerPath, () => actualScanner);
	restoreDomGlobals();
});

async function show(props: Record<string, unknown>) {
	container = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	root = createRoot(container);
	await act(async () => {
		root.render(
			createElement(ComputerPairingStage, {
				name: "Scout",
				onPair: async () => {},
				...props,
			} as never),
		);
	});
}

async function hide() {
	await act(async () => root.unmount());
	container.remove();
}

const buttons = () =>
	[...container.querySelectorAll("button")].map((b) => b.textContent?.trim());

async function typeLink(value: string) {
	const input = container.querySelector("input") as HTMLInputElement;
	await act(async () => {
		const setter = Object.getOwnPropertyDescriptor(
			window.HTMLInputElement.prototype,
			"value",
		)?.set;
		setter?.call(input, value);
		input.dispatchEvent(new window.Event("input", { bubbles: true }) as never);
	});
}

describe("the unpaired stage", () => {
	test("names the host and hides Scan without a detector", async () => {
		await show({ hostLabel: "Studio Mac" });
		expect(container.textContent).toContain("Pair this device with Studio Mac");
		expect(buttons()).toEqual(["Pair"]);
		await hide();
	});

	test("falls back to the bot when no host label is known and offers Scan with a detector", async () => {
		await show({ detect: async () => [] });
		expect(container.textContent).toContain(
			"Pair this device with the computer that runs Scout",
		);
		expect(buttons()).toEqual(["Scan QR code", "Pair"]);
		await hide();
	});

	test("pastes a link and pairs; the button waits for text", async () => {
		const onPair = mock(async () => {});
		await show({ onPair });
		const pair = container.querySelector("form button") as HTMLButtonElement;
		expect(pair.disabled).toBe(true);
		await typeLink("flowlike://pair-screen?v=1&h=x");
		await act(async () => {
			container.querySelector("form")?.dispatchEvent(
				new window.Event("submit", {
					bubbles: true,
					cancelable: true,
				}) as never,
			);
		});
		expect(onPair).toHaveBeenCalledWith("flowlike://pair-screen?v=1&h=x");
		await hide();
	});

	test("shows the error of a failed pairing inline", async () => {
		await show({
			onPair: async () => {
				throw new Error("The desktop app did not answer the pairing");
			},
		});
		await typeLink("flowlike://pair-screen?v=1");
		await act(async () => {
			container.querySelector("form")?.dispatchEvent(
				new window.Event("submit", {
					bubbles: true,
					cancelable: true,
				}) as never,
			);
		});
		expect(container.querySelector('[role="alert"]')?.textContent).toContain(
			"did not answer the pairing",
		);
		await hide();
	});

	test("a scanned photo pairs with the pairing link among its codes", async () => {
		const onPair = mock(async () => {});
		await show({ detect: async () => [], onPair });
		await act(async () =>
			(container.querySelector("button") as HTMLElement).click(),
		);
		const shutter = [...container.querySelectorAll("button")].find(
			(b) => b.textContent === "shutter",
		) as HTMLElement;
		await act(async () => shutter.click());
		expect(onPair).toHaveBeenCalledWith("flowlike://pair-screen?v=1");
		await hide();
	});
});
