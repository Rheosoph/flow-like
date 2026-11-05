import { afterAll, afterEach, beforeAll, expect, mock, test } from "bun:test";
import { act, createElement } from "react";
import { ApiResponseError } from "../../lib/api-error";
import {
	EMPTY_MONTHLY_LIMITS,
	type MonthlyUsageLimits,
} from "../../lib/monthly-usage";
import {
	fakeI18n,
	installDomGlobals,
	restoreDomGlobals,
	translate,
	domWindow as window,
} from "../bots-tab/testing/dom-harness";

const actualLocales = { ...(await import("@flow-like/locales")) };
mock.module("@flow-like/locales", () => ({
	...actualLocales,
	useTranslation: () => ({ t: translate, i18n: fakeI18n }),
}));
const { createRoot } = await import("react-dom/client");
const { MonthlyUsageLimitsEditor } = await import("./monthly-usage-budget");
let root: ReturnType<typeof createRoot> | undefined;
let container: HTMLElement;

beforeAll(() => installDomGlobals());
afterEach(async () => {
	if (root) await act(async () => root?.unmount());
	root = undefined;
	container?.remove();
});
afterAll(() => {
	mock.module("@flow-like/locales", () => actualLocales);
	restoreDomGlobals();
});

async function show(
	onSave: (limits: MonthlyUsageLimits) => Promise<unknown>,
	ceiling?: MonthlyUsageLimits,
) {
	container = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	root = createRoot(container);
	await act(async () =>
		root?.render(
			createElement(MonthlyUsageLimitsEditor, {
				initial: EMPTY_MONTHLY_LIMITS,
				onSave,
				ceiling,
			}),
		),
	);
}

async function input(index: number, value: string) {
	const element = container.querySelectorAll("input")[index];
	const setter = Object.getOwnPropertyDescriptor(
		window.HTMLInputElement.prototype,
		"value",
	)?.set;
	await act(async () => {
		setter?.call(element, value);
		element.dispatchEvent(
			new window.Event("input", { bubbles: true }) as unknown as Event,
		);
	});
}

async function submit() {
	await act(async () => {
		container.querySelector("form")?.dispatchEvent(
			new window.Event("submit", {
				bubbles: true,
				cancelable: true,
			}) as unknown as Event,
		);
	});
}

test("owner saves converted EUR and seconds while preserving a zero call stop", async () => {
	const onSave = mock(async (_limits: MonthlyUsageLimits) => {});
	await show(onSave);
	await input(0, "12.50");
	await input(1, "0");
	await input(2, "1.5");
	await submit();
	expect(onSave).toHaveBeenCalledWith({
		aiCostMicros: 12_500_000,
		aiCalls: 0,
		runtimeMs: 1_500,
	});
	expect(container.textContent).toContain("Monthly limits saved.");
});

test("raising an owner limit above the enterprise ceiling stays local and names the required approval", async () => {
	const onSave = mock(async (_limits: MonthlyUsageLimits) => {});
	await show(onSave, { ...EMPTY_MONTHLY_LIMITS, aiCalls: 100 });
	await input(1, "101");
	await submit();
	expect(onSave).not.toHaveBeenCalled();
	expect(container.querySelector('[role="alert"]')?.textContent).toContain(
		"administrator must approve",
	);
});

test("inheriting a ceiling sends null and displays an authoritative refusal without a success message", async () => {
	const onSave = mock(async (_limits: MonthlyUsageLimits) => {
		throw new ApiResponseError({
			status: 403,
			message: "Only the bot owner can change this budget.",
		});
	});
	await show(onSave);
	await submit();
	expect(onSave).toHaveBeenCalledWith(EMPTY_MONTHLY_LIMITS);
	expect(container.querySelector('[role="alert"]')?.textContent).toContain(
		"Only the bot owner",
	);
	expect(container.textContent).not.toContain("Monthly limits saved.");
});

test("invalid fractional call counts are rejected before sending", async () => {
	const onSave = mock(async (_limits: MonthlyUsageLimits) => {});
	await show(onSave);
	await input(1, "1.5");
	await submit();
	expect(onSave).not.toHaveBeenCalled();
	expect(container.querySelector('[role="alert"]')).not.toBeNull();
});
