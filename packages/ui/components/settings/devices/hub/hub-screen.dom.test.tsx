import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { ApiResponseError } from "../../../../lib/api-error";
import {
	sampleFleet,
	sampleFleetOlderAgent,
	sampleFleetOlderHub,
} from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import { generateFleet } from "../../../../lib/device-management/model/__fixtures__/sample-fleet-200";
import { byRole, click, installDom, queryByRole } from "../testing/dom-harness";
import type { MountDevicesOptions } from "../testing/mount-devices";
import {
	DAY_S,
	TEST_RELEASE_URL as RELEASE_URL,
	type TestReleaseOptions,
	publishTestRelease,
} from "./release-test-kit";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { act } = await import("react");
const { fakeDeviceApi } = await import("../testing/fake-device-api");
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { AreaPrefsContext } = await import("../primitives/area-context");
const { useOverlayStore } = await import("../workspace/overlay-store");
const { HubScreen } = await import("./hub-screen");
const { HubLimitsUsage, ReadinessList } = await import("./readiness-list");

let restoreFetch: (() => void) | undefined;
afterEach(async () => {
	restoreFetch?.();
	restoreFetch = undefined;
	await cleanupDevices();
	useOverlayStore.getState().close();
	await dom.cleanup();
});
afterAll(dom.restore);

const CLOCK = String.raw`\d{1,2}:\d{2}:\d{2}`;
const SCREEN = (
	<HubScreen route={{ screen: "hub" }} scope={{ kind: "account" }} />
);

/**
 * The hub serves a release list signed here, valid for a year unless the test
 * chooses otherwise, so no assertion depends on how long the shared fake's own
 * list lives. A test that brings its own fake keeps what it set up; those
 * tests say nothing about the release's validity.
 */
async function mountWith(
	release: TestReleaseOptions | undefined,
	options: MountDevicesOptions = {},
) {
	const fake =
		options.fake ?? (await createFakeWorkspace(options.seed, options));
	const published = options.fake
		? undefined
		: await publishTestRelease(fake, release);
	const view = await mountDevices(SCREEN, { ...options, fake });
	return { view, published };
}

async function mount(options: MountDevicesOptions = {}) {
	return (await mountWith(undefined, options)).view;
}

type View = Awaited<ReturnType<typeof mount>>;

function find(root: ParentNode, selector: string) {
	const node = root.querySelector(selector);
	if (!node) throw new Error(`Nothing matches ${selector}`);
	return node as HTMLElement;
}

function all(root: ParentNode, selector: string) {
	return Array.from(root.querySelectorAll<HTMLElement>(selector));
}

function text(root: ParentNode, selector: string) {
	return String(find(root, selector).textContent);
}

function block(view: View, id: string) {
	return find(view.container, `#${id}`);
}

function headline(view: View) {
	return text(view.container, "[data-headline]");
}

function stateOf(view: View, feature: string) {
	return find(view.container, `[data-feature="${feature}"]`).dataset.state;
}

/**
 * R3: no wire value, condition key or gate code is rendered. Ids, addresses
 * and keys are data and may contain anything, so they are left out.
 */
function expectPlainWords(container: HTMLElement) {
	const copy = container.cloneNode(true) as HTMLElement;
	for (const node of all(copy, "[data-idref], [title^='https://']"))
		node.remove();
	expect(String(copy.textContent)).not.toMatch(
		/[a-z]+_[a-z_]+|\bG\d{1,2}\b|\bD\d{1,2}\b|signaling\b|enrollment|standalone|telemetry/,
	);
}

function readinessWith(failing: string) {
	const seed = sampleFleet();
	const base = seed.readiness;
	if (!base) throw new Error("The sample fleet has readiness checks");
	const checks = base.checks.map((check) =>
		check.id === failing
			? { ...check, ready: false, message: "server text" }
			: check,
	);
	seed.readiness = { version: 1, ready: false, checks };
	return seed;
}

function setupRequests(view: View) {
	return view.fake.api.sent("GET", "devices/setup").length;
}

type Fake = Awaited<ReturnType<typeof createFakeWorkspace>>;

interface FetchApi {
	fetch(profile: unknown, path: string, init?: RequestInit): Promise<unknown>;
}

/** What the hub answers on `GET devices/setup` while device support is off: the limits check fails. */
function checksWhileOff(fake: Fake) {
	const checks = [];
	for (const check of fake.hub.readiness.checks) {
		const off = check.id === "policy";
		checks.push(
			off ? { ...check, ready: false, message: "server text" } : check,
		);
	}
	return { version: 1, ready: false, checks };
}

/** The fake refuses every device route while devices are off; the hub still answers its checks. */
function answerChecksWhileOff(fake: Fake) {
	const api = fake.api as unknown as FetchApi;
	const refused = api.fetch.bind(api);
	api.fetch = function answer(profile, path, init) {
		const route = path.replace(/^\/+/, "").split("?")[0];
		if (route !== "devices/setup" || fake.api.mode.devicesEnabled)
			return refused(profile, path, init);
		fake.api.calls.push(["GET", route]);
		return Promise.resolve(checksWhileOff(fake));
	};
}

function releaseRequests(view: View) {
	return view.fake.api.calls.filter(([, path]) => path === RELEASE_URL).length;
}

function rateLimited() {
	return new ApiResponseError({
		status: 429,
		code: "RATE_LIMITED",
		message: "slow down",
	});
}

/** The hub's own retries of a failed read have finished: nothing is fetching any more. */
async function idle(view: View) {
	for (let round = 0; round < 80; round++) {
		if (!view.fake.queryClient.isFetching()) {
			// The last answer reaches the components a tick after the cache has it.
			await view.settle();
			return;
		}
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 100));
		});
	}
	throw new Error("The hub reads never settled");
}

/** Real time passes, so a ticking area clock reads the fake clock again. */
async function pass(ms: number) {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, ms));
	});
}

/** R7: a gated control is disabled, and clicking it navigates nowhere and sends nothing. */
async function expectGated(view: View, name: string) {
	const control = byRole("button", name);
	expect(control.getAttribute("aria-disabled")).toBe("true");
	const calls = view.fake.api.calls.length;
	await click(control);
	await view.settle();
	expect(view.navigations).toEqual([]);
	expect(view.fake.api.calls.length).toBe(calls);
}

