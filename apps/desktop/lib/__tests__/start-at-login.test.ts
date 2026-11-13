import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	info: vi.fn(),
	success: vi.fn(),
	error: vi.fn(),
	mobile: false,
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("sonner", () => ({
	toast: { info: mocks.info, success: mocks.success, error: mocks.error },
}));
vi.mock("@flow-like/locales", () => ({
	i18n: { t: (_key: string, fallback: string) => fallback },
}));
vi.mock("../platform", () => ({
	isTauriRuntime: () => true,
	isMobileDevice: () => mocks.mobile,
}));

type StartAtLogin = typeof import("../start-at-login");

async function freshModule(): Promise<StartAtLogin> {
	vi.resetModules();
	return import("../start-at-login");
}

async function settle(): Promise<void> {
	await new Promise((resolve) => setTimeout(resolve, 0));
}

function claimAnswers(due: boolean, enabledAfterSet = true): void {
	mocks.invoke.mockImplementation(async (command: string) => {
		if (command === "claim_start_at_login_offer") return due;
		if (command === "set_start_at_login") return enabledAfterSet;
		throw new Error(`unexpected command ${command}`);
	});
}

describe("start at login offer", () => {
	beforeEach(() => {
		mocks.invoke.mockReset();
		mocks.info.mockReset();
		mocks.success.mockReset();
		mocks.error.mockReset();
		mocks.mobile = false;
	});

	test("offers once when the host claims the offer, and the action turns it on", async () => {
		claimAnswers(true);
		const { offerStartAtLogin } = await freshModule();

		offerStartAtLogin(async () => true);
		await settle();

		expect(mocks.invoke).toHaveBeenCalledWith("claim_start_at_login_offer");
		expect(mocks.info).toHaveBeenCalledTimes(1);
		const options = mocks.info.mock.calls[0][1];
		options.action.onClick();
		await settle();
		expect(mocks.invoke).toHaveBeenCalledWith("set_start_at_login", {
			enabled: true,
		});
		expect(mocks.success).toHaveBeenCalledTimes(1);

		offerStartAtLogin(async () => true);
		await settle();
		expect(mocks.info).toHaveBeenCalledTimes(1);
		expect(
			mocks.invoke.mock.calls.filter(
				([command]) => command === "claim_start_at_login_offer",
			),
		).toHaveLength(1);
	});

	test("shows nothing when the host says the offer was made or login start is on", async () => {
		claimAnswers(false);
		const { offerStartAtLogin } = await freshModule();

		offerStartAtLogin(async () => true);
		await settle();

		expect(mocks.invoke).toHaveBeenCalledWith("claim_start_at_login_offer");
		expect(mocks.info).not.toHaveBeenCalled();
	});

	test("a bot hosted elsewhere neither claims nor spends the offer", async () => {
		claimAnswers(true);
		const { offerStartAtLogin } = await freshModule();

		offerStartAtLogin(async () => false);
		await settle();
		expect(mocks.invoke).not.toHaveBeenCalled();

		offerStartAtLogin(async () => true);
		await settle();
		expect(mocks.info).toHaveBeenCalledTimes(1);
	});

	test("never asks on a phone", async () => {
		claimAnswers(true);
		mocks.mobile = true;
		const hostedHere = vi.fn(async () => true);
		const { offerStartAtLogin } = await freshModule();

		offerStartAtLogin(hostedHere);
		await settle();

		expect(hostedHere).not.toHaveBeenCalled();
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("a failing lookup is logged, not thrown", async () => {
		const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
		const { offerStartAtLogin } = await freshModule();

		offerStartAtLogin(async () => {
			throw new Error("hub offline");
		});
		await settle();

		expect(warn).toHaveBeenCalled();
		expect(mocks.info).not.toHaveBeenCalled();
		warn.mockRestore();
	});
});
