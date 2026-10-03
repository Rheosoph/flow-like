import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { StandaloneRelease } from "../../../../lib/device-management/package";
import {
	NEW_SLOT,
	type SetupDraft,
	type StepFacts,
	defaultMode,
	duplicateName,
	findDraft,
	freshDraft,
	isExpired,
	keyBackupFile,
	lockedOut,
	modeAvailable,
	nameIssue,
	packageFile,
	packageFolder,
	packsAgent,
	passwordIssue,
	readDrafts,
	releaseCutoff,
	repeatIssue,
	resolveStep,
	startBy,
	suggestName,
	targetOptions,
	waitingSetup,
	writeDraft,
} from "./setup-state";

const MIB = 1024 * 1024;
const NOW = 1_790_769_600;

const release = (
	artifacts: [StandaloneRelease["artifacts"][number]["target"], number][],
	platforms: string[] | null,
): StandaloneRelease => ({
	version: 1,
	state_schema_version: 12,
	sequence: 44,
	release_version: "0.9.4",
	issued_at: NOW - 3600,
	expires_at: NOW + 86_400,
	artifacts: artifacts.map(([target, size]) => ({
		target,
		size,
		url: `https://releases.test/${target}`,
		sha256: "0".repeat(64),
	})),
	container: platforms ? { image: "ghcr.io/a/b@sha256:0", platforms } : null,
});

const created = (patch: Partial<SetupDraft> = {}): SetupDraft =>
	freshDraft({
		name: "factory-line-4",
		target: "x86_64-unknown-linux-gnu",
		mode: "binary",
		created: {
			enrollmentId: "enrollment-1",
			deviceId: "device-1",
			createdAt: NOW - 60,
			expiresAt: NOW + 86_340,
			outcome: "saved",
		},
		...patch,
	});

describe("names and passwords", () => {
	test("the name follows the hub's rule: not empty, at most 256 bytes, trimmed, no control characters", () => {
		expect(nameIssue("")).toEqual({ code: "empty" });
		expect(nameIssue(" edge")).toEqual({ code: "edge_space" });
		expect(nameIssue("edge\tpi")).toEqual({ code: "control" });
		expect(nameIssue("edge\u0085pi")).toEqual({ code: "control" });
		expect(nameIssue("ä".repeat(128))).toBeUndefined();
		expect(nameIssue("ä".repeat(129))).toEqual({
			code: "too_long",
			bytes: 258,
		});
		expect(nameIssue("factory line 4")).toBeUndefined();
	});

	test("a name in use is found without regard to case", () => {
		expect(duplicateName("Edge-Berlin-01", ["edge-berlin-01"])).toBe(
			"edge-berlin-01",
		);
		expect(duplicateName("", [""])).toBeUndefined();
		expect(duplicateName("new", ["old"])).toBeUndefined();
	});

	test("the next free numbered name is suggested, keeping leading zeros", () => {
		expect(suggestName(["factory-line-3", "factory-line-4"])).toEqual({
			name: "factory-line-5",
			follows: "factory-line-3",
		});
		expect(suggestName(["lab-gpu-09"])?.name).toBe("lab-gpu-10");
		expect(suggestName(["kiosk"])).toBeUndefined();
	});

	test("the password is counted in bytes and must be repeated exactly", () => {
		expect(passwordIssue("")).toEqual({ code: "empty" });
		expect(passwordIssue("ääääää")).toBeUndefined();
		expect(passwordIssue("ääääé")).toEqual({ code: "too_short", bytes: 10 });
		expect(passwordIssue("a".repeat(4097))).toEqual({
			code: "too_long",
			bytes: 4097,
		});
		expect(repeatIssue("secret-secret", "")).toBe("empty");
		expect(repeatIssue("secret-secret", "secret-secreT")).toBe("mismatch");
		expect(repeatIssue("secret-secret", "secret-secret")).toBeUndefined();
	});

	test("file names keep what reads well in a shell", () => {
		expect(packageFile("factory line 4")).toBe("flow-like-factory-line-4.zip");
		expect(packageFolder("büro/pc")).toBe("flow-like-b-ro-pc");
		expect(keyBackupFile("  ")).toBe("flow-like-device-keys.json");
	});
});