describe("Hub status: a ready hub", () => {
	test("says in one headline that the hub is ready", async () => {
		const view = await mount();
		expect(byRole("heading", "Hub status")).toBeTruthy();
		expect(headline(view)).toContain("hub.test is ready for devices.");
		expect(headline(view)).toContain(
			"All 6 checks pass and the current agent release is verified until",
		);
		expect(all(view.container, "[data-dv-primary]").length).toBe(1);
		expect(byRole("link", "Set up a device").getAttribute("href")).toBe(
			"/settings/devices?flow=setup",
		);
		expectPlainWords(view.container);
		expect(view.container.querySelector('[role="alert"]')).toBeNull();
	});

	test("every block head states its source and age", async () => {
		const view = await mount();
		const ids = [
			"hub",
			"readiness",
			"features",
			"limits",
			"releases",
			"history",
		];
		const sources = ids.map(
			(id) => find(block(view, id), "header [data-stamp]").dataset.src,
		);
		expect(sources).toEqual(ids.map(() => "hub"));
	});

	test("lists the six checks in the viewer's words with the time they ran", async () => {
		const view = await mount();
		const readiness = block(view, "readiness");
		expect(text(readiness, "header")).toContain("6 of 6 pass");
		expect(all(readiness, 'li[data-state="pass"]').length).toBe(6);
		expect(readiness.textContent).toContain("Connection service");
		expect(readiness.textContent).toContain(
			"Secure connection addresses are set.",
		);
		// BG37: the hub's own English sentence ("signaling ready" in the fake) is never shown for a known check.
		expect(readiness.textContent).not.toContain("signaling ready");
		expect(text(readiness, "footer")).toMatch(
			new RegExp(`Checked at ${CLOCK}\\.`),
		);
	});

	test("shows the limits with the hub's own usage counts", async () => {
		const view = await mount();
		const limits = block(view, "limits");
		expect(all(limits, "[data-limit]").length).toBe(4);
		expect(limits.textContent).toContain("You: 6 of 100");
		expect(limits.textContent).toContain("You: 1 of 10");
		expect(limits.textContent).toContain("factory-line-3");
		expect(limits.textContent).toContain("You: 1 of 220");
		expect(limits.textContent).toContain("24hours");
		expect(block(view, "hub").textContent).toContain(
			"includes 1 unused setup package",
		);
	});

	test("shows where releases come from, who signs them and the verified release", async () => {
		const view = await mount();
		const releases = block(view, "releases");
		const shown = String(releases.textContent);
		expect(shown).toContain(RELEASE_URL);
		expect(shown).toContain("Trusted signing keys1");
		expect(shown).toContain("Minimum release number41");
		expect(shown).toContain("0.9.4");
		expect(shown).toContain("release number 44");
		expect(shown).toContain("Verified");
		expect(shown).toContain("Valid until");
		// A year-long release shows its end date, not a day count, and no maximum.
		expect(shown).not.toMatch(/days? left/);
		expect(shown).not.toContain("30 days");
	});

	test("explains the release number, the minimum and the validity in plain words", async () => {
		const view = await mount();
		const shown = String(block(view, "releases").textContent);
		expect(shown).toContain(
			"Release number44Release numbers only go up. Devices refuse a lower one.",
		);
		expect(shown).toContain(
			"Minimum release number41The lowest release number this hub accepts.",
		);
		expect(shown).toContain(
			"For the hub operator: releases 41 to 43 are still accepted. Raise the minimum to 44 so that only the current release is.",
		);
		expect(shown).toMatch(
			/For the hub operator: publish or renew a release before .+\. Running devices are not affected\./,
		);
		expect(shown).toContain(
			"Signed by a trusted key · release number 44 is at or above the minimum 41 · within its validity window",
		);
	});

	test("the operator's note on the minimum appears only while it is behind the release", async () => {
		const level = await mountWith({ minimum: 44 });
		expect(block(level.view, "releases").textContent).not.toContain(
			"still accepted",
		);
		await cleanupDevices();
		const one = await mountWith({ minimum: 43 });
		expect(block(one.view, "releases").textContent).toContain(
			"For the hub operator: release 43 is still accepted. Raise the minimum to 44 so that only the current release is.",
		);
	});

	test("raw field names show only with developer mode on", async () => {
		const plain = await mount();
		expect(plain.container.querySelector("[data-tech]")).toBeNull();
		await cleanupDevices();
		const fake = await createFakeWorkspace();
		await publishTestRelease(fake);
		const tech = await mountDevices(
			<AreaPrefsContext.Provider value={{ showTechnicalKeys: true }}>
				{SCREEN}
			</AreaPrefsContext.Provider>,
			{ fake },
		);
		const names = all(block(tech, "releases"), "[data-tech]").map((node) =>
			String(node.textContent),
		);
		expect(names).toContain("sequence");
		expect(names).toContain("minimum_sequence");
		expect(
			names.some((name) => /^issued_at \d+ · expires_at \d+$/.test(name)),
		).toBe(true);
		expect(names).toContain("state_schema_version 12");
	});

	test("shows the signing key fingerprint with its case kept and copies it as it is", async () => {
		const view = await mount();
		const fingerprint = find(view.container, "[data-fingerprint]");
		const value = find(fingerprint, "button[title]");
		const raw = String(value.getAttribute("title"));
		expect(raw).toMatch(/^[A-Za-z0-9_-]{43}$/);
		const short = String(value.textContent).replace(/[ …]/g, "");
		expect(short.length).toBe(16);
		expect(raw.startsWith(short)).toBe(true);
		await click(value);
		expect(String(value.textContent).replaceAll(" ", "")).toBe(raw);
		await click(byRole("button", "Copy signing key fingerprint", fingerprint));
		expect(dom.clipboard.at(-1)).toBe(raw);
	});

	test("lists the platforms of the release, and the ones it lacks", async () => {
		const view = await mount();
		const releases = block(view, "releases");
		const targets = all(releases, "tr[data-target]");
		expect(releases.textContent).toContain("1 of 4 platforms");
		expect(targets.length).toBe(4);
		expect(targets.filter((row) => row.dataset.dim).length).toBe(3);
		expect(targets[0].textContent).toContain("Linux (Intel/AMD 64-bit)");
		expect(targets[0].textContent).toContain("Run directly");
		expect(targets[1].textContent).toContain("Not in this release");
	});

	test("lists each device's agent with older agents first", async () => {
		const view = await mount();
		const releases = block(view, "releases");
		const rows = all(releases, "[data-device]");
		expect(rows.length).toBe(5);
		expect(rows[0].textContent).toContain("warehouse-pi");
		expect(rows[0].textContent).toContain("0.9.4 available");
		expect(releases.textContent).toContain("On the current release");
		expect(releases.textContent).toContain(
			"5 active · 1 on an older agent, listed first",
		);
	});

	test("marks the viewer's plan in the history table", async () => {
		const view = await mount();
		const history = block(view, "history");
		const pro = find(history, 'tr[data-tier="PRO"]');
		expect(pro.getAttribute("aria-selected")).toBe("true");
		expect(pro.textContent).toContain("Your plan");
		expect(pro.textContent).toContain("7 days");
		expect(pro.textContent).toContain("256 MiB");
		expect(text(history, 'tr[data-tier="FREE"]')).toContain(
			"Not stored in the cloud",
		);
		expect(history.textContent).toContain("87 MiB of 256 MiB used");
		expect(text(history, "[data-history-device]")).toContain("edge-berlin-01");
	});
});

