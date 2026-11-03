import { afterAll, beforeAll, describe, expect, mock, test } from "bun:test";
import { act, createElement } from "react";
import {
	fakeI18n,
	installDomGlobals,
	restoreDomGlobals,
	translate,
	domWindow as window,
} from "./testing/dom-harness";

const actualLocales = { ...(await import("@flow-like/locales")) };
mock.module("@flow-like/locales", () => ({
	...actualLocales,
	useTranslation: () => ({ t: translate, i18n: fakeI18n }),
}));

const actualBots = { ...(await import("../../hooks/use-bots")) };
mock.module("../../hooks/use-bots", () => ({
	...actualBots,
	useBotApps: () => ({
		data: [
			{ app_id: "app-1", added_at: "", owner_role: null },
			{ app_id: "app-2", added_at: "", owner_role: null },
		],
	}),
}));

const actualRestrictions = { ...(await import("./bot-app-restrictions")) };
mock.module("./bot-app-restrictions", () => ({
	...actualRestrictions,
	useAppNames: () =>
		new Map([
			["app-1", "Sales"],
			["app-2", "Ops"],
		]),
}));

const { createRoot } = await import("react-dom/client");
const { RoomLimitsDialog } = await import("./room-limits-dialog");
const { PERMISSION_BITS } = await import("./room-limits-model");

let root: ReturnType<typeof createRoot>;
let container: HTMLElement;

beforeAll(() => installDomGlobals());
afterAll(() => {
	mock.module("@flow-like/locales", () => actualLocales);
	mock.module("../../hooks/use-bots", () => actualBots);
	mock.module("./bot-app-restrictions", () => actualRestrictions);
	restoreDomGlobals();
});

async function show(onConfirm: (limits: unknown) => void, initial?: unknown) {
	container = window.document.createElement("div");
	window.document.body.appendChild(container);
	root = createRoot(container as unknown as Element);
	await act(async () =>
		root.render(
			createElement(RoomLimitsDialog, {
				open: true,
				onOpenChange: () => {},
				botId: "bot-1",
				botName: "Scout",
				roomName: "Launch",
				initial: initial as never,
				confirmLabel: "Accept and add",
				onConfirm,
			}),
		),
	);
}
const body = () => window.document.body;
const find = (selector: string) =>
	body().querySelector(selector) as unknown as HTMLElement;
const click = (element: Element | null | undefined) =>
	act(async () => (element as HTMLElement).click());
const confirmButton = () =>
	[...body().querySelectorAll("button")].find((b) =>
		b.textContent?.includes("Accept and add"),
	);

describe("the room limits dialog", () => {
	test("untouched, accepting adds no limit", async () => {
		const onConfirm = mock((_limits: unknown) => {});
		await show(onConfirm);
		expect(body().textContent).toContain("Scout in Launch");
		await click(confirmButton());
		expect(onConfirm).toHaveBeenCalledWith(null);
		await act(async () => root.unmount());
	});

	test("switching a permission on widens the mask by exactly that bit", async () => {
		const onConfirm = mock((_limits: unknown) => {});
		await show(onConfirm, { mask: PERMISSION_BITS.read_files });
		await click(
			[...body().querySelectorAll("button")].find((b) =>
				b.textContent?.includes("Advanced"),
			),
		);
		await click(find('button[role="switch"][aria-label="Call events"]'));
		await click(confirmButton());
		expect(onConfirm).toHaveBeenCalledWith({
			mask: PERMISSION_BITS.read_files | PERMISSION_BITS.execute_events,
		});
		await act(async () => root.unmount());
	});

	test("unticking a project limits the bot to the others", async () => {
		const onConfirm = mock((_limits: unknown) => {});
		await show(onConfirm);
		const boxes = body().querySelectorAll('button[role="checkbox"]');
		expect(boxes.length).toBe(2);
		await click(boxes[0]);
		await click(confirmButton());
		expect(onConfirm).toHaveBeenCalledWith({ apps: ["app-2"] });
		await act(async () => root.unmount());
	});

	test("an earlier limit is shown and kept when saving unchanged", async () => {
		const onConfirm = mock((_limits: unknown) => {});
		const initial = { mask: PERMISSION_BITS.read_files, apps: ["app-1"] };
		await show(onConfirm, initial);
		await click(confirmButton());
		expect(onConfirm).toHaveBeenCalledWith(initial);
		await act(async () => root.unmount());
	});
});
