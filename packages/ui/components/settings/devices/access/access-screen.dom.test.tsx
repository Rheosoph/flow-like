import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { ApiResponseError } from "../../../../lib/api-error";
import {
	SAMPLE_APPS,
	SAMPLE_IDS,
	SAMPLE_NOW,
	SAMPLE_PEOPLE,
	emptyInput,
} from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import { deviceApiBase } from "../../../../lib/device-management/storage";
import type { ManagementPolicy } from "../../../../lib/device-management/types";
import {
	allByRole,
	byRole,
	byText,
	click,
	clickByText,
	dropFiles,
	inPortal,
	installDom,
	queryByRole,
	typeInto,
} from "../testing/dom-harness";
import type { FakeWorkspace } from "../testing/fake-workspace";
import type { MountedDevices } from "../testing/mount-devices";

const dom = installDom();
const { cleanupDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { act } = await import("react");
const { MACHINE_WORDS, mountAccess, requestFile } = await import(
	"./access-test-kit"
);
const { AccessScreen } = await import("./access-screen");
const { GrantRowActions } = await import("./change-permissions-sheet");
const { accessStoreOf } = await import("./use-access");
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { fakeCompact, fakeKeys } = await import("../testing/fake-device-api");
const { useDevicesRoute } = await import("../routing/use-devices-route");
const { useOverlayStore } = await import("../workspace/overlay-store");
const { useActivityTray } = await import("../shell/activity-tray");

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
	useOverlayStore.getState().close();
	globalThis.localStorage?.clear();
});
afterAll(dom.restore);

const DAY = 86_400;
const { mira: MIRA, jonas: JONAS } = SAMPLE_PEOPLE;
const ANNA = "usr_anna";
const PEOPLE = {
	[MIRA]: "Mira Novak",
	[JONAS]: "Jonas Weber",
	[ANNA]: "Anna Petrova",
};

function Routed() {
	const { route, scope } = useDevicesRoute();
	return <AccessScreen route={route} scope={scope} />;
}

const mount = (options: Parameters<typeof mountAccess>[1] = {}) =>
	mountAccess(<Routed />, {
		search: "view=access",
		people: PEOPLE,
		...options,
	});

const primaries = (root: ParentNode) =>
	root.querySelectorAll("[data-dv-primary]").length;

const section = (container: HTMLElement, deviceId: string) =>
	container.querySelector(`#access-device-${deviceId}`) as HTMLElement;

const policyOf = (fake: FakeWorkspace, deviceId: string) =>
	fake.hub.policies.get(deviceId)?.policy as ManagementPolicy;

const policyPuts = (fake: FakeWorkspace) =>
	fake.api.sent("PUT", /management\/policy$/);

/** The device confirms the saved rules and the screen reads the hub again. */
async function deviceApplies(mounted: MountedDevices, deviceId: string) {
	mounted.fake.hub.applyPolicy(deviceId);
	await act(async () => {
		await mounted.fake.queryClient.invalidateQueries();
	});
	await mounted.settle();
}