describe("Hub status: actions with a visible result", () => {
	test("Check again reads the checks once more and says what came back", async () => {
		const view = await mount();
		const before = setupRequests(view);
		await click(byRole("button", "Check again"));
		await view.settle();
		expect(setupRequests(view)).toBe(before + 1);
		const result = find(block(view, "readiness"), '[data-result="good"]');
		expect(result.textContent).toMatch(
			new RegExp(`Checked again at ${CLOCK}\\. All 6 checks pass\\.`),
		);
		await click(byRole("button", "Dismiss", result));
		expect(view.container.querySelector("[data-result]")).toBeNull();
	});

	test("Verify again fetches and verifies the release again", async () => {
		const view = await mount();
		const before = releaseRequests(view);
		await click(byRole("button", "Verify again"));
		await view.settle();
		expect(releaseRequests(view)).toBe(before + 1);
		expect(text(block(view, "releases"), '[data-result="good"]')).toMatch(
			new RegExp(`Verified again at ${CLOCK}: signed by a trusted key`),
		);
	});

	test("Copy diagnostics copies an English report without secrets and changes nothing", async () => {
		const view = await mount();
		const writes = view.fake.api.writes().length;
		await click(byRole("button", "Copy diagnostics"));
		await view.settle();
		const report = String(dom.clipboard.at(-1));
		expect(report).toContain("Flow-Like hub status · hub.test");
		expect(report).toContain("[ready] signaling");
		expect(report).toContain("max_devices_per_user=100");
		expect(report).toContain("Contains no keys, passwords or device data.");
		expect(text(view.container, '[data-hub="diagnostics"]')).toMatch(
			new RegExp(`Copied a plain-text report at ${CLOCK}`),
		);
		expect(view.fake.api.writes().length).toBe(writes);
	});

	test("a failed refresh keeps the last results and says they are from before", async () => {
		const view = await mount();
		view.fake.api.fail({ method: "GET", path: "devices/setup" }, rateLimited());
		await click(byRole("button", "Check again"));
		await view.settle();
		const readiness = block(view, "readiness");
		expect(all(readiness, 'li[data-state="pass"]').length).toBe(6);
		expect(text(readiness, '[data-result="warning"]')).toMatch(
			new RegExp(`Couldn't check at ${CLOCK}`),
		);
		const stamp = find(readiness, "header [data-stamp]");
		expect(stamp.dataset.age).toBe("error");
		expect(stamp.textContent).toContain("couldn't refresh · data from");
		expect(readiness.textContent).toContain("Last known results, checked at");
		expect(view.container.querySelector('[role="alert"]')).toBeNull();
	});

	test("a check that finishes after the page is gone is dropped", async () => {
		const view = await mount();
		const release = view.fake.api.hold({
			method: "GET",
			path: "devices/setup",
		});
		await click(byRole("button", "Check again"));
		expect(byRole("button", "Checking…").getAttribute("aria-busy")).toBe(
			"true",
		);
		await view.unmount();
		release();
		await new Promise((resolve) => setTimeout(resolve, 0));
		expect(document.querySelector("[data-result]")).toBeNull();
	});
});

describe("Hub status: a hub that isn't ready", () => {
	test("a failing check names the fix for the hub operator", async () => {
		const view = await mount({ seed: readinessWith("signaling") });
		expect(headline(view)).toContain(
			"hub.test can't set up devices right now: Connection service fails.",
		);
		const readiness = block(view, "readiness");
		expect(text(readiness, "header")).toContain("5 of 6 pass");
		const failing = find(readiness, 'li[data-state="fail"]');
		expect(failing.textContent).toContain("Connection service");
		expect(failing.textContent).toContain("Fails");
		expect(failing.textContent).toContain(
			"The hub needs one to four secure connection addresses.",
		);
		expect(failing.textContent).not.toContain("server text");
		const fix = text(failing, "[data-fix]");
		expect(fix).toContain(
			"Fix, for the hub operator: Configure WSS signaling URLs in the hub settings",
		);
		expect(fix).toContain("Until then: Live connections to devices fail");
	});

	test("a failing check blocks what depends on it and gates setup", async () => {
		const view = await mount({ seed: readinessWith("signaling") });
		expect(stateOf(view, "live")).toBe("blocked");
		expect(stateOf(view, "running")).toBe("ok");
		expect(all(view.container, "[data-dv-primary]").length).toBe(1);
		expect(text(view.container, "[data-gate-inline]")).toBe(
			"Setup needs all 6 checks to pass. Connection service fails.",
		);
		await expectGated(view, "Set up a device");
		await click(byRole("button", "Check again"));
		await view.settle();
		expect(text(block(view, "readiness"), '[data-result="warning"]')).toContain(
			"5 of 6 pass; Connection service still fails.",
		);
	});

	test("a hub without signed releases says so and fetches nothing", async () => {
		const fake = await createFakeWorkspace(readinessWith("release"));
		fake.hub.releaseTrust = null;
		fake.hub.release = undefined;
		const view = await mount({ fake });
		const releases = block(view, "releases");
		expect(releases.textContent).toContain(
			"This hub has no signed agent releases, so agents can't be updated remotely.",
		);
		expect(text(releases, "[data-fix]")).toContain(
			"Configure a release manifest URL and at least one trusted release signing key",
		);
		expect(releases.textContent).toContain("Not configured on this hub");
		expect(block(view, "hub").textContent).toContain(
			"No signed agent releases",
		);
		expect(stateOf(view, "update")).toBe("blocked");
		expect(releaseRequests(view)).toBe(0);
		expect(queryByRole("button", "Verify again")).toBeNull();
	});
});

