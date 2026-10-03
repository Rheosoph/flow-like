import { afterAll, afterEach, describe, expect, test } from "bun:test";
import type { HubErrorCode } from "../../../../lib/device-management/model/types";
import type { MemoryNavigation } from "../routing/use-devices-route";
import {
	byRole,
	byText,
	click,
	installDom,
	queryByRole,
} from "../testing/dom-harness";
import type { AreaGateProps, AreaGateState } from "./area-gate";

const dom = installDom();
const { MemoryDevicesRoute } = await import("../routing/use-devices-route");
const { AreaGate } = await import("./area-gate");

afterEach(dom.cleanup);
afterAll(dom.restore);

const HUB = "api.flow-like.com";

async function mount(props: AreaGateProps) {
	const navigations: MemoryNavigation[] = [];
	const view = await dom.render(
		<MemoryDevicesRoute onNavigate={(entry) => navigations.push(entry)}>
			<AreaGate {...props} />
		</MemoryDevicesRoute>,
	);
	return { navigations, text: view.container.textContent ?? "", view };
}

const primaries = () =>
	dom.document.querySelectorAll("[data-dv-primary]").length;

describe("account gates", () => {
	test("signed out: a sentence and the one primary action", async () => {
		let signIns = 0;
		await mount({ state: { kind: "signed_out" }, onSignIn: () => signIns++ });
		expect(byText("Sign in to manage devices")).toBeTruthy();
		await click(byRole("button", "Sign in"));
		expect(signIns).toBe(1);
		expect(primaries()).toBe(1);
	});

	test("an expired session asks to sign in again", async () => {
		let signIns = 0;
		await mount({
			state: { kind: "session_expired" },
			onSignIn: () => signIns++,
		});
		expect(byText("Your sign-in has expired.")).toBeTruthy();
		await click(byRole("button", "Sign in again"));
		expect(signIns).toBe(1);
	});

	test("a restricted token says what it can't do and how to fix it", async () => {
		const { text } = await mount({ state: { kind: "token" } });
		expect(text).toContain("Your access token can't manage devices.");
		expect(text).toContain("Use a token with full permissions.");
		expect(byRole("button", "Sign in with full permissions")).toBeTruthy();
		expect(primaries()).toBe(0);
	});

	test("the account loads as a skeleton, never as an empty page", async () => {
		const { view } = await mount({ state: { kind: "loading" } });
		const status = byRole("status", "Loading your account…");
		expect(status.getAttribute("aria-busy")).toBe("true");
		expect(view.container.querySelector('[data-kind="empty"]')).toBeNull();
	});

	test("a profile that failed to load can be retried, with a visible result", async () => {
		let retries = 0;
		const { view } = await mount({
			state: { kind: "profile_error" },
			onRetry: async () => {
				retries++;
			},
		});
		expect(byText("Your account profile couldn't be loaded.")).toBeTruthy();
		await click(byRole("button", "Retry"));
		expect(retries).toBe(1);
		expect(view.container.textContent).toMatch(
			/Still couldn't load it at \d{1,2}:\d{2}:\d{2}/,
		);
	});
});

describe("hub gates", () => {
	test("checking shows the hub's name over skeleton rows", async () => {
		await mount({ state: { kind: "checking" }, hub: HUB });
		expect(byText(`Checking ${HUB}…`)).toBeTruthy();
		expect(byRole("status").getAttribute("aria-busy")).toBe("true");
	});

	test("devices off links to Hub status", async () => {
		const { navigations, text } = await mount({
			state: { kind: "off" },
			hub: HUB,
		});
		expect(text).toContain(`Devices aren't enabled on ${HUB}.`);
		expect(text).toContain("Ask the hub operator to turn on device support.");
		const link = byRole("link", "Open hub status");
		expect(link.getAttribute("href")).toBe("/settings/devices?view=hub");
		await click(link);
		expect(navigations).toEqual([
			{ mode: "push", href: "/settings/devices?view=hub" },
		]);
	});

	test("unreachable names the cause and Retry reports the attempt inline", async () => {
		const attempts: number[] = [];
		const { view } = await mount({
			state: { kind: "unreachable", code: "timeout" },
			hub: HUB,
			onRetry: async () => {
				attempts.push(Date.now());
			},
		});
		expect(
			byText(`Can't reach ${HUB}: it didn't answer in time.`),
		).toBeTruthy();
		expect(queryByRole("status")).toBeNull();
		await click(byRole("button", "Retry"));
		expect(attempts).toHaveLength(1);
		const result = view.container.querySelector('[data-result="warning"]');
		expect(result?.textContent).toMatch(
			/^Still unreachable at \d{1,2}:\d{2}:\d{2}$/,
		);
	});

	test("a retry that throws still ends with the inline result", async () => {
		const { view } = await mount({
			state: { kind: "unreachable" },
			hub: HUB,
			onRetry: () => Promise.reject(new Error("connection refused")),
		});
		await click(byRole("button", "Retry"));
		expect(view.container.textContent).toContain("Still unreachable at");
		expect(byRole("button", "Retry").getAttribute("aria-busy")).toBeNull();
	});

	test("every hub error has its own sentence with the hub's name", async () => {
		const codes: HubErrorCode[] = [
			"network",
			"timeout",
			"unauthorized",
			"forbidden",
			"token_restricted",
			"not_found",
			"rate_limited",
			"server_error",
			"invalid_response",
		];
		for (const code of codes) {
			const { view } = await mount({
				state: { kind: "unreachable", code },
				hub: HUB,
			});
			const title = view.container.querySelector("p")?.textContent ?? "";
			expect(title).toContain(HUB);
			expect(title).not.toContain(code);
			await dom.cleanup();
		}
	});

	test("without a known hub the copy says 'this hub'", async () => {
		const { text } = await mount({ state: { kind: "off" } });
		expect(text).toContain("Devices aren't enabled on this hub.");
	});
});

test("no gate shows codes or keys (R3)", async () => {
	const states: AreaGateState[] = [
		{ kind: "loading" },
		{ kind: "signed_out" },
		{ kind: "session_expired" },
		{ kind: "profile_error" },
		{ kind: "token" },
		{ kind: "checking" },
		{ kind: "off" },
		{ kind: "unreachable", code: "rate_limited" },
	];
	for (const state of states) {
		const { text, view } = await mount({ state, hub: HUB });
		expect(text).not.toMatch(/\b[GD]\d{1,2}\b/);
		expect(text).not.toMatch(/\b[a-z]+_[a-z_]+\b/);
		expect(
			view.container
				.querySelector("[data-area-gate]")
				?.getAttribute("data-area-gate"),
		).toBe(state.kind);
		await dom.cleanup();
	}
});