describe("Access › People", () => {
	test("the headline, a section per shared device and the people of unlocked devices", async () => {
		const { container } = await mount();
		expect(byRole("heading", "Access")).toBeTruthy();
		const headline = container.querySelector("[data-headline]")?.textContent;
		expect(headline).toContain("2 people can reach your devices.");
		// One sentence, as SPEC §5.7 words it.
		expect(headline).toMatch(
			/Mira Novak's access to edge-berlin-01 ends at \d{2}:\d{2}, and studio-mac-mini hasn't applied your last change yet\.$/,
		);
		const edge = section(container, SAMPLE_IDS.edge);
		expect(edge.textContent).toContain("Saved v5");
		expect(edge.textContent).toContain("device has v5");
		expect(edge.textContent).toContain("2 of 24 used");
		expect(edge.textContent).toContain("Sandbox required");
		// Ends are counts, never "next mo." or "tomorrow".
		expect(edge.textContent).toContain(
			"in 31d · re-signed for 31 days with every change",
		);
		expect(edge.querySelector("[data-stamp]")?.textContent).not.toContain(
			"polling",
		);
		const rows = Array.from(edge.querySelectorAll("[data-grant]"));
		expect(rows).toHaveLength(2);
		expect(rows[0]?.textContent).toContain("Mira Novak");
		expect(rows[0]?.textContent).toContain("Whole device");
		expect(rows[0]?.textContent).toContain("Active on device");
		expect(rows[0]?.querySelector("[data-ends-soon]")).toBeTruthy();
		expect(rows[1]?.textContent).toContain("Jonas Weber");
		expect(rows[1]?.textContent).toContain("Runs code, sandboxed");
		// App scopes read "App {name}" and link to App › Devices (APP §6.2).
		const appLink = rows[1]?.querySelector("[data-scope=app] a");
		expect(rows[1]?.querySelector("[data-scope=app]")?.textContent).toBe(
			"App Invoice AI",
		);
		expect(appLink?.getAttribute("href")).toBe(
			`/library/config/devices?id=${SAMPLE_APPS.invoiceAi}`,
		);
		const studio = section(container, SAMPLE_IDS.studio);
		expect(studio.textContent).toContain("Saved v2");
		expect(studio.textContent).toContain("device has v1");
		expect(studio.querySelector("[data-rules-waiting]")?.textContent).toContain(
			"It's online, so this usually applies within 5 minutes.",
		);
		// The block says it reads the hub faster while the device hasn't applied the rules.
		expect(studio.querySelector("[data-stamp]")?.textContent).toContain(
			"polling every 10 s · checked",
		);
		expect(studio.textContent).toContain("No sandbox");
		expect(
			studio
				.querySelector("[data-grant-status]")
				?.getAttribute("data-grant-status"),
		).toBe("waiting");
		// Devices without rules are "Only you", never an empty table.
		expect(
			container.querySelector(`[data-only-you="${SAMPLE_IDS.warehouse}"]`),
		).toBeTruthy();
		expect(
			container.querySelector(`[data-only-you="${SAMPLE_IDS.cold}"]`)
				?.textContent,
		).toContain("Unlock it first");
		expect(primaries(container)).toBe(1);
		expect(container.textContent).not.toMatch(MACHINE_WORDS);
	});

	test("an account the directory knows without a name is unnamed, never labelled with its account id", async () => {
		const { container } = await mount({ people: { ...PEOPLE, [MIRA]: "" } });
		const edge = section(container, SAMPLE_IDS.edge);
		expect(edge.textContent).toContain("Jonas Weber");
		const unknown = edge.querySelector("[data-person-unknown]") as HTMLElement;
		// The id is the technical second line, not the name.
		expect(unknown.firstElementChild?.textContent).toBe("?Unknown account");
		expect(unknown.lastElementChild?.textContent).toBe(MIRA);
		expect(edge.querySelector("[data-person]")?.textContent).not.toContain(
			"usr_",
		);
		expect(container.querySelector("[data-headline]")?.textContent).toContain(
			"One person's access to edge-berlin-01 ends at",
		);
	});

	test("the permissions cell shows the preset and a count; the list opens on demand", async () => {
		const { container } = await mount();
		const cell = section(container, SAMPLE_IDS.edge).querySelectorAll(
			"[data-permissions]",
		)[1] as HTMLElement;
		expect(cell.textContent).toBe("Custom · 7 permissions");
		await click(byRole("button", "7 permissions", cell));
		const list = inPortal("dialog");
		expect(list.textContent).toContain("Deploy & configure");
		expect(list.textContent).toContain("Change instance count");
		expect(list.querySelectorAll("li")).toHaveLength(7);
	});

	test("the connection file sheet names the owner key, the device and the hub the other person signs in to", async () => {
		const { container, fake } = await mount();
		await click(
			byRole(
				"button",
				"Download connection file…",
				section(container, SAMPLE_IDS.edge),
			),
		);
		const sheet = inPortal("dialog");
		expect(sheet.textContent).toContain("Connection file for edge-berlin-01");
		expect(sheet.textContent).toContain("Your owner key fingerprint");
		const hub = sheet.querySelector("[data-connection-hub]")?.textContent;
		expect(hub).toBe(new URL(deviceApiBase(fake.workspace.deps.scope)).host);
		expect(hub).not.toContain("/");
		expect(
			byRole("button", "Download connection file", sheet).getAttribute(
				"aria-disabled",
			),
		).toBeNull();
	});

	test("locked: the rules stay readable, the people need the keys, gated controls send nothing", async () => {
		const { container, fake } = await mount({ unlock: "none" });
		const edge = section(container, SAMPLE_IDS.edge);
		expect(edge.textContent).toContain("Saved v5");
		expect(edge.querySelector("[data-gate=locked]")?.textContent).toContain(
			"Unlock edge-berlin-01 to change who has access.",
		);
		expect(edge.querySelector("[data-grant]")).toBeNull();
		expect(edge.textContent).not.toContain("Nobody else has access");
		const headline = container.querySelector("[data-headline]")?.textContent;
		expect(headline).toContain(
			"Unlock your shared devices to see who can reach them.",
		);
		const renew = byRole("button", "Renew access rules…", edge);
		expect(renew.getAttribute("aria-disabled")).toBe("true");
		expect(edge.textContent).toContain(
			"Unlock first: renewing re-signs the rules.",
		);
		const before = fake.api.writes().length;
		await click(renew);
		await click(byRole("button", "Download connection file…", edge));
		expect(fake.api.writes()).toHaveLength(before);
		expect(queryByRole("alertdialog")).toBeNull();
		await click(byRole("button", "Unlock…", edge));
		expect(useOverlayStore.getState().overlay).toMatchObject({
			kind: "unlock",
			deviceId: SAMPLE_IDS.edge,
		});
	});

	test("remove access: consequence preview with an undo row, one signed save, live status until the device applies it", async () => {
		const mounted = await mount();
		const { container, fake } = mounted;
		const edge = section(container, SAMPLE_IDS.edge);
		await click(byRole("button", "More for Mira Novak", edge));
		await clickByText("Remove Mira Novak's access…", inPortal("menu"));
		const sheet = inPortal("alertdialog");
		expect(sheet.textContent).toContain("Remove Mira Novak's access?");
		expect(sheet.textContent).toContain(
			"Mira Novak loses Viewer · 3 permissions on edge-berlin-01, the whole device.",
		);
		expect(sheet.textContent).toContain("Who loses access");
		expect(sheet.textContent).toContain("Can you undo it?");
		expect(sheet.textContent).toContain(
			"Add Mira again from a new access request.",
		);
		expect(policyPuts(fake)).toHaveLength(0);
		await click(byRole("button", "Remove Mira Novak", sheet));
		await mounted.settle();

		expect(policyPuts(fake)).toHaveLength(1);
		const saved = policyOf(fake, SAMPLE_IDS.edge);
		expect(saved.policy_version).toBe(6);
		expect(saved.grants.map((grant) => grant.user_id)).toEqual([JONAS]);
		expect(saved.expires_at).toBe(SAMPLE_NOW + 31 * DAY);
		const removed = edge.querySelector("[data-grant-status=removing]");
		expect(removed?.textContent).toBe("Removed · waiting for device");
		expect(edge.textContent).toContain(
			"Saved access rules v6 without Mira Novak",
		);
		expect(edge.textContent).toContain(
			"Waiting for edge-berlin-01 to apply it.",
		);
		const tray = fake.workspace.activity
			.list()
			.find((item) => item.kind === "access_rules" && item.state === "waiting");
		expect(tray?.resume).toEqual({ type: "policy", version: 6 });
		// What this computer did is listed under Recent access changes.
		expect(container.textContent).toContain(
			"You saved access rules v6 on edge-berlin-01: removed Mira Novak.",
		);

		await deviceApplies(mounted, SAMPLE_IDS.edge);
		expect(edge.querySelectorAll("[data-grant]")).toHaveLength(1);
		expect(edge.textContent).toContain("edge-berlin-01 applied it.");
		expect(edge.textContent).toContain("device has v6");
	});

	test("cancelling the removal sends nothing", async () => {
		const { container, fake } = await mount();
		const edge = section(container, SAMPLE_IDS.edge);
		await click(byRole("button", "More for Jonas Weber", edge));
		await clickByText("Remove Jonas Weber's access…", inPortal("menu"));
		await click(byRole("button", "Cancel", inPortal("alertdialog")));
		expect(policyPuts(fake)).toHaveLength(0);
		expect(edge.querySelectorAll("[data-grant]")).toHaveLength(2);
	});

	test("renew: the end moves, nobody else changes, and the sheet shows the consequences itself", async () => {
		const mounted = await mount();
		const { container, fake } = mounted;
		const edge = section(container, SAMPLE_IDS.edge);
		const before = policyOf(fake, SAMPLE_IDS.edge);
		const mira = before.grants.find((grant) => grant.user_id === MIRA);
		const row = edge.querySelector("[data-grant]") as HTMLElement;
		await click(byRole("button", "Renew…", row));
		const sheet = inPortal("alertdialog");
		expect(sheet.textContent).toContain(
			"Renew Mira Novak's access to edge-berlin-01?",
		);
		expect(sheet.textContent).toContain("Nobody else's access changes.");
		expect(sheet.querySelector("input[type=password]")).toBeNull();
		await click(byRole("button", "Renew Mira's access", sheet));
		await mounted.settle();
		expect(policyPuts(fake)).toHaveLength(1);
		const after = policyOf(fake, SAMPLE_IDS.edge);
		expect(
			after.grants.find((grant) => grant.user_id === MIRA)?.expires_at,
		).toBe((mira?.expires_at ?? 0) + DAY);
		expect(after.grants.find((grant) => grant.user_id === JONAS)).toEqual(
			before.grants.find((grant) => grant.user_id === JONAS),
		);
		expect(queryByRole("alertdialog")).toBeNull();
		expect(edge.textContent).toContain("Mira Novak's access renewed until");
	});

	test("renew access rules re-signs the same people for 31 days", async () => {
		const mounted = await mount();
		const { container, fake } = mounted;
		const edge = section(container, SAMPLE_IDS.edge);
		const before = policyOf(fake, SAMPLE_IDS.edge);
		await click(byRole("button", "Renew access rules…", edge));
		const sheet = inPortal("alertdialog");
		expect(sheet.textContent).toContain(
			"The same people keep the same permissions and end dates.",
		);
		await click(byRole("button", "Renew access rules", sheet));
		await mounted.settle();
		const after = policyOf(fake, SAMPLE_IDS.edge);
		expect(after.policy_version).toBe(before.policy_version + 1);
		expect(after.grants).toEqual(before.grants);
		expect(after.expires_at).toBe(SAMPLE_NOW + 31 * DAY);
		expect(edge.textContent).toContain("Access rules renewed as v6");
	});

	test("a hub that refuses the save leaves the rules alone and says so next to the control", async () => {
		const mounted = await mount();
		const { container, fake } = mounted;
		const edge = section(container, SAMPLE_IDS.edge);
		fake.api.fail({ method: "PUT", path: /management\/policy$/ });
		await click(byRole("button", "Renew access rules…", edge));
		await click(
			byRole("button", "Renew access rules", inPortal("alertdialog")),
		);
		await mounted.settle();
		expect(policyOf(fake, SAMPLE_IDS.edge).policy_version).toBe(5);
		const result = edge.querySelector("[data-result=critical]");
		expect(result?.textContent).toContain("Nothing changed");
		expect(result?.textContent).toContain(
			"edge-berlin-01 keeps its current access rules.",
		);
	});

	test("the rules are read every 30 s, and every 10 s while a device has not applied them", async () => {
		const { container } = await mount();
		const cadence = (deviceId: string) =>
			section(container, deviceId)
				.querySelector("header [data-stamp]")
				?.getAttribute("title") ?? "";
		expect(cadence(SAMPLE_IDS.edge)).toContain("30");
		expect(cadence(SAMPLE_IDS.edge)).not.toContain("10");
		expect(cadence(SAMPLE_IDS.studio)).toContain("10");
	});

	test("a refresh that fails keeps the people on screen and says the data is older", async () => {
		const mounted = await mount();
		const { container, fake } = mounted;
		fake.api.fail({ method: "GET", path: /management\/policy$/ });
		await act(async () => {
			await fake.queryClient.invalidateQueries();
		});
		await mounted.settle();
		const edge = section(container, SAMPLE_IDS.edge);
		expect(edge.querySelectorAll("[data-grant]")).toHaveLength(2);
		expect(
			edge.querySelector("header [data-stamp]")?.getAttribute("data-age"),
		).toBe("error");
		expect(container.textContent).not.toContain("Nobody else has access");
	});

	test("rules the hub can't deliver are an error with a retry, never 'only you'", async () => {
		const fake = await createFakeWorkspace();
		const stop = fake.api.fail(
			{
				method: "GET",
				path: new RegExp(`${SAMPLE_IDS.edge}/management/policy$`),
			},
			new ApiResponseError({
				status: 429,
				code: "RATE_LIMITED",
				message: "Too many requests",
			}),
		);
		fake.queryClient.clear();
		const mounted = await mount({ fake });
		const { container } = mounted;
		expect(section(container, SAMPLE_IDS.edge)).toBeNull();
		expect(
			container.querySelector(`[data-only-you="${SAMPLE_IDS.edge}"]`),
		).toBeNull();
		const error = container.querySelector("[data-kind=error]") as HTMLElement;
		expect(error.textContent).toContain(
			"Couldn't read the access rules of edge-berlin-01",
		);
		stop();
		await click(byRole("button", "Try again", error));
		await mounted.settle();
		expect(
			section(container, SAMPLE_IDS.edge).querySelectorAll("[data-grant]"),
		).toHaveLength(2);
		expect(container.querySelector("[data-kind=error]")).toBeNull();
	});
});