/* The agent release: one verdict, read by the block, the headline, the summary cell and the features. */

function verdictLine(view: View) {
	return find(block(view, "releases"), "[data-verdict]");
}

function releaseCell(view: View) {
	return find(block(view, "hub"), '[data-jump="releases"]');
}

/** What every surface says about the release, by its verdict. */
function verdicts(view: View) {
	return {
		block: verdictLine(view).dataset.verdict,
		cell: releaseCell(view).dataset.verdict,
	};
}

/** Outside a usable release nothing on the block reads as verified, in any wording. */
function expectNothingPositive(view: View) {
	const releases = block(view, "releases");
	const shown = String(releases.textContent);
	expect(shown).not.toContain("Verified");
	expect(shown).not.toContain("Verification");
	expect(shown).not.toContain("genuine and current");
	expect(shown).not.toContain("within its validity window");
	expect(releases.querySelector("[data-fingerprint]")).toBeNull();
	expect(releases.querySelector("tr[data-target]")).toBeNull();
	expect(releaseCell(view).textContent).not.toContain("Verified");
	expect(releaseCell(view).querySelector(".text-good")).toBeNull();
}

/** A list the pinned key never signed: the same header and content with another signature. */
function forged(jws: string) {
	const [head, body] = jws.split(".");
	return `${head}.${body}.${"A".repeat(86)}`;
}

/** The release address fails like a dropped connection or holds its answer; everything else answers. */
async function fakeWithReleaseFetch(
	answer: "network" | "never",
): Promise<Fake> {
	const fake = await createFakeWorkspace();
	await publishTestRelease(fake);
	const inner = globalThis.fetch;
	globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
		if (String(input) !== RELEASE_URL) return inner(input, init);
		return answer === "network"
			? Promise.reject(new TypeError("Failed to fetch"))
			: new Promise<Response>(() => {});
	}) as typeof fetch;
	restoreFetch = () => {
		globalThis.fetch = inner;
	};
	return fake;
}