describe("platforms of a release", () => {
	const options = targetOptions(
		release(
			[
				["x86_64-unknown-linux-gnu", 100 * MIB],
				["aarch64-apple-darwin", 512 * MIB],
			],
			["linux/amd64"],
		),
	);
	const of = (target: string) =>
		options.find((option) => option.target === target);

	test("a platform without an artifact is unavailable; Docker exists only where the image does", () => {
		expect(of("x86_64-unknown-linux-gnu")).toMatchObject({
			available: true,
			binary: true,
			docker: true,
			deferredDownload: false,
		});
		expect(of("aarch64-unknown-linux-gnu")).toMatchObject({
			available: false,
		});
		expect(of("x86_64-apple-darwin")).toMatchObject({
			available: false,
			mac: true,
		});
		expect(of("aarch64-apple-darwin")).toMatchObject({
			available: true,
			docker: false,
			deferredDownload: true,
			mac: true,
		});
	});

	test("modes, the fallback mode and what this window packs", () => {
		const linux = of("x86_64-unknown-linux-gnu");
		const mac = of("aarch64-apple-darwin");
		expect(modeAvailable(linux, "both")).toBe(true);
		expect(modeAvailable(mac, "docker")).toBe(false);
		expect(modeAvailable(mac, "both")).toBe(false);
		expect(modeAvailable(undefined, "binary")).toBe(false);
		expect(mac && defaultMode(mac)).toBe("binary");
		expect(packsAgent(linux, "binary")).toBe(true);
		expect(packsAgent(linux, "docker")).toBe(false);
		expect(packsAgent(mac, "binary")).toBe(false);
	});

	test("without a verified release nothing can be chosen", () => {
		expect(targetOptions(undefined).every((option) => !option.available)).toBe(
			true,
		);
	});
});

describe("the step to show", () => {
	const options = targetOptions(
		release([["x86_64-unknown-linux-gnu", MIB]], null),
	);
	const facts = (draft: SetupDraft, patch: Partial<StepFacts> = {}) => ({
		draft,
		nowS: NOW,
		creating: false,
		options,
		checksPass: true,
		...patch,
	});

	test("nothing past the first missing choice opens before the device is registered", () => {
		expect(resolveStep(6, facts(freshDraft()))).toBe(1);
		expect(resolveStep(6, facts(freshDraft({ name: "edge" })))).toBe(2);
		const chosen = freshDraft({
			name: "edge",
			target: "x86_64-unknown-linux-gnu",
			mode: "binary",
		});
		expect(resolveStep(6, facts(chosen))).toBe(4);
		expect(resolveStep(3, facts(chosen))).toBe(3);
		expect(resolveStep(undefined, facts({ ...chosen, step: 2 }))).toBe(2);
	});

	test("checks that fail for good bring every step back to the check", () => {
		expect(
			resolveStep(
				3,
				facts(freshDraft({ name: "edge" }), { checksPass: false }),
			),
		).toBe(0);
	});

	test("a platform the release no longer offers counts as not chosen", () => {
		const stale = freshDraft({
			name: "edge",
			target: "aarch64-apple-darwin",
			mode: "binary",
		});
		expect(resolveStep(4, facts(stale))).toBe(2);
	});

	test("a running creation holds the Create step; a registered device locks the earlier ones", () => {
		expect(resolveStep(1, facts(freshDraft(), { creating: true }))).toBe(4);
		expect(resolveStep(2, facts(created()))).toBe(5);
		expect(resolveStep(4, facts(created()))).toBe(4);
		expect(resolveStep(2, facts(created({ resumed: true })))).toBe(6);
		expect(lockedOut(2, created())).toBe(true);
		expect(lockedOut(5, created())).toBe(false);
		expect(lockedOut(5, created({ resumed: true }))).toBe(true);
		expect(lockedOut(undefined, created())).toBe(false);
	});

	test("an expired package shows the waiting step; a registered device never expires", () => {
		const lapsed = created({
			created: {
				deviceId: "device-1",
				createdAt: NOW - 90_000,
				expiresAt: NOW - 3_600,
			},
		});
		expect(isExpired(lapsed, NOW)).toBe(true);
		expect(resolveStep(5, facts(lapsed))).toBe(7);
		expect(isExpired({ ...lapsed, registeredAt: NOW - 4_000 }, NOW)).toBe(
			false,
		);
		expect(isExpired({ ...lapsed, cancelledAt: NOW - 4_000 }, NOW)).toBe(false);
	});

	test("a package has to be started before the agent release inside it runs out", () => {
		const made = {
			deviceId: "device-1",
			createdAt: NOW - 60,
			expiresAt: NOW + 86_340,
		};
		expect(startBy(made)).toBe(NOW + 86_340);
		const cut = { ...made, releaseEndsAt: NOW + 7_200 };
		expect(startBy(cut)).toBe(NOW + 7_200);
		const draft = created({ created: cut });
		expect(isExpired(draft, NOW + 7_199)).toBe(false);
		expect(isExpired(draft, NOW + 7_200)).toBe(true);
		expect(resolveStep(5, { ...facts(draft), nowS: NOW + 7_200 })).toBe(7);
	});

	test("a release that ends before a package made now would is the cut-off; a longer one is none", () => {
		const day = 86_400;
		expect(releaseCutoff(undefined, NOW, day)).toBeUndefined();
		expect(releaseCutoff({ expires_at: NOW + day }, NOW, day)).toBeUndefined();
		expect(releaseCutoff({ expires_at: NOW + day - 1 }, NOW, day)).toBe(
			NOW + day - 1,
		);
		expect(releaseCutoff({ expires_at: NOW + 365 * day }, NOW, day)).toBe(
			undefined,
		);
	});
});