describe("Access › rules that can't be verified", () => {
	test("an unlocked device whose rules don't check out says so; the headline doesn't ask to unlock it", async () => {
		const fake = await createFakeWorkspace();
		const stored = fake.hub.policies.get(SAMPLE_IDS.edge);
		if (!stored?.policy) throw new Error("the sample fleet changed");
		stored.jws = fakeCompact(
			stored.policy,
			fakeKeys.invitation("usr_someone_else", SAMPLE_IDS.edge),
		);
		fake.queryClient.clear();
		const mounted = await mount({ fake });
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 700));
		});
		await mounted.settle();
		const { container } = mounted;
		const edge = section(container, SAMPLE_IDS.edge);
		expect(edge.querySelector("[data-grant]")).toBeNull();
		expect(edge.querySelector("[data-kind=error]")?.textContent).toContain(
			"These access rules don't check out with your owner key",
		);
		expect(edge.querySelector("[data-kind=loading]")).toBeNull();
		// studio-mac-mini still verifies: its people count, and nothing asks to unlock edge-berlin-01.
		const headline = container.querySelector("[data-headline]")?.textContent;
		expect(headline).toContain("1 person can reach your devices.");
		expect(headline).not.toContain("Unlock");
		expect(
			section(container, SAMPLE_IDS.studio).querySelectorAll("[data-grant]"),
		).toHaveLength(1);
	});
});