describe("Hub status: what the agent release means", () => {
	test("ok: one sentence says what devices get and that nothing is to do", async () => {
		const view = await mount();
		const line = verdictLine(view);
		expect(verdicts(view)).toEqual({ block: "ok", cell: "ok" });
		expect(line.dataset.tone).toBe("good");
		expect(line.textContent).toMatch(
			/^New and updated devices get agent 0\.9\.4\. The release is genuine and current\. Nothing to do until .*2027\.$/,
		);
		// The sentence is the first thing in the block, above the version.
		expect(line.parentElement?.firstElementChild).toBe(line);
		const cell = releaseCell(view);
		expect(cell.textContent).toMatch(/0\.9\.4Verifieduntil .*2027$/);
		expect(cell.querySelector(".text-good")).not.toBeNull();
		expect(headline(view)).toMatch(
			/All 6 checks pass and the current agent release is verified until .*2027\./,
		);
		expect(text(view.container, '[data-feature="update"]')).toMatch(
			/Current release 0\.9\.4 · verified until .*2027\./,
		);
		expect(view.container.querySelector('[role="alert"]')).toBeNull();
	});

	test("ends soon: a warning from 30 days before the end, with the days left", async () => {
		const { view } = await mountWith({ endsInS: 12 * DAY_S });
		const line = verdictLine(view);
		expect(verdicts(view)).toEqual({ block: "ends_soon", cell: "ends_soon" });
		expect(line.dataset.tone).toBe("warning");
		expect(line.textContent).toMatch(
			/^Agent 0\.9\.4 can be used until .+ \(12 days left\)\. After that no device can be set up or updated until the hub operator publishes or renews a release\. Devices that are running keep running\.$/,
		);
		const releases = block(view, "releases");
		// It is still verified: the chip and the row stay, the day count appears.
		expect(releases.textContent).toContain("Verified");
		expect(releases.textContent).toContain("12 days left");
		expect(releases.textContent).toContain("within its validity window");
		const cell = releaseCell(view);
		expect(cell.textContent).toMatch(
			/0\.9\.4Ends soonuntil .+ · 12 days left$/,
		);
		expect(cell.querySelector(".text-good")).toBeNull();
		expect(cell.querySelector(".text-warning")).not.toBeNull();
		expect(headline(view)).toMatch(
			/All 6 checks pass, but the agent release runs out on .+\./,
		);
		expect(stateOf(view, "update")).toBe("ok");
		expect(text(view.container, '[data-feature="update"]')).toMatch(
			/Current release 0\.9\.4 · runs out on .+\. After that, updates wait for a new or renewed release\./,
		);
		expect(stateOf(view, "setup")).toBe("ok");
	});

	test("the warning starts at 30 days and not before", async () => {
		const before = await mountWith({ endsInS: 30 * DAY_S + 60 });
		expect(verdicts(before.view).block).toBe("ok");
		await cleanupDevices();
		const at = await mountWith({ endsInS: 30 * DAY_S });
		expect(verdicts(at.view).block).toBe("ends_soon");
		expect(verdictLine(at.view).textContent).toContain("(30 days left)");
	});

	test('on its last day the release ends at a time, never in "1 day"', async () => {
		const { view } = await mountWith({ endsInS: 5 * 3_600 });
		expect(verdicts(view).block).toBe("ends_soon");
		expect(verdictLine(view).textContent).toMatch(
			/^Agent 0\.9\.4 can be used until \d{1,2}:\d{2} \(less than a day left\)\./,
		);
		expect(releaseCell(view).textContent).toMatch(
			/Ends soonuntil \d{1,2}:\d{2} · less than a day left$/,
		);
		expect(block(view, "releases").textContent).not.toMatch(/\d days? left/);
		await cleanupDevices();
		// A full day and a second is two days when rounded up; exactly one day is "1 day left".
		const day = await mountWith({ endsInS: DAY_S });
		expect(verdictLine(day.view).textContent).toContain("(1 day left)");
	});

	test("a release that runs out while the page is open stops reading as verified", async () => {
		const { view } = await mountWith({ endsInS: 7 * DAY_S }, { tickMs: 100 });
		const releases = block(view, "releases");
		expect(verdicts(view).block).toBe("ends_soon");
		expect(releases.textContent).toContain("within its validity window");
		view.fake.clock.advance(8 * DAY_S * 1000);
		await pass(300);
		expect(verdicts(view)).toEqual({ block: "expired", cell: "expired" });
		const line = verdictLine(view);
		expect(line.dataset.tone).toBe("critical");
		expect(line.getAttribute("role")).toBe("alert");
		expect(line.textContent).toMatch(
			/^The release of agent 0\.9\.4 ran out on .+\. No device can be set up or updated until the hub operator publishes or renews a release\. Devices that are running keep running\.$/,
		);
		expectNothingPositive(view);
		expect(releases.textContent).toContain("Expired");
		expect(releases.textContent).toContain("Ran out");
		expect(releaseCell(view).textContent).toMatch(/0\.9\.4Expiredran out on /);
		expect(headline(view)).toMatch(
			/All 6 checks pass, but the agent release ran out on .+, so setup can't create a package\./,
		);
		expect(headline(view)).not.toContain("ready for devices");
		expect(stateOf(view, "setup")).toBe("blocked");
		expect(text(view.container, '[data-feature="setup"]')).toMatch(
			/The agent release ran out on .+, so no setup package can be made\./,
		);
		expect(stateOf(view, "update")).toBe("blocked");
		expect(text(view.container, '[data-feature="update"]')).toMatch(
			/The agent release ran out on .+\. Devices keep the agent they have\./,
		);
		// No device is offered a release that can't be installed any more.
		expect(releases.textContent).not.toContain("0.9.4 available");
		expect(releases.textContent).toContain(
			"Keeps this agent until the hub has a verified release again",
		);
		// The clock is live here: stop it before the test hands back to the runner.
		await view.unmount();
	});

	test("a list that had run out before it was fetched says when, from its own signed dates", async () => {
		const { view } = await mountWith({
			issuedAgoS: 40 * DAY_S,
			endsInS: -2 * DAY_S,
		});
		expect(verdicts(view)).toEqual({ block: "expired", cell: "expired" });
		expect(verdictLine(view).textContent).toMatch(
			/^The release of agent 0\.9\.4 ran out on .+\./,
		);
		expectNothingPositive(view);
		const shown = String(block(view, "releases").textContent);
		expect(shown).toContain("release number 44");
		expect(shown).toContain("Release number44");
		expect(shown).not.toContain("publish or renew a release before");
	});

	const REFUSED: readonly {
		name: string;
		release: TestReleaseOptions;
		reason: string;
		/** The list is genuinely signed, so its own number and dates are shown. */
		facts: boolean;
	}[] = [
		{
			name: "a release number below the hub's minimum",
			release: { sequence: 40 },
			reason: "its release number is below the hub's minimum",
			facts: true,
		},
		{
			name: "a list dated in the future",
			release: { issuedAgoS: -3_600 },
			reason: "it isn't valid yet; check this computer's clock",
			facts: true,
		},
		{
			name: "a list valid for longer than five years",
			release: { endsInS: 6 * 365 * DAY_S },
			reason: "it is valid for longer than this app accepts; update the app",
			facts: true,
		},
	];

	for (const { name, release, reason, facts } of REFUSED)
		test(`failed: ${name} names the check`, async () => {
			const { view } = await mountWith(release);
			const line = verdictLine(view);
			expect(verdicts(view)).toEqual({ block: "failed", cell: "failed" });
			expect(line.dataset.tone).toBe("critical");
			expect(line.textContent).toBe(
				`The hub's agent release failed a check: ${reason}. Don't set up or update devices until this is resolved.`,
			);
			expectNothingPositive(view);
			const shown = String(block(view, "releases").textContent);
			expect(shown.includes("Release number")).toBe(facts);
			expect(releaseCell(view).textContent).toContain("Not verified");
			expect(headline(view)).toContain(
				"All 6 checks pass, but the agent release couldn't be verified, so setup can't create a package.",
			);
			expect(stateOf(view, "setup")).toBe("blocked");
			expect(stateOf(view, "update")).toBe("blocked");
			expectPlainWords(view.container);
		});

	test("failed: a list no pinned key signed, and bytes that are no list at all", async () => {
		const unsigned = await mountWith(undefined);
		unsigned.published?.serve(forged(unsigned.published.jws));
		await click(byRole("button", "Verify again"));
		await unsigned.view.settle();
		expect(verdictLine(unsigned.view).textContent).toBe(
			"The hub's agent release failed a check: it isn't signed by a key the hub operator pinned. Don't set up or update devices until this is resolved.",
		);
		await cleanupDevices();

		const fake = await createFakeWorkspace();
		const published = await publishTestRelease(fake);
		published.serve("<html>not a release list</html>");
		const view = await mount({ fake });
		expect(verdicts(view)).toEqual({ block: "failed", cell: "failed" });
		expect(verdictLine(view).textContent).toBe(
			"The hub's agent release failed a check: it isn't a valid release list. Don't set up or update devices until this is resolved.",
		);
		expectNothingPositive(view);
		expect(block(view, "releases").textContent).toContain(RELEASE_URL);
		expect(block(view, "releases").textContent).not.toContain("Release number");
	});

	test("a verified list followed by one that fails a check reads failed everywhere", async () => {
		const { view, published } = await mountWith(undefined);
		if (!published) throw new Error("This mount publishes its own release");
		expect(verdicts(view)).toEqual({ block: "ok", cell: "ok" });
		expect(stateOf(view, "update")).toBe("ok");

		published.serve(forged(published.jws));
		await click(byRole("button", "Verify again"));
		await view.settle();

		expect(verdicts(view)).toEqual({ block: "failed", cell: "failed" });
		expectNothingPositive(view);
		const releases = block(view, "releases");
		expect(text(releases, '[data-result="critical"]')).toMatch(
			new RegExp(
				`Checked at ${CLOCK}\\. The release failed a check: it isn't signed by a key the hub operator pinned\\.`,
			),
		);
		expect(headline(view)).toContain(
			"but the agent release couldn't be verified, so setup can't create a package.",
		);
		expect(headline(view)).not.toContain("is verified until");
		expect(headline(view)).toContain(
			"hub.test can't set up devices right now.",
		);
		expect(headline(view)).not.toContain("ready for devices");
		expect(releaseCell(view).textContent).toContain("Not verified");
		expect(stateOf(view, "setup")).toBe("blocked");
		expect(stateOf(view, "update")).toBe("blocked");
		expect(text(view.container, '[data-feature="update"]')).toContain(
			"The agent release couldn't be verified. Devices keep the agent they have.",
		);
		// The block now shows the hub's settings only, with the hub's own read time.
		expect(find(releases, "header [data-stamp]").dataset.age).not.toBe("error");
		expect(releases.textContent).not.toContain("0.9.4 available");

		// A list that verifies again brings the verdict back.
		published.serve(published.jws);
		await click(byRole("button", "Verify again"));
		await view.settle();
		expect(verdicts(view)).toEqual({ block: "ok", cell: "ok" });
	});

	test("a verified list followed by a fetch that fails keeps its verdict and says how old it is", async () => {
		const { view } = await mountWith(undefined);
		expect(verdicts(view).block).toBe("ok");
		// The release address now answers 404: the list didn't arrive, so nothing was checked.
		view.fake.hub.release = undefined;
		await click(byRole("button", "Verify again"));
		await view.settle();

		expect(verdicts(view)).toEqual({ block: "ok", cell: "ok" });
		const releases = block(view, "releases");
		expect(text(releases, '[data-result="warning"]')).toMatch(
			new RegExp(
				`Couldn't verify at ${CLOCK}: The release address didn't answer with a release list\\.`,
			),
		);
		const stamp = find(releases, "header [data-stamp]");
		expect(stamp.dataset.age).toBe("error");
		expect(stamp.textContent).toContain("couldn't refresh · data from");
		expect(releases.textContent).toMatch(
			new RegExp(`\\(checked on this computer at ${CLOCK}\\)`),
		);
		expect(releases.textContent).toContain("within its validity window");
	});

	test("unfetched: without an earlier verification nothing is claimed", async () => {
		const fake = await createFakeWorkspace();
		await publishTestRelease(fake);
		fake.hub.release = undefined;
		const view = await mount({ fake });
		const line = verdictLine(view);
		expect(verdicts(view)).toEqual({ block: "unfetched", cell: "unfetched" });
		expect(line.dataset.tone).toBe("warning");
		expect(line.textContent).toBe(
			"The release list couldn't be fetched, so nothing is verified. Setup and agent updates wait. The release address didn't answer with a release list.",
		);
		expectNothingPositive(view);
		expect(releaseCell(view).textContent).toContain(
			"Not verifiedSetup and agent updates wait",
		);
		expect(stateOf(view, "setup")).toBe("blocked");
		expect(byRole("button", "Verify again")).toBeTruthy();
	});

	test("unfetched: a dropped connection is named as the cause", async () => {
		const view = await mount({ fake: await fakeWithReleaseFetch("network") });
		// A dropped connection is tried again before it counts as a failure.
		expect(verdicts(view).block).toBe("checking");
		await idle(view);
		expect(verdicts(view).block).toBe("unfetched");
		expect(verdictLine(view).textContent).toBe(
			"The release list couldn't be fetched, so nothing is verified. Setup and agent updates wait. The hub can't be reached.",
		);
	}, 30_000);

	test("checking: the first verification is running", async () => {
		const view = await mount({ fake: await fakeWithReleaseFetch("never") });
		const line = verdictLine(view);
		expect(verdicts(view)).toEqual({ block: "checking", cell: "checking" });
		expect(line.dataset.tone).toBe("neutral");
		expect(line.textContent).toBe("Checking the release…");
		expectNothingPositive(view);
		expect(headline(view)).toContain(
			"All 6 checks pass. The agent release is being verified.",
		);
		expect(stateOf(view, "setup")).toBe("unknown");
		// Nothing is said against the devices' agents before the answer is in.
		const releases = block(view, "releases");
		expect(all(releases, "[data-device]").length).toBe(5);
		expect(releases.textContent).not.toContain("Keeps this agent");
		expect(releases.textContent).toContain("5 active");
		expect(releases.textContent).not.toContain("on an older agent");
	});
});

