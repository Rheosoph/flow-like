// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

const state = vi.hoisted(() => ({
	data: {
		status: "focus",
		source: "manual",
		until: Date.now() + 60_000,
		preferences: { allow_mentions: true, allow_calls: true },
		calendar_links: [],
	},
	update: vi.fn(async () => {}),
	clear: vi.fn(async () => {}),
	clearDevice: vi.fn(async () => {}),
}));
vi.mock("../../hooks/use-user-availability", () => ({
	useUserAvailability: (options?: { local?: boolean }) => ({
		data: state.data,
		scope: "account",
		update: state.update,
		clear: options?.local ? state.clearDevice : state.clear,
		isPending: false,
		isError: false,
		isLocal: false,
	}),
}));
vi.mock("./user-calendar-links", () => ({
	UserCalendarLinks: () => createElement("div", null, "Calendar settings"),
}));
vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({
		t: (_key: string, fallback: string, values?: Record<string, unknown>) =>
			fallback.replace(/\{\{(\w+)\}\}/g, (_, key) =>
				String(values?.[key] ?? key),
			),
	}),
}));
const { AvailabilitySettings } = await import("./availability-settings");
let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
beforeEach(async () => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	state.update.mockClear();
	state.clear.mockClear();
	state.clearDevice.mockClear();
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
	await act(async () => {
		root.render(createElement(AvailabilitySettings));
	});
});
afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
});
test("offers one hour by default and durations no longer than four hours", async () => {
	const duration = [...container.querySelectorAll("select")].find((select) =>
		select.parentElement?.textContent?.includes("Duration"),
	);
	if (!duration) throw new Error("Duration select missing");
	expect(duration.value).toBe("60");
	expect(
		Math.max(...[...duration.options].map((option) => Number(option.value))),
	).toBe(240);
	const set = [...container.querySelectorAll("button")].find(
		(button) => button.textContent === "Set status",
	);
	if (!set) throw new Error("Set status button missing");
	await act(async () => set.click());
	expect(state.update).toHaveBeenCalledWith({
		status: "focus",
		duration_minutes: 60,
	});
});
test("changing a focus exception preserves the other preference", async () => {
	const calls = [
		...container.querySelectorAll<HTMLInputElement>('input[type="checkbox"]'),
	].find((input) =>
		input.parentElement?.textContent?.includes("Incoming calls"),
	);
	if (!calls) throw new Error("Incoming calls checkbox missing");
	await act(async () => calls.click());
	expect(state.update).toHaveBeenCalledWith({
		preferences: { allow_mentions: true, allow_calls: false },
	});
});
test("clearing an override uses the dedicated action and keeps calendar controls", async () => {
	const clear = [...container.querySelectorAll("button")].find(
		(button) => button.textContent === "Clear override",
	);
	if (!clear) throw new Error("Clear override button missing");
	await act(async () => clear.click());
	expect(state.clear).toHaveBeenCalledOnce();
	expect(state.update).not.toHaveBeenCalled();
	expect(container.textContent).toContain("Calendar settings");
});

test("device-only focus is visible and clears separately from the account override", async () => {
	const clear = [...container.querySelectorAll("button")].find(
		(button) => button.textContent === "Clear device status",
	);
	if (!clear) throw new Error("Clear device status button missing");
	expect(container.textContent).toContain("does not change your shared status");
	await act(async () => clear.click());
	expect(state.clearDevice).toHaveBeenCalledOnce();
	expect(state.clear).not.toHaveBeenCalled();
});