describe("Access › Add people", () => {
	async function openWizard(mounted: MountedDevices) {
		await click(byRole("button", "Add people…", mounted.container));
		return inPortal("dialog");
	}

	const next = (sheet: HTMLElement) => byRole("button", "Continue", sheet);

	async function chooseDevice(sheet: HTMLElement, deviceId: string) {
		const box = sheet.querySelector(
			`[data-device-choice="${deviceId}"] [role=checkbox]`,
		) as HTMLElement;
		await click(box);
	}

	async function importFile(mounted: MountedDevices, file: File) {
		const drop = inPortal("dialog").querySelector(
			"label[for=access-wizard-files]",
		) as HTMLElement;
		await dropFiles(drop, [file]);
		await mounted.settle();
	}

	test("devices → request file → Viewer → review → saved; the result follows the device live", async () => {
		const mounted = await mount();
		const { fake } = mounted;
		const existing = policyOf(fake, SAMPLE_IDS.edge).grants;
		let sheet = await openWizard(mounted);
		expect(sheet.textContent).toContain("Step 1 of 5 · Devices");
		expect(next(sheet).getAttribute("aria-disabled")).toBe("true");
		expect(sheet.textContent).toContain("Select at least one device.");
		// A locked device can't be chosen; it says why and offers Unlock.
		const cold = sheet.querySelector(
			`[data-device-choice="${SAMPLE_IDS.cold}"]`,
		) as HTMLElement;
		expect(cold.getAttribute("data-locked")).toBe("locked");
		expect(cold.textContent).toContain(
			"Unlock cold-storage-nas first: access rules are signed with your owner key.",
		);
		await chooseDevice(sheet, SAMPLE_IDS.edge);
		await click(next(sheet));

		sheet = inPortal("dialog");
		expect(sheet.textContent).toContain("Step 2 of 5 · Request files");
		expect(sheet.textContent).toContain(
			"Import at least one access request file.",
		);
		await importFile(
			mounted,
			requestFile(SAMPLE_IDS.edge, ANNA, "anna-key", "anna-grant"),
		);
		sheet = inPortal("dialog");
		const fileRow = sheet.querySelector("[data-file-row]") as HTMLElement;
		expect(fileRow.getAttribute("data-ok")).toBe("true");
		expect(fileRow.textContent).toContain("Anna Petrova");
		expect(fileRow.textContent).toContain("edge-berlin-01");
		expect(fileRow.textContent).toContain(
			"Ready. Compare the fingerprint with what Anna reads out.",
		);
		// Blocks of four with the case and "-" kept, as the other person reads them out.
		expect(
			fileRow.querySelector("[data-key-fingerprint] button")?.textContent,
		).toBe("anna -key AAAA AAAA …");
		expect(fileRow.textContent).toContain("Request ID anna-gra");
		await click(next(sheet));

		sheet = inPortal("dialog");
		expect(sheet.textContent).toContain("Step 3 of 5 · Permissions");
		expect(sheet.querySelector("[data-summary]")?.textContent).toBe(
			"Viewer · 3 permissions",
		);
		await click(next(sheet));

		// Viewer runs no code: no trust step.
		sheet = inPortal("dialog");
		expect(sheet.textContent).toContain("Step 4 of 5 · Review");
		expect(sheet.textContent).toContain("access rules v5 → v6");
		expect(sheet.textContent).toContain("New access");
		expect(sheet.textContent).toContain(
			"Anna Petrova gets Viewer · 3 permissions on edge-berlin-01, the whole device, until",
		);
		// New access: every permission is a chip marked as added.
		expect(
			Array.from(sheet.querySelectorAll("[data-k=added]"), (chip) =>
				chip.textContent?.replace("added", ""),
			),
		).toEqual(["View status", "Read logs", "Read metrics"]);
		expect(sheet.querySelector("[data-k=removed]")).toBeNull();
		expect(sheet.textContent).toContain("Can you undo it?");
		expect(sheet.querySelector("[data-signing-note]")?.textContent).toContain(
			"There's no separate password.",
		);
		expect(sheet.querySelector("input[type=password]")).toBeNull();
		expect(policyPuts(fake)).toHaveLength(0);
		await click(byRole("button", "Save access rules", sheet));
		await mounted.settle();

		expect(policyPuts(fake)).toHaveLength(1);
		const saved = policyOf(fake, SAMPLE_IDS.edge);
		expect(saved.policy_version).toBe(6);
		const anna = saved.grants.find((grant) => grant.user_id === ANNA);
		expect(anna).toMatchObject({
			grant_id: "anna-grant",
			scope: { kind: "device" },
			capabilities: ["status", "logs", "metrics"],
			expires_at: SAMPLE_NOW + DAY,
		});
		// Everyone who had access keeps exactly what they had.
		expect(saved.grants.slice(0, 2)).toEqual(existing);
		expect(saved.grants).toHaveLength(3);
		sheet = inPortal("dialog");
		const result = sheet.querySelector("[data-result-device]") as HTMLElement;
		expect(result.getAttribute("data-result")).toBe("waiting");
		expect(result.textContent).toContain("saved v6");
		expect(result.textContent).toContain("Waiting for device");

		await deviceApplies(mounted, SAMPLE_IDS.edge);
		const applied = inPortal("dialog").querySelector(
			"[data-result-device]",
		) as HTMLElement;
		expect(applied.getAttribute("data-result")).toBe("applied");
		expect(applied.textContent).toContain("Anna can use it now.");
		await click(byRole("button", "Done", inPortal("dialog")));
		expect(queryByRole("dialog")).toBeNull();
		expect(
			section(mounted.container, SAMPLE_IDS.edge).querySelectorAll(
				"[data-grant]",
			),
		).toHaveLength(3);
		// The change is listed with what was given.
		expect(
			mounted.container.querySelector("[data-change]")?.textContent,
		).toContain(
			"You saved access rules v6 on edge-berlin-01: added Anna Petrova (Viewer on the whole device).",
		);
	});

	test("code-running permissions on a device without a required sandbox need the trust confirmation", async () => {
		const mounted = await mount();
		const { fake } = mounted;
		let sheet = await openWizard(mounted);
		await chooseDevice(sheet, SAMPLE_IDS.studio);
		await click(next(sheet));
		await importFile(
			mounted,
			requestFile(SAMPLE_IDS.studio, ANNA, "anna-studio"),
		);
		await click(next(inPortal("dialog")));
		sheet = inPortal("dialog");
		await click(byRole("button", "Operator", sheet));
		expect(inPortal("dialog").textContent).toContain("Step 3 of 6");
		await click(next(inPortal("dialog")));

		sheet = inPortal("dialog");
		expect(sheet.textContent).toContain("Step 4 of 6 · Trust");
		expect(sheet.textContent).toContain("studio-mac-mini has no sandbox.");
		expect(sheet.textContent).toContain(
			"Anna Petrova would get Start services, Restart services, Change instance count on studio-mac-mini.",
		);
		expect(next(sheet).getAttribute("aria-disabled")).toBe("true");
		expect(sheet.textContent).toContain(
			"Tick each confirmation, or choose fewer permissions.",
		);
		await click(next(sheet));
		expect(inPortal("dialog").textContent).toContain("Step 4 of 6 · Trust");
		await click(
			byRole(
				"checkbox",
				"I understand Anna can run code on studio-mac-mini with the agent's full access.",
				sheet,
			),
		);
		await click(next(inPortal("dialog")));
		sheet = inPortal("dialog");
		expect(sheet.textContent).toContain("Step 5 of 6 · Review");
		expect(sheet.textContent).toContain(
			"Runs code with the agent's full access to studio-mac-mini.",
		);
		// The device still waits for v2: the review says it goes straight on.
		expect(sheet.textContent).toContain(
			"v2 isn't applied yet. The device goes straight to v3.",
		);
		await click(byRole("button", "Save access rules", sheet));
		await mounted.settle();
		expect(
			policyOf(fake, SAMPLE_IDS.studio).grants.find(
				(grant) => grant.user_id === ANNA,
			)?.capabilities,
		).toEqual([
			"status",
			"logs",
			"metrics",
			"start",
			"stop",
			"restart",
			"scale",
		]);
		// The steps taken stay in the stepper: the saved rules alone would no longer ask for trust.
		sheet = inPortal("dialog");
		const steps = Array.from(sheet.querySelectorAll("ol > li[data-s]"));
		expect(steps).toHaveLength(6);
		expect(steps[3]?.textContent).toContain("Trust");
		expect(steps[5]?.getAttribute("aria-current")).toBe("step");
		// "Follow in activity" hands over to the tray.
		expect(useActivityTray.getState().open).toBe(false);
		await click(byRole("button", "Follow in activity", sheet));
		expect(queryByRole("dialog")).toBeNull();
		expect(useActivityTray.getState().open).toBe(true);
		useActivityTray.getState().setOpen(false);
	});

	test("choosing Viewer instead takes the trust step away, and a device that requires a sandbox never asks", async () => {
		const mounted = await mount();
		let sheet = await openWizard(mounted);
		await chooseDevice(sheet, SAMPLE_IDS.studio);
		await click(next(sheet));
		await importFile(
			mounted,
			requestFile(SAMPLE_IDS.studio, ANNA, "anna-studio"),
		);
		await click(next(inPortal("dialog")));
		await click(byRole("button", "Deployer", inPortal("dialog")));
		await click(next(inPortal("dialog")));
		sheet = inPortal("dialog");
		expect(sheet.textContent).toContain("· Trust");
		await clickByText("Give Anna Viewer instead", sheet);
		sheet = inPortal("dialog");
		expect(sheet.textContent).toContain("Step 4 of 5 · Review");
		expect(sheet.textContent).not.toContain("I understand");

		// edge-berlin-01 requires a sandbox: Deployer goes straight to the review.
		await click(byRole("button", "Cancel", sheet));
		sheet = await openWizard(mounted);
		await chooseDevice(sheet, SAMPLE_IDS.edge);
		await click(next(sheet));
		await importFile(mounted, requestFile(SAMPLE_IDS.edge, ANNA, "anna-edge"));
		await click(next(inPortal("dialog")));
		await click(byRole("button", "Deployer", inPortal("dialog")));
		await click(next(inPortal("dialog")));
		expect(inPortal("dialog").textContent).toContain("Step 4 of 5 · Review");
	});

	test("a device that stops requiring a sandbox while the review is open asks for trust before anything is saved", async () => {
		const mounted = await mount();
		const { fake } = mounted;
		let sheet = await openWizard(mounted);
		await chooseDevice(sheet, SAMPLE_IDS.edge);
		await click(next(sheet));
		await importFile(mounted, requestFile(SAMPLE_IDS.edge, ANNA, "anna-edge"));
		await click(next(inPortal("dialog")));
		await click(byRole("button", "Deployer", inPortal("dialog")));
		await click(next(inPortal("dialog")));
		expect(inPortal("dialog").textContent).toContain("Step 4 of 5 · Review");

		fake.agent(SAMPLE_IDS.edge).facts.host_isolation = "none";
		await act(async () => {
			await fake.workspace.live.refreshInspection(SAMPLE_IDS.edge);
		});
		await mounted.settle();
		await click(byRole("button", "Save access rules", inPortal("dialog")));
		await mounted.settle();
		expect(policyPuts(fake)).toHaveLength(0);
		sheet = inPortal("dialog");
		expect(sheet.textContent).toContain("· Trust");
		const confirmation = byRole(
			"checkbox",
			"I understand Anna can run code on edge-berlin-01 with the agent's full access.",
			sheet,
		);

		await click(confirmation);
		await click(next(inPortal("dialog")));
		await click(byRole("button", "Save access rules", inPortal("dialog")));
		await mounted.settle();
		expect(policyPuts(fake)).toHaveLength(1);
	});

	test("a device that stops requiring a sandbox after its save failed asks for trust before the retry signs", async () => {
		const mounted = await mount();
		const { fake } = mounted;
		const isolate = async (deviceId: string, value: "required" | "none") => {
			fake.agent(deviceId).facts.host_isolation = value;
			await act(async () => {
				await fake.workspace.live.refreshInspection(deviceId);
			});
			await mounted.settle();
		};
		const putsFor = (deviceId: string) =>
			policyPuts(fake).filter(([, path]) => path.includes(deviceId));
		await isolate(SAMPLE_IDS.studio, "required");

		let sheet = await openWizard(mounted);
		await chooseDevice(sheet, SAMPLE_IDS.edge);
		await chooseDevice(inPortal("dialog"), SAMPLE_IDS.studio);
		await click(next(inPortal("dialog")));
		await importFile(mounted, requestFile(SAMPLE_IDS.edge, ANNA, "anna-edge"));
		await importFile(
			mounted,
			requestFile(SAMPLE_IDS.studio, ANNA, "anna-studio"),
		);
		await click(next(inPortal("dialog")));
		await click(byRole("button", "Deployer", inPortal("dialog")));
		await click(next(inPortal("dialog")));
		// Both devices require a sandbox: no trust step.
		expect(inPortal("dialog").textContent).toContain("Step 4 of 5 · Review");

		fake.api.fail(
			{
				method: "PUT",
				path: new RegExp(`${SAMPLE_IDS.edge}/management/policy$`),
			},
			undefined,
			1,
		);
		await click(byRole("button", "Save access rules", inPortal("dialog")));
		await mounted.settle();
		expect(putsFor(SAMPLE_IDS.studio)).toHaveLength(1);
		const attempts = putsFor(SAMPLE_IDS.edge).length;
		expect(policyOf(fake, SAMPLE_IDS.edge).policy_version).toBe(5);

		await isolate(SAMPLE_IDS.edge, "none");
		sheet = inPortal("dialog");
		await click(byRole("button", "Try again for 1 device", sheet));
		await mounted.settle();
		expect(putsFor(SAMPLE_IDS.edge)).toHaveLength(attempts);
		sheet = inPortal("dialog");
		expect(sheet.textContent).toContain("Step 4 of 6 · Trust");
		// What is saved stays saved: the steps before the confirmation are closed.
		expect(queryByRole("button", "Back", sheet)).toBeNull();
		await click(
			byRole(
				"checkbox",
				"I understand Anna can run code on edge-berlin-01 with the agent's full access.",
				sheet,
			),
		);
		await click(next(inPortal("dialog")));
		await click(byRole("button", "Save access rules", inPortal("dialog")));
		await mounted.settle();
		expect(putsFor(SAMPLE_IDS.edge)).toHaveLength(attempts + 1);
		expect(policyOf(fake, SAMPLE_IDS.edge).policy_version).toBe(6);
		expect(putsFor(SAMPLE_IDS.studio)).toHaveLength(1);
	});

	test("narrower scopes can't hold device-wide permissions, and say why", async () => {
		const mounted = await mount();
		let sheet = await openWizard(mounted);
		await chooseDevice(sheet, SAMPLE_IDS.edge);
		await click(next(sheet));
		await importFile(mounted, requestFile(SAMPLE_IDS.edge, ANNA, "anna-edge"));
		await click(next(inPortal("dialog")));
		sheet = inPortal("dialog");
		await click(byRole("button", "Device admin", sheet));
		expect(
			inPortal("dialog").querySelector("[data-summary]")?.textContent,
		).toBe("Device admin · 12 permissions");
		await click(byRole("button", "App", inPortal("dialog")));
		sheet = inPortal("dialog");
		for (const capability of [
			"update_agent",
			"reboot",
			"manage_certificates",
		]) {
			const row = sheet.querySelector(
				`[data-permission="${capability}"]`,
			) as HTMLElement;
			expect(row.getAttribute("data-blocked")).toBe("device_only");
			expect(row.textContent).toContain("Only with whole-device access.");
			const box = row.querySelector("[role=checkbox]") as HTMLElement;
			expect(box.getAttribute("aria-checked")).toBe("false");
			expect(box.hasAttribute("disabled")).toBe(true);
		}
		expect(sheet.textContent).toContain(
			"Device admin needs whole-device access.",
		);
		expect(
			byRole("button", "Device admin", sheet).hasAttribute("disabled"),
		).toBe(true);
		expect(sheet.querySelector("[data-summary]")?.textContent).toBe(
			"Deployer · 9 permissions",
		);
		// An app has to be chosen before the step can be left.
		expect(next(sheet).getAttribute("aria-disabled")).toBe("true");
		expect(sheet.textContent).toContain("Choose an app.");
	});

	test("file problems are flagged per row: own account, duplicate, another person's request ID, unreadable file", async () => {
		const mounted = await mount();
		const { fake } = mounted;
		let sheet = await openWizard(mounted);
		await chooseDevice(sheet, SAMPLE_IDS.edge);
		await click(next(sheet));
		const miraGrant = policyOf(fake, SAMPLE_IDS.edge).grants[0]?.grant_id;
		const drop = inPortal("dialog").querySelector(
			"label[for=access-wizard-files]",
		) as HTMLElement;
		await dropFiles(drop, [
			requestFile(SAMPLE_IDS.edge, fake.hub.me, "me"),
			requestFile(SAMPLE_IDS.edge, ANNA, "anna-1"),
			requestFile(SAMPLE_IDS.edge, ANNA, "anna-2"),
			requestFile(SAMPLE_IDS.edge, JONAS, "stolen", miraGrant),
			new File(["{}"], "notes.json"),
			requestFile(SAMPLE_IDS.studio, JONAS, "jonas-studio"),
		]);
		await mounted.settle();
		sheet = inPortal("dialog");
		const rows = Array.from(sheet.querySelectorAll("[data-file-row]"));
		const flags = rows.map((row) => [
			row.getAttribute("data-ok"),
			row.querySelector("[data-flag]")?.getAttribute("data-flag"),
		]);
		expect(flags).toEqual([
			["false", "critical"],
			["true", "good"],
			["false", "warning"],
			["false", "critical"],
			["false", "critical"],
			["false", "warning"],
		]);
		expect(rows[0]?.textContent).toContain("This is your own account.");
		expect(rows[2]?.textContent).toContain(
			"Anna Petrova is listed twice for edge-berlin-01.",
		);
		expect(rows[3]?.textContent).toContain(
			"already belongs to Mira Novak's access on edge-berlin-01",
		);
		expect(rows[4]?.textContent).toContain(
			"This isn't an access request file.",
		);
		expect(rows[5]?.textContent).toContain(
			"Made for studio-mac-mini, which isn't selected.",
		);
		expect(sheet.textContent).toContain("1 of 6 files can be used.");
		// "Include" adds the device; the file becomes usable.
		await clickByText("Include studio-mac-mini", sheet);
		expect(inPortal("dialog").textContent).toContain(
			"2 of 6 files can be used.",
		);
	});

	test("importing an existing person's own request again renews instead of adding", async () => {
		// Mira's key as her app would have sent it (the sample rules carry a short stand-in).
		const fake = await createFakeWorkspace();
		const rules = structuredClone(policyOf(fake, SAMPLE_IDS.edge));
		const existing = rules.grants[0];
		if (existing)
			existing.controller_key = {
				kty: "OKP",
				crv: "Ed25519",
				x: "mira".padEnd(43, "A"),
			};
		fake.hub.setPolicy(SAMPLE_IDS.edge, rules, 5, 5);
		const mounted = await mount({ fake });
		let sheet = await openWizard(mounted);
		await chooseDevice(sheet, SAMPLE_IDS.edge);
		await click(next(sheet));
		const again = new File(
			[
				JSON.stringify([
					{
						user_id: MIRA,
						controller_key: existing?.controller_key,
						grant_id: existing?.grant_id,
					},
				]),
			],
			`device-access-${SAMPLE_IDS.edge}.json`,
		);
		await importFile(mounted, again);
		sheet = inPortal("dialog");
		expect(sheet.querySelector("[data-flag=info]")?.textContent).toContain(
			"Mira Novak already has access to edge-berlin-01",
		);
		await click(next(sheet));
		await click(next(inPortal("dialog")));
		sheet = inPortal("dialog");
		expect(
			sheet
				.querySelector("[data-change-kind]")
				?.getAttribute("data-change-kind"),
		).toBe("renewed");
		await click(byRole("button", "Save access rules", sheet));
		await mounted.settle();
		const saved = policyOf(fake, SAMPLE_IDS.edge);
		expect(saved.grants).toHaveLength(2);
		expect(
			saved.grants.find((grant) => grant.user_id === MIRA)?.expires_at,
		).toBe(SAMPLE_NOW + DAY);
	});

	async function toPermissions(mounted: MountedDevices, deviceId: string) {
		const sheet = await openWizard(mounted);
		await chooseDevice(sheet, deviceId);
		await click(next(sheet));
		await importFile(mounted, requestFile(deviceId, ANNA, "anna-key"));
		await click(next(inPortal("dialog")));
	}

	const certificates = () =>
		inPortal("dialog").querySelector(
			"[data-permission=manage_certificates]",
		) as HTMLElement;

	test("Manage certificates can be given while the agent supports sharing it", async () => {
		const mounted = await mount();
		const { fake } = mounted;
		await toPermissions(mounted, SAMPLE_IDS.edge);
		expect(certificates().getAttribute("data-blocked")).toBeNull();
		await click(certificates().querySelector("[role=checkbox]") as HTMLElement);
		expect(
			inPortal("dialog").querySelector("[data-summary]")?.textContent,
		).toBe("Custom · 4 permissions");
		await click(next(inPortal("dialog")));
		await click(byRole("button", "Save access rules", inPortal("dialog")));
		await mounted.settle();
		expect(
			policyOf(fake, SAMPLE_IDS.edge).grants.find(
				(grant) => grant.user_id === ANNA,
			)?.capabilities,
		).toEqual(["status", "logs", "metrics", "manage_certificates"]);
	});

	test("certificate support is checked again when the change is signed: lost support saves nothing", async () => {
		const mounted = await mount();
		const { fake } = mounted;
		await toPermissions(mounted, SAMPLE_IDS.edge);
		await click(certificates().querySelector("[role=checkbox]") as HTMLElement);
		await click(next(inPortal("dialog")));
		// The agent stops supporting it while the save reads the current rules.
		const release = fake.api.hold({
			method: "GET",
			path: /management\/policy$/,
		});
		await click(byRole("button", "Save access rules", inPortal("dialog")));
		fake.agent(SAMPLE_IDS.edge).facts.certificate_management = undefined;
		await act(async () => {
			await fake.workspace.live.refreshInspection(SAMPLE_IDS.edge);
		});
		release();
		await mounted.settle();
		expect(policyPuts(fake)).toHaveLength(0);
		let sheet = inPortal("dialog");
		expect(sheet.textContent).toContain("· Review");
		expect(sheet.textContent).toContain(
			"edge-berlin-01's agent can't share Manage certificates.",
		);
		// The permission is no longer offered, and the review no longer carries it.
		expect(sheet.textContent).toContain("Viewer · 3 permissions");
		await click(byRole("button", "Back", sheet));
		expect(certificates().getAttribute("data-blocked")).toBe(
			"certificates_unsupported",
		);
		expect(certificates().textContent).toContain(
			"edge-berlin-01's agent can't share this. Update the agent first.",
		);
		await click(next(inPortal("dialog")));
		sheet = inPortal("dialog");
		await click(byRole("button", "Save access rules", sheet));
		await mounted.settle();
		expect(policyPuts(fake)).toHaveLength(1);
		expect(
			policyOf(fake, SAMPLE_IDS.edge).grants.find(
				(grant) => grant.user_id === ANNA,
			)?.capabilities,
		).toEqual(["status", "logs", "metrics"]);
	});

	test("without a held owner key the password is asked in place, and a wrong one saves nothing", async () => {
		const mounted = await mount({ heldSigner: false });
		const { fake } = mounted;
		let sheet = await openWizard(mounted);
		await chooseDevice(sheet, SAMPLE_IDS.edge);
		await click(next(sheet));
		await importFile(mounted, requestFile(SAMPLE_IDS.edge, ANNA, "anna-edge"));
		await click(next(inPortal("dialog")));
		await click(next(inPortal("dialog")));
		sheet = inPortal("dialog");
		const password = sheet.querySelector(
			"input[type=password]",
		) as HTMLInputElement;
		expect(password).toBeTruthy();
		expect(sheet.querySelector("[data-signing-note]")).toBeNull();
		const save = byRole("button", "Save access rules", sheet);
		expect(save.getAttribute("aria-disabled")).toBe("true");
		await click(save);
		expect(policyPuts(fake)).toHaveLength(0);

		await typeInto(password, "not the password");
		await click(byRole("button", "Save access rules", inPortal("dialog")));
		await mounted.settle();
		expect(policyPuts(fake)).toHaveLength(0);
		sheet = inPortal("dialog");
		expect(sheet.textContent).toContain(
			"That password didn't open the owner key of edge-berlin-01. Nothing was saved.",
		);

		await typeInto(
			sheet.querySelector("input[type=password]") as HTMLInputElement,
			fake.password,
		);
		await click(byRole("button", "Save access rules", inPortal("dialog")));
		await mounted.settle();
		expect(policyPuts(fake)).toHaveLength(1);
		expect(policyOf(fake, SAMPLE_IDS.edge).grants).toHaveLength(3);
		expect(JSON.stringify(fake.api.calls)).not.toContain(fake.password);
	});
});