describe("Hub status: devices whose agent version isn't known", () => {
	test("a locked device offers Unlock to read instead of a value", async () => {
		const view = await mount({ unlock: "none", viewFacts: false });
		const releases = block(view, "releases");
		expect(releases.textContent).not.toContain("Unknown");
		const unlock = all(releases, '[data-act="unlock"]');
		expect(unlock.length).toBeGreaterThan(0);
		expect(unlock[0].textContent).toBe("Unlock to read");
		const row = unlock[0].closest<HTMLElement>("[data-device]");
		if (!row) throw new Error("The action sits in a device row");
		// The row doesn't repeat the action as a sentence.
		expect(row.textContent).not.toContain("agent version");
		await click(unlock[0]);
		expect(useOverlayStore.getState().overlay).toMatchObject({
			kind: "unlock",
			deviceId: row.dataset.device,
		});
	});

	test("a device that never checked in shows a dash and says so", async () => {
		const view = await mount({ unlock: "none" });
		const rows = all(block(view, "releases"), "[data-device]");
		const never = rows.find((row) =>
			String(row.textContent).includes("Hasn't checked in yet."),
		);
		if (!never)
			throw new Error("The sample fleet has a device without a check-in");
		expect(never.querySelector('[data-act="unlock"]')).toBeNull();
		expect(never.textContent).toContain("–");
	});
});

