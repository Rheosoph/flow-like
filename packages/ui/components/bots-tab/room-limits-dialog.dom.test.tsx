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

const isolated = {
	use_owner_memory: false,
	use_shared_memory: false,
	share_memory: false,
};

async function show(
	onConfirm: (limits: unknown) => void,
	initial?: unknown,
	group = true,
) {
	container = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	root = createRoot(container);
	await act(async () =>
		root.render(
			createElement(RoomLimitsDialog, {
				open: true,
				onOpenChange: () => {},
				botId: "bot-1",
				group,
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
const click = (element: unknown) =>
	act(async () => (element as HTMLElement).click());
const confirmButton = () =>
	[...body().querySelectorAll("button")].find((b) =>
		b.textContent?.includes("Accept and add"),
	);

describe("the room limits dialog", () => {
	test("untouched, accepting keeps all memory within the group", async () => {
		const onConfirm = mock((_limits: unknown) => {});
		await show(onConfirm);
		expect(body().textContent).toContain("Scout in Launch");
		await click(confirmButton());
		expect(onConfirm).toHaveBeenCalledWith(isolated);
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
			...isolated,
			mask: PERMISSION_BITS.read_files | PERMISSION_BITS.execute_events,
		});
		await act(async () => root.unmount());
	});

	test("unticking a project limits the bot to the others", async () => {
		const onConfirm = mock((_limits: unknown) => {});
		await show(onConfirm);
		const boxes = body().querySelectorAll('button[role="checkbox"]');
		expect(boxes.length).toBe(3);
		await click(boxes[1]);
		await click(confirmButton());
		expect(onConfirm).toHaveBeenCalledWith({ ...isolated, apps: ["app-2"] });
		await act(async () => root.unmount());
	});

	test("ticking answer-only-when-mentioned stores that limit", async () => {
		const onConfirm = mock((_limits: unknown) => {});
		await show(onConfirm);
		expect(body().textContent).toContain("Answer only when mentioned");
		await click(body().querySelector('button[role="checkbox"]'));
		await click(confirmButton());
		expect(onConfirm).toHaveBeenCalledWith({
			...isolated,
			mentioned_only: true,
		});
		await act(async () => root.unmount());
	});

	test("an earlier limit is shown and kept when saving unchanged", async () => {
		const onConfirm = mock((_limits: unknown) => {});
		const initial = { mask: PERMISSION_BITS.read_files, apps: ["app-1"] };
		await show(onConfirm, initial);
		await click(confirmButton());
		expect(onConfirm).toHaveBeenCalledWith({ ...isolated, ...initial });
		await act(async () => root.unmount());
	});
});

describe("group memory controls", () => {
	test("recall and contribution switches are independent and explain the result", async () => {
		const onConfirm = mock((_limits: unknown) => {});
		await show(onConfirm);
		const owner = find('button[role="switch"][aria-label="Use 1:1 memory"]');
		const shared = find(
			'button[role="switch"][aria-label="Use shared memory"]',
		);
		const contribute = find(
			'button[role="switch"][aria-label="Contribute to shared memory"]',
		);
		for (const control of [owner, shared, contribute])
			expect(control.getAttribute("aria-checked")).toBe("false");
		expect(body().textContent).toContain(
			"What this bot learns here stays in this group.",
		);
		await click(shared);
		expect(owner.getAttribute("aria-checked")).toBe("false");
		expect(contribute.getAttribute("aria-checked")).toBe("false");
		await click(contribute);
		expect(body().querySelector('[aria-live="polite"]')?.textContent).toContain(
			"available in your 1:1",
		);
		await click(confirmButton());
		expect(onConfirm).toHaveBeenCalledWith({
			...isolated,
			use_shared_memory: true,
			share_memory: true,
		});
		await act(async () => root.unmount());
	});
	test("an enabled switch can be revoked without changing the other choices", async () => {
		const onConfirm = mock((_limits: unknown) => {});
		await show(onConfirm, {
			use_owner_memory: true,
			use_shared_memory: true,
			share_memory: true,
		});
		await click(
			find('button[role="switch"][aria-label="Contribute to shared memory"]'),
		);
		await click(confirmButton());
		expect(onConfirm).toHaveBeenCalledWith({
			use_owner_memory: true,
			use_shared_memory: true,
			share_memory: false,
		});
		await act(async () => root.unmount());
	});
	test("a direct message request has no group memory or mention controls", async () => {
		await show(() => {}, undefined, false);
		expect(body().textContent).toContain("Scout with Launch");
		expect(body().textContent).not.toContain("Knowledge and memory");
		expect(body().textContent).not.toContain("Answer only when mentioned");
		await act(async () => root.unmount());
	});
});