describe("Access › requests, deep links and interims", () => {
	test("imported request files are kept on this computer, reviewed from the list and cleared once used", async () => {
		const mounted = await mount();
		const { container, fake } = mounted;
		const block = container.querySelector("#access-requests") as HTMLElement;
		expect(block.textContent).toContain("No requests waiting.");
		expect(block.textContent).toContain("imported here · not synced");
		await dropFiles(
			block.querySelector("label[for=access-requests-files]") as HTMLElement,
			[requestFile(SAMPLE_IDS.edge, ANNA, "anna-edge", "anna-grant")],
		);
		await mounted.settle();
		// The review opens with the file; closing it keeps the request listed.
		expect(inPortal("dialog").textContent).toContain("Request files");
		await click(byRole("button", "Cancel", inPortal("dialog")));
		const request = block.querySelector("[data-request]") as HTMLElement;
		expect(request.textContent).toContain("Anna Petrova");
		expect(request.textContent).toContain("asks for access to");
		expect(request.textContent).toContain("edge-berlin-01");
		expect(request.textContent).toContain(
			"Compare with what Anna reads out before you add them.",
		);
		expect(accessStoreOf(fake.workspace).get().requests).toHaveLength(1);

		await click(byRole("button", "Review and add…", request));
		let sheet = inPortal("dialog");
		expect(sheet.textContent).toContain("Step 2 of 5 · Request files");
		await click(byRole("button", "Continue", sheet));
		await click(byRole("button", "Continue", inPortal("dialog")));
		sheet = inPortal("dialog");
		await click(byRole("button", "Save access rules", sheet));
		await mounted.settle();
		expect(policyPuts(fake)).toHaveLength(1);
		expect(accessStoreOf(fake.workspace).get().requests).toHaveLength(0);
	});

	test("discarding a request asks first and removes only the local file", async () => {
		const mounted = await mount();
		const { container, fake } = mounted;
		await act(async () => {
			accessStoreOf(fake.workspace).addRequests([
				{
					id: "r1",
					file: "device-access-anna.json",
					userId: ANNA,
					controllerKey: {
						kty: "OKP",
						crv: "Ed25519",
						x: "a".repeat(43),
					},
					deviceId: SAMPLE_IDS.warehouse,
					importedAt: SAMPLE_NOW - 3600,
				},
			]);
		});
		await mounted.settle();
		const writes = fake.api.writes().length;
		const request = container.querySelector("[data-request]") as HTMLElement;
		expect(request.textContent).toMatch(
			/warehouse-pi is offline since \d{2}:\d{2}\. You can add Anna now; the change applies when it checks in again\./,
		);
		expect(
			request.querySelector("[data-key-fingerprint]")?.textContent,
		).toContain("aaaa aaaa aaaa aaaa …");
		await click(byRole("button", "Discard request…", request));
		const sheet = inPortal("alertdialog");
		expect(sheet.textContent).toContain("Import the file again.");
		await click(byRole("button", "Discard request", sheet));
		await mounted.settle();
		expect(container.querySelector("[data-request]")).toBeNull();
		expect(fake.api.writes()).toHaveLength(writes);
		expect(container.textContent).toContain("Discarded Anna Petrova's request");
	});

	test("a file read that finishes after the screen is gone never lands (late import, account switch)", async () => {
		const mounted = await mount();
		const { container, fake } = mounted;
		const store = accessStoreOf(fake.workspace);
		let finish: (text: string) => void = () => undefined;
		const slow = requestFile(SAMPLE_IDS.edge, ANNA, "anna-edge");
		const text = await slow.text();
		Object.defineProperty(slow, "text", {
			value: () =>
				new Promise<string>((resolve) => {
					finish = resolve;
				}),
		});
		await dropFiles(
			container.querySelector(
				"label[for=access-requests-files]",
			) as HTMLElement,
			[slow],
		);
		await mounted.unmount();
		finish(text);
		await new Promise((resolve) => setTimeout(resolve, 0));
		expect(store.get().requests).toHaveLength(0);
		expect(queryByRole("dialog")).toBeNull();
	});

	test("deep links open their flow once and drop the parameter", async () => {
		const added = await mount({ search: "view=access&action=add-people" });
		expect(inPortal("dialog").textContent).toContain("Add people");
		expect(added.navigations.at(-1)).toMatchObject({ mode: "replace" });
		expect(added.navigations.at(-1)?.href).not.toContain("action=");
		await cleanupDevices();

		const imported = await mount({ search: "view=access&import=request" });
		expect(inPortal("dialog").textContent).toContain(
			"Import access request files",
		);
		expect(imported.navigations.at(-1)?.href).not.toContain("import=");
		await cleanupDevices();

		const requested = await mount({ search: "view=access&action=request" });
		expect(inPortal("dialog").textContent).toContain(
			"Request access to someone's device",
		);
		// The request ends as a pending row on Shared with me, so the link lands there.
		expect(requested.navigations.at(-1)).toMatchObject({ mode: "replace" });
		expect(requested.navigations.at(-1)?.href).toContain("tab=shared");
		expect(requested.navigations.at(-1)?.href).not.toContain("action=");
	});

	test("an older hub: rules, people and changes work; nothing new is asked more than once and no error shows", async () => {
		const mounted = await mount({ hubVersion: "old" });
		const { container, fake } = mounted;
		const edge = section(container, SAMPLE_IDS.edge);
		expect(edge.querySelectorAll("[data-grant]")).toHaveLength(2);
		expect(container.querySelector("[data-kind=error]")).toBeNull();
		expect(container.querySelector("[role=alert]")).toBeNull();
		// The header leaves the cloud approvals count out instead of guessing.
		expect(
			byRole("heading", "Access").parentElement?.textContent,
		).not.toContain("cloud approval");
		for (const route of [
			/devices\/resource-summary/,
			/devices\/usage/,
			/management\/my-access/,
		])
			expect(fake.api.sent("GET", route).length).toBeLessThanOrEqual(1);
		await click(byRole("button", "Renew access rules…", edge));
		await click(
			byRole("button", "Renew access rules", inPortal("alertdialog")),
		);
		await mounted.settle();
		expect(policyOf(fake, SAMPLE_IDS.edge).policy_version).toBe(6);
	});

	test("an older agent: no command is sent, and unknown certificate support blocks only that permission", async () => {
		const mounted = await mount({ agentFeatures: {} });
		const { container, fake } = mounted;
		const commands = fake.api.commands.length;
		expect(
			section(container, SAMPLE_IDS.edge).querySelectorAll("[data-grant]"),
		).toHaveLength(2);
		expect(container.querySelector("[role=alert]")).toBeNull();
		await click(byRole("button", "Add people…", container));
		let sheet = inPortal("dialog");
		await click(
			sheet.querySelector(
				`[data-device-choice="${SAMPLE_IDS.warehouse}"] [role=checkbox]`,
			) as HTMLElement,
		);
		await click(byRole("button", "Continue", sheet));
		await dropFiles(
			inPortal("dialog").querySelector(
				"label[for=access-wizard-files]",
			) as HTMLElement,
			[requestFile(SAMPLE_IDS.warehouse, ANNA, "anna-wh")],
		);
		await mounted.settle();
		await click(byRole("button", "Continue", inPortal("dialog")));
		sheet = inPortal("dialog");
		// warehouse-pi was never read live: its agent's support is unknown.
		const certificates = sheet.querySelector(
			"[data-permission=manage_certificates]",
		) as HTMLElement;
		expect(certificates.getAttribute("data-blocked")).toBe(
			"certificates_unknown",
		);
		expect(certificates.textContent).toContain(
			"warehouse-pi hasn't been read live",
		);
		expect(
			sheet
				.querySelector("[data-permission=reboot] [role=checkbox]")
				?.hasAttribute("disabled"),
		).toBe(false);
		expect(fake.api.commands).toHaveLength(commands);
	});

	test("no devices: the page says so and offers setup as the one primary action", async () => {
		const { container } = await mount({ seed: emptyInput() });
		expect(container.textContent).toContain(
			"You don't own any devices, and none are shared with you.",
		);
		expect(allByRole("link", "Set up a device").length).toBeGreaterThan(0);
		expect(primaries(container)).toBe(1);
		expect(byText("You don't own any devices yet")).toBeTruthy();
	});
});