describe("Hub status: release settings among the hub's checks", () => {
	test("the hub's own check is about its settings and points to the block that verifies", async () => {
		const view = await mount();
		const check = all(block(view, "readiness"), 'li[data-state="pass"]').find(
			(row) => String(row.textContent).includes("Release settings"),
		);
		if (!check) throw new Error("The checks list has no release settings row");
		expect(check.textContent).not.toContain("Signed agent releases fails");
		expect(check.textContent).toContain(
			"The hub names where releases are published and which keys sign them. This app verifies the release itself; see Signed agent releases.",
		);
	});
});

describe("Hub status: hub states", () => {
	test("checking: nothing is claimed before the hub answers", async () => {
		const never = (async (_input: RequestInfo | URL) =>
			new Promise<Response>(() => {})) as typeof fetch;
		const view = await mount({ workspace: { fetch: never } });
		expect(headline(view)).toContain("Checking hub.test…");
		expect(block(view, "hub").textContent).toContain("Checking…");
		expect(find(block(view, "limits"), '[data-kind="loading"]')).toBeTruthy();
		expect(find(block(view, "releases"), '[data-kind="loading"]')).toBeTruthy();
		await expectGated(view, "Set up a device");
		expect(view.container.querySelector('[role="alert"]')).toBeNull();
		expect(all(view.container, "[data-kind='empty']").length).toBe(0);
	});

	test("off: unread checks say so and the limits still show, without usage", async () => {
		const fake = await createFakeWorkspace();
		fake.api.mode.devicesEnabled = false;
		const view = await mount({ fake });
		await idle(view);
		expect(headline(view)).toContain("Devices are off on hub.test.");
		expect(block(view, "hub").textContent).toContain("Off on this hub");
		expect(text(block(view, "readiness"), '[data-gate="hub"]')).toContain(
			"Devices are off on this hub. Its checks couldn't be read.",
		);
		expect(stateOf(view, "checkin")).toBe("blocked");
		const limits = block(view, "limits");
		expect(limits.textContent).toContain(
			"These limits apply once the hub operator turns device support on.",
		);
		expect(all(limits, "[data-limit]").length).toBe(4);
		expect(limits.querySelector("[data-usage]")).toBeNull();
		expect(all(view.container, "[data-kind='empty']").length).toBe(0);
	}, 20_000);

	test("off: Set up a device is gated and sends nothing; Check again asks the hub and says what came back", async () => {
		const fake = await createFakeWorkspace();
		fake.api.mode.devicesEnabled = false;
		const view = await mount({ fake });
		await idle(view);
		await expectGated(view, "Set up a device");

		const again = byRole("button", "Check again");
		expect(again.getAttribute("aria-disabled")).toBeNull();
		const asked = setupRequests(view);
		await click(again);
		await idle(view);
		expect(setupRequests(view)).toBeGreaterThan(asked);
		expect(block(view, "readiness").textContent).toMatch(
			new RegExp(`Couldn't check at ${CLOCK}`),
		);
	}, 30_000);

	test("off: the hub still answers its checks, every block states its own read, and Check again shows the operator's fix", async () => {
		const fake = await createFakeWorkspace();
		fake.api.mode.devicesEnabled = false;
		answerChecksWhileOff(fake);
		const view = await mount({ fake });
		await idle(view);
		expect(headline(view)).toContain("Devices are off on hub.test.");
		const readiness = block(view, "readiness");
		expect(readiness.textContent).toContain("5 of 6 pass");
		expect(text(readiness, "[data-fix]")).toContain(
			"Turn on device support in the hub settings",
		);
		// The device list can't be read while devices are off. That isn't a failed read of these blocks.
		const ages = ["hub", "readiness", "features", "limits", "releases"].map(
			(id) => find(block(view, id), "header [data-stamp]").dataset.age,
		);
		expect(ages).not.toContain("error");

		fake.api.mode.devicesEnabled = true;
		await click(byRole("button", "Check again"));
		await idle(view);
		expect(headline(view)).toContain("hub.test is ready for devices.");
		expect(readiness.textContent).toMatch(
			new RegExp(`Checked again at ${CLOCK}\\. All 6 checks pass\\.`),
		);
	}, 30_000);

	test("unreachable: Retry reports the attempt, and the page fills once the hub answers", async () => {
		const api = fakeDeviceApi();
		let answers = false;
		const hub = (async (input: RequestInfo | URL) =>
			answers
				? api.hubFetch(input)
				: new Response("Not Found", { status: 404 })) as typeof fetch;
		const fake = await createFakeWorkspace(undefined, {
			api,
			workspace: { fetch: hub },
		});
		const view = await mount({ fake });
		const banner = byRole("alert");
		expect(banner.textContent).toContain(
			"hub.test doesn't say whether it supports devices.",
		);
		expect(block(view, "hub").textContent).toContain("Hub unreachable");
		expect(find(block(view, "limits"), '[data-kind="notloaded"]')).toBeTruthy();
		expect(all(view.container, "[data-kind='empty']").length).toBe(0);
		await click(byRole("button", "Retry now", banner));
		await view.settle();
		expect(banner.textContent).toMatch(
			new RegExp(`Still unreachable at ${CLOCK}`),
		);
		answers = true;
		await click(byRole("button", "Retry now", banner));
		await view.settle();
		expect(queryByRole("alert")).toBeNull();
		expect(headline(view)).toContain("hub.test is ready for devices.");
	});
});