describe("drafts of this window", () => {
	const SCOPE = "scope-a";
	const store = new Map<string, string>();
	const original = Object.getOwnPropertyDescriptor(
		globalThis,
		"sessionStorage",
	);

	beforeEach(() => {
		store.clear();
		Object.defineProperty(globalThis, "sessionStorage", {
			configurable: true,
			value: {
				getItem: (key: string) => store.get(key) ?? null,
				setItem: (key: string, value: string) => void store.set(key, value),
				removeItem: (key: string) => void store.delete(key),
			},
		});
	});
	afterEach(() => {
		if (original) Object.defineProperty(globalThis, "sessionStorage", original);
		else Reflect.deleteProperty(globalThis, "sessionStorage");
	});

	test("are kept per account scope and dropped when unreadable", () => {
		writeDraft(SCOPE, NEW_SLOT, created());
		expect(readDrafts(SCOPE)[NEW_SLOT]?.name).toBe("factory-line-4");
		expect(readDrafts("scope-b")).toEqual({});
		store.set(
			"flow-like.devices.setup.scope-b",
			JSON.stringify({ new: { step: 99, name: 1 }, ok: created() }),
		);
		expect(Object.keys(readDrafts("scope-b"))).toEqual(["ok"]);
		store.set("flow-like.devices.setup.scope-b", "{not json");
		expect(readDrafts("scope-b")).toEqual({});
		writeDraft(SCOPE, NEW_SLOT, undefined);
		expect(store.has(`flow-like.devices.setup.${SCOPE}`)).toBe(false);
	});

	test("never carry anything but the wizard's own fields", () => {
		writeDraft(SCOPE, NEW_SLOT, {
			...created(),
			password: "violet-harbour-lantern-42",
		} as SetupDraft);
		const stored = store.get(`flow-like.devices.setup.${SCOPE}`) ?? "";
		expect(stored).toContain("factory-line-4");
		expect(stored).not.toContain("violet");
	});

	test("keep the end of the agent release a package carries", () => {
		const draft = created();
		if (!draft.created) throw new Error("The draft has a package");
		const cut = { ...draft.created, releaseEndsAt: NOW + 7_200 };
		writeDraft(SCOPE, NEW_SLOT, { ...draft, created: cut });
		expect(readDrafts(SCOPE)[NEW_SLOT]?.created?.releaseEndsAt).toBe(
			NOW + 7_200,
		);
	});

	test("a setup made here is found by its enrollment id too", () => {
		const drafts = { [NEW_SLOT]: created() };
		expect(findDraft(drafts, "enrollment-1")?.slot).toBe(NEW_SLOT);
		expect(findDraft(drafts, "enrollment-2")).toBeUndefined();
		expect(findDraft({}, NEW_SLOT)).toBeUndefined();
	});

	test("only a setup of this window that still waits for its device is offered again", () => {
		const waiting = created();
		const drafts = {
			a: waiting,
			b: created({ checkedInAt: NOW }),
			c: created({ cancelledAt: NOW }),
			d: created({ resumed: true }),
			e: created({ registeredAt: NOW }),
		};
		expect(waitingSetup(drafts, NOW)?.draft).toBe(waiting);
		expect(waitingSetup(drafts, NOW, "a")).toBeUndefined();
		expect(waitingSetup(drafts, NOW + 90_000)).toBeUndefined();
	});
});