describe("Access › one person's actions outside the Access screens", () => {
	test("Renew…, Change permissions… and Remove access… work for one row, and nothing renders while the people can't be read", async () => {
		const fake = await createFakeWorkspace();
		const jonas = policyOf(fake, SAMPLE_IDS.edge).grants.find(
			(grant) => grant.user_id === JONAS,
		);
		if (!jonas) throw new Error("the sample fleet changed");
		const mounted = await mountAccess(
			<GrantRowActions deviceId={SAMPLE_IDS.edge} grantId={jonas.grant_id} />,
			{ fake, people: PEOPLE },
		);
		const { container } = mounted;
		await click(byRole("button", "Renew…", container));
		expect(inPortal("alertdialog").textContent).toContain(
			"Renew Jonas Weber's access to edge-berlin-01?",
		);
		await click(byRole("button", "Cancel", inPortal("alertdialog")));
		await click(byRole("button", "More for Jonas Weber", container));
		await clickByText("Remove Jonas Weber's access…", inPortal("menu"));
		const confirm = inPortal("alertdialog");
		expect(confirm.textContent).toContain("Can you undo it?");
		await click(byRole("button", "Remove Jonas Weber", confirm));
		await mounted.settle();
		expect(policyPuts(fake)).toHaveLength(1);
		expect(
			policyOf(fake, SAMPLE_IDS.edge).grants.some(
				(grant) => grant.user_id === JONAS,
			),
		).toBe(false);
		expect(container.textContent).toContain(
			"Saved access rules v6 without Jonas Weber",
		);

		await cleanupDevices();
		const locked = await mountAccess(
			<GrantRowActions deviceId={SAMPLE_IDS.edge} grantId={jonas.grant_id} />,
			{ unlock: "none", people: PEOPLE },
		);
		expect(locked.container.querySelector("[data-grant-actions]")).toBeNull();
		expect(queryByRole("button", "Renew…", locked.container)).toBeNull();
	});
});