describe("Hub status: limits", () => {
	test("over the device limit with 200 devices: setup is gated and the list stays capped", async () => {
		const seed = generateFleet(200).input;
		if (!seed.usage) throw new Error("The generated fleet has usage");
		seed.usage.limits.max_devices = 100;
		const { active_devices, pending_enrollments } = seed.usage.usage;
		const used = active_devices + pending_enrollments;
		const view = await mount({ seed, unlock: "none" });
		expect(used).toBeGreaterThan(100);
		expect(block(view, "hub").textContent).toContain("Over the limit");
		expect(headline(view)).toContain(
			`You're over your device limit, though: ${used} of 100`,
		);
		expect(block(view, "limits").textContent).toContain(
			`You're using ${used} of 100 device slots.`,
		);
		expect(text(view.container, "[data-gate-inline]")).toContain(
			"device limit",
		);
		await expectGated(view, "Set up a device");
		const releases = block(view, "releases");
		expect(all(releases, "[data-device]").length).toBe(6);
		await click(byRole("button", "Show 20 more"));
		expect(all(releases, "[data-device]").length).toBe(26);
		await click(byRole("button", "Show fewer"));
		expect(all(releases, "[data-device]").length).toBe(6);
	}, 30_000);

	test("a hub that states its limits but not your usage shows the limits alone", async () => {
		const view = await mount();
		view.fake.api.fail(
			{ method: "GET", path: "devices/usage" },
			new ApiResponseError({ status: 404, code: "NOT_FOUND", message: "no" }),
		);
		await act(async () => {
			await view.fake.queryClient.refetchQueries();
		});
		await view.settle();
		const limits = block(view, "limits");
		expect(all(limits, "[data-limit]").length).toBe(4);
		expect(limits.textContent).toContain("100");
		expect(limits.textContent).toContain("220");
		expect(limits.querySelector("[data-usage]")).toBeNull();
		expect(text(limits, "footer")).toContain(
			"This hub doesn't report how much of them you use yet",
		);
		expect(block(view, "hub").textContent).toContain(
			"This hub doesn't report your usage",
		);
		expect(view.container.querySelector('[role="alert"]')).toBeNull();
	});
});

describe("Hub status: older hub and older agents", () => {
	test("an older hub shows the interims and no error", async () => {
		const view = await mount({
			hubVersion: "old",
			seed: sampleFleetOlderHub(),
		});
		expect(headline(view)).toContain("hub.test is ready for devices.");
		expect(text(block(view, "limits"), '[data-kind="unsupported"]')).toContain(
			"This hub doesn't state its limits.",
		);
		expect(block(view, "limits").textContent).not.toContain(
			"only the limits are shown",
		);
		const history = block(view, "history");
		expect(all(history, "tr[data-tier]").length).toBeGreaterThan(0);
		expect(history.querySelector('tr[aria-selected="true"]')).toBeNull();
		expect(history.textContent).not.toContain("Your plan");
		expect(history.textContent).toContain(
			"This hub doesn't report your plan or how much history you use yet.",
		);
		expect(block(view, "hub").textContent).toContain(
			"This hub doesn't report your usage",
		);
		expect(view.container.querySelector('[role="alert"]')).toBeNull();
		expect(view.container.querySelector('[data-kind="error"]')).toBeNull();
		expectPlainWords(view.container);
	});

	test("an older hub is asked for each new route once, not in a loop", async () => {
		const view = await mount({
			hubVersion: "old",
			seed: sampleFleetOlderHub(),
		});
		await view.settle();
		for (const route of ["devices/usage", "devices/archive-usage"])
			expect(view.fake.api.sent("GET", route).length).toBeLessThanOrEqual(1);
	});

	test("older agents change nothing here: no device command is sent", async () => {
		const view = await mount({
			agentFeatures: {},
			seed: sampleFleetOlderAgent(),
		});
		const before = view.fake.api.commands.length;
		const devices = find(block(view, "releases"), '[data-hub="devices"]');
		expect(all(devices, "[data-device]").length).toBe(5);
		await click(byRole("button", "Check again"));
		await click(byRole("button", "Verify again"));
		await view.settle();
		expect(view.fake.api.commands.length).toBe(before);
		expect(view.container.querySelector('[role="alert"]')).toBeNull();
	});
});

describe("The setup wizard's blocks", () => {
	test("the compact checks are a complete block with head, stamp and Check again", async () => {
		const ready: boolean[] = [];
		const view = await mountDevices(
			<ReadinessList compact onReady={(value) => ready.push(value)} />,
		);
		const checks = find(view.container, "[data-block]");
		expect(text(checks, "h2")).toContain("Hub status checks");
		expect(text(checks, "header")).toContain("6 of 6 ready");
		expect(find(checks, "header [data-stamp]").dataset.src).toBe("hub");
		expect(byRole("button", "Check again", checks)).toBeTruthy();
		const rows = all(checks, "li");
		expect(rows.length).toBe(6);
		expect(rows[0].textContent).toContain("Device limits · Ready");
		expect(text(checks, "footer")).toMatch(
			new RegExp(
				`Checked at ${CLOCK}\\. Each failing check is fixed by the hub operator\\.`,
			),
		);
		expect(ready).toEqual([true]);
	});

	test("the compact limits are a complete block with three counts", async () => {
		const view = await mountDevices(<HubLimitsUsage compact />);
		const limits = find(view.container, "[data-block]");
		expect(text(limits, "h2")).toContain("Your limits");
		expect(find(limits, "header [data-stamp]").dataset.src).toBe("hub");
		const counts = all(limits, "[data-limit]").map((row) => row.textContent);
		expect(counts.length).toBe(3);
		expect(counts[0]).toContain("Devices6 of 100");
		expect(counts[1]).toContain("Unused setup packages1 of 10");
		expect(counts[2]).toContain("Packages in the last 24 h1 of 220");
		expect(all(limits, "[data-meter]").length).toBe(3);
	});

	test("a failing check reports not ready and says who fixes it", async () => {
		const ready: boolean[] = [];
		const view = await mountDevices(
			<ReadinessList compact onReady={(value) => ready.push(value)} />,
			{ seed: readinessWith("database") },
		);
		const failing = text(view.container, 'li[data-state="fail"]');
		expect(failing).toContain("Database migrations · Not ready");
		expect(failing).toContain(
			"Until the hub operator fixes this, no device can be set up on this hub.",
		);
		expect(text(view.container, "header")).toContain("5 of 6 ready");
		expect(ready).toEqual([false]);
	});

	test("on an older hub the compact limits render their interim without an error", async () => {
		const view = await mountDevices(<HubLimitsUsage compact />, {
			hubVersion: "old",
			seed: sampleFleetOlderHub(),
		});
		expect(text(view.container, '[data-kind="unsupported"]')).toContain(
			"This hub doesn't state its limits.",
		);
		expect(view.container.querySelector('[role="alert"]')).toBeNull();
		const asked = view.fake.api.sent("GET", "devices/usage").length;
		expect(asked).toBeLessThanOrEqual(1);
	});
});
