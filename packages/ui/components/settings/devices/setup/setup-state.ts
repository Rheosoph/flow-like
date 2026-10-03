import { z } from "zod";
import {
	SETUP_STEPS,
	type SetupStep,
} from "../../../../lib/device-management/model/types";
import {
	type ReleaseTarget,
	type StandaloneRelease,
	standalonePackageDownloadsBinary,
	standalonePackageModes,
} from "../../../../lib/device-management/package";
import type { AccountBackupOutcome } from "../../../../lib/device-management/setup";

/* The wizard's own state and rules (SPEC §5.5). No React and no I/O beyond sessionStorage; never the password, the package or the key backup. */

/** Length in UTF-8 bytes: the unit the hub and the key vault count in. */
export const utf8Bytes = (value: string) =>
	new TextEncoder().encode(value).length;

export type SetupMode = "binary" | "docker" | "both";
export type BackupOutcome = AccountBackupOutcome | "off";

export const NAME_MAX_BYTES = 256;
export const PASSWORD_MIN_BYTES = 12;
export const PASSWORD_MAX_BYTES = 4096;
export const STEP_COUNT = SETUP_STEPS.length;
/** The step that registers the device; earlier steps lock once it ran. */
export const CREATE_STEP: SetupStep = 4;
export const SAVE_STEP: SetupStep = 5;
export const START_STEP: SetupStep = 6;
export const WAIT_STEP: SetupStep = 7;

export const SETUP_TARGETS: readonly ReleaseTarget[] = [
	"x86_64-unknown-linux-gnu",
	"aarch64-unknown-linux-gnu",
	"x86_64-apple-darwin",
	"aarch64-apple-darwin",
];
const MODES: readonly SetupMode[] = ["binary", "docker", "both"];

export const isMacTarget = (target: ReleaseTarget | undefined) =>
	target?.endsWith("apple-darwin") === true;

export interface CreatedSetup {
	/** Absent only when the hub's answer could not be read back; the setup can then not be cancelled from here. */
	enrollmentId?: string;
	deviceId: string;
	/** Unix seconds. */
	createdAt: number;
	expiresAt: number;
	/**
	 * Unix seconds at which the agent release inside the package runs out, when
	 * that is before the package's own end: the device refuses the package from
	 * then on. Unknown for a setup this window did not create.
	 */
	releaseEndsAt?: number;
	/** Unknown for a setup this window did not create. */
	outcome?: BackupOutcome;
}

/** The time a package has to be started by: its own end, or the earlier end of the agent release it carries. */
export const startBy = (created: CreatedSetup) =>
	created.releaseEndsAt ?? created.expiresAt;

/** The end of the agent release when a package made now would outlive it: such a package has to be started before then. */
export function releaseCutoff(
	release: Pick<StandaloneRelease, "expires_at"> | undefined,
	nowS: number,
	lifetimeS: number,
): number | undefined {
	return release && release.expires_at < nowS + lifetimeS
		? release.expires_at
		: undefined;
}

export interface SetupDraft {
	step: SetupStep;
	name: string;
	target?: ReleaseTarget;
	mode?: SetupMode;
	backup: boolean;
	created?: CreatedSetup;
	keySaved: boolean;
	packageSaved: boolean;
	/** A window without the package: "I have the package from when it was created". */
	acknowledged: boolean;
	/** Which commands step 6 shows when the package holds both. */
	startMode?: "binary" | "docker";
	/** Unix seconds. */
	cancelledAt?: number;
	registeredAt?: number;
	checkedInAt?: number;
	/** Opened from a pending setup instead of this window's own run. */
	resumed: boolean;
}

export function freshDraft(patch: Partial<SetupDraft> = {}): SetupDraft {
	return {
		step: 0,
		name: "",
		backup: true,
		keySaved: false,
		packageSaved: false,
		acknowledged: false,
		resumed: false,
		...patch,
	};
}

const seconds = z.number().int().nonnegative();
const draftSchema = z.object({
	step: z.number().int().min(0).max(7),
	name: z.string().max(1024),
	target: z
		.enum(SETUP_TARGETS as [ReleaseTarget, ...ReleaseTarget[]])
		.optional(),
	mode: z.enum(MODES as [SetupMode, ...SetupMode[]]).optional(),
	backup: z.boolean(),
	created: z
		.object({
			enrollmentId: z.string().min(1).max(128).optional(),
			deviceId: z.string().min(1).max(128),
			createdAt: seconds,
			expiresAt: seconds,
			releaseEndsAt: seconds.optional(),
			outcome: z.enum(["saved", "local_only", "limit", "off"]).optional(),
		})
		.optional(),
	keySaved: z.boolean(),
	packageSaved: z.boolean(),
	acknowledged: z.boolean(),
	startMode: z.enum(["binary", "docker"]).optional(),
	cancelledAt: seconds.optional(),
	registeredAt: seconds.optional(),
	checkedInAt: seconds.optional(),
	resumed: z.boolean(),
});

/** The slot of a setup started here; a resumed one is filed under its enrollment id. */
export const NEW_SLOT = "new";
const STORAGE_PREFIX = "flow-like.devices.setup.";

function sessionStore(): Storage | undefined {
	try {
		return globalThis.sessionStorage;
	} catch {
		return undefined;
	}
}

/** Drafts of this window, per account scope. Unreadable entries are dropped. */
export function readDrafts(scopeKey: string): Record<string, SetupDraft> {
	const raw = sessionStore()?.getItem(STORAGE_PREFIX + scopeKey);
	if (!raw) return {};
	try {
		const parsed = z.record(z.string(), z.unknown()).parse(JSON.parse(raw));
		return Object.fromEntries(
			Object.entries(parsed).flatMap(([slot, value]) => {
				const draft = draftSchema.safeParse(value);
				return draft.success ? [[slot, draft.data as SetupDraft]] : [];
			}),
		);
	} catch {
		return {};
	}
}

export function writeDraft(
	scopeKey: string,
	slot: string,
	draft: SetupDraft | undefined,
): void {
	const store = sessionStore();
	if (!store) return;
	const drafts = readDrafts(scopeKey);
	// Only the fields of the schema are ever written: nothing else can reach the store through a draft.
	const clean = draft ? draftSchema.safeParse(draft) : undefined;
	if (clean?.success) drafts[slot] = clean.data as SetupDraft;
	else delete drafts[slot];
	try {
		if (Object.keys(drafts).length)
			store.setItem(STORAGE_PREFIX + scopeKey, JSON.stringify(drafts));
		else store.removeItem(STORAGE_PREFIX + scopeKey);
	} catch {
		// A full or blocked session store only costs the resume after a reload.
	}
}

/** A setup made in this window is also found by its enrollment id (Pending setups → Start instructions). */
export function findDraft(
	drafts: Record<string, SetupDraft>,
	slot: string,
): { slot: string; draft: SetupDraft } | undefined {
	const direct = drafts[slot];
	if (direct) return { slot, draft: direct };
	if (slot === NEW_SLOT) return undefined;
	const own = drafts[NEW_SLOT];
	return own?.created?.enrollmentId === slot
		? { slot: NEW_SLOT, draft: own }
		: undefined;
}

export interface WaitingSetup {
	enrollmentId: string;
	draft: SetupDraft;
}

/** The newest setup this window made that still waits for its device, other than the one in the given slot. */
export function waitingSetup(
	drafts: Readonly<Record<string, SetupDraft>>,
	nowS: number,
	currentSlot: string = NEW_SLOT,
): WaitingSetup | undefined {
	return Object.entries(drafts)
		.flatMap(([slot, draft]) => {
			const enrollmentId = draft.created?.enrollmentId;
			return slot !== currentSlot &&
				enrollmentId &&
				!draft.resumed &&
				!isClosed(draft) &&
				draft.registeredAt === undefined &&
				!isExpired(draft, nowS)
				? [{ enrollmentId, draft }]
				: [];
		})
		.sort(
			(a, b) =>
				(b.draft.created?.createdAt ?? 0) - (a.draft.created?.createdAt ?? 0),
		)[0];
}

/* Validation: the hub signs the name into the package and rejects what `bounded_text(name, 256)` rejects. */

export type NameIssue =
	| { code: "empty" }
	| { code: "control" }
	| { code: "edge_space" }
	| { code: "too_long"; bytes: number };

function hasControlCharacter(value: string): boolean {
	for (const char of value) {
		const point = char.codePointAt(0) ?? 0;
		if (point < 0x20 || (point >= 0x7f && point <= 0x9f)) return true;
	}
	return false;
}

export function nameIssue(name: string): NameIssue | undefined {
	if (!name) return { code: "empty" };
	if (hasControlCharacter(name)) return { code: "control" };
	if (name !== name.trim()) return { code: "edge_space" };
	const bytes = utf8Bytes(name);
	return bytes > NAME_MAX_BYTES ? { code: "too_long", bytes } : undefined;
}

/** The existing device or setup with the same name, compared without case. */
export function duplicateName(
	name: string,
	taken: readonly string[],
): string | undefined {
	const wanted = name.toLowerCase();
	return wanted
		? taken.find((other) => other.toLowerCase() === wanted)
		: undefined;
}

/** The next free name after one that ends in a number ("factory-line-3" → "factory-line-4"). */
export function suggestName(
	taken: readonly string[],
): { name: string; follows: string } | undefined {
	const used = new Set(taken.map((name) => name.toLowerCase()));
	for (const follows of taken) {
		const match = /^(.*?)(\d+)$/.exec(follows);
		if (!match) continue;
		const [, stem = "", digits = ""] = match;
		for (let next = Number(digits) + 1; next < Number(digits) + 50; next++) {
			const name = `${stem}${String(next).padStart(digits.length, "0")}`;
			if (!used.has(name.toLowerCase())) return { name, follows };
		}
	}
	return undefined;
}

export type PasswordIssue =
	| { code: "empty" }
	| { code: "too_short"; bytes: number }
	| { code: "too_long"; bytes: number };

export function passwordIssue(password: string): PasswordIssue | undefined {
	if (!password) return { code: "empty" };
	const bytes = utf8Bytes(password);
	if (bytes < PASSWORD_MIN_BYTES) return { code: "too_short", bytes };
	return bytes > PASSWORD_MAX_BYTES ? { code: "too_long", bytes } : undefined;
}

export type RepeatIssue = "empty" | "mismatch";

export function repeatIssue(
	password: string,
	repeat: string,
): RepeatIssue | undefined {
	if (!repeat) return "empty";
	return repeat === password ? undefined : "mismatch";
}

/* Platform and mode, as the verified release offers them. */

export interface TargetOption {
	target: ReleaseTarget;
	/** Size of the agent binary in bytes; absent when the release has none for this platform. */
	size?: number;
	binary: boolean;
	docker: boolean;
	available: boolean;
	/** Too large for the browser to pack: the device downloads the agent on first start. */
	deferredDownload: boolean;
	mac: boolean;
}

export function targetOptions(
	release: StandaloneRelease | undefined,
): TargetOption[] {
	return SETUP_TARGETS.map((target) => {
		const artifact = release?.artifacts.find((item) => item.target === target);
		const modes = release
			? standalonePackageModes(release, target)
			: { binary: false, docker: false };
		return {
			target,
			...(artifact ? { size: artifact.size } : {}),
			binary: modes.binary,
			docker: modes.docker,
			available: modes.binary || modes.docker,
			deferredDownload: release
				? standalonePackageDownloadsBinary(release, target)
				: false,
			mac: isMacTarget(target),
		};
	});
}

export function modeAvailable(
	option: TargetOption | undefined,
	mode: SetupMode | undefined,
): boolean {
	if (!option || !mode) return false;
	if (mode === "binary") return option.binary;
	if (mode === "docker") return option.docker;
	return option.binary && option.docker;
}

/** The mode a platform falls back to when the chosen one does not exist for it. */
export function defaultMode(option: TargetOption): SetupMode | undefined {
	if (option.binary) return "binary";
	return option.docker ? "docker" : undefined;
}

/** Whether this window downloads the agent into the package. */
export function packsAgent(
	option: TargetOption | undefined,
	mode: SetupMode | undefined,
): boolean {
	return !!option?.binary && mode !== "docker" && !option.deferredDownload;
}

/* File names: the device's name, reduced to what reads well in a shell. */

export function safeName(name: string): string {
	return (
		name.replace(/[^A-Za-z0-9._-]+/g, "-").replace(/^-+|-+$/g, "") || "device"
	);
}
export const packageFolder = (name: string) => `flow-like-${safeName(name)}`;
export const packageFile = (name: string) => `${packageFolder(name)}.zip`;
export const keyBackupFile = (name: string) =>
	`${packageFolder(name)}-keys.json`;

/* Step flow. */

/** The package can't be started any more: it ran out, or the agent release it carries did. */
export const isExpired = (draft: SetupDraft, nowS: number) =>
	!!draft.created &&
	!draft.cancelledAt &&
	!draft.checkedInAt &&
	!draft.registeredAt &&
	startBy(draft.created) <= nowS;

/** A finished or cancelled setup: opening the wizard again starts a new one. */
export const isClosed = (draft: SetupDraft) =>
	!!draft.cancelledAt || !!draft.checkedInAt;

/** The first step that may be shown once the device is registered. */
export const lockedBefore = (draft: SetupDraft): SetupStep =>
	draft.resumed ? START_STEP : CREATE_STEP;

export interface StepFacts {
	draft: SetupDraft;
	nowS: number;
	/** The package is being built in this window. */
	creating: boolean;
	options: readonly TargetOption[];
	/** Step 0 does not fail for good: the hub is ready (or still answering), the release is not rejected, there is room. */
	checksPass: boolean;
}

/**
 * The choices a later step needs, by step. The device password (step 3) is not
 * among them: it is never stored, so Create asks for it again after a reload.
 */
const CHOICE_MADE: readonly ((facts: StepFacts) => boolean)[] = [
	(facts) => facts.checksPass,
	({ draft }) => !nameIssue(draft.name),
	({ draft, options }) =>
		modeAvailable(
			options.find((item) => item.target === draft.target),
			draft.mode,
		),
];

/** Nothing past the first missing choice opens, and nothing past Create. */
function beforeRegistration(wanted: number, facts: StepFacts): SetupStep {
	const limit = Math.min(wanted, CREATE_STEP);
	const missing = CHOICE_MADE.findIndex(
		(made, step) => step < limit && !made(facts),
	);
	return (missing < 0 ? limit : missing) as SetupStep;
}

/** The earlier steps are locked; an expired package only has the waiting step left. */
function afterRegistration(
	wanted: number,
	draft: SetupDraft,
	nowS: number,
): SetupStep {
	if (isExpired(draft, nowS)) return WAIT_STEP;
	if (wanted >= lockedBefore(draft)) return wanted as SetupStep;
	return draft.resumed ? START_STEP : SAVE_STEP;
}

/** The step to show for a requested one (URL or stored). */
export function resolveStep(
	requested: number | undefined,
	facts: StepFacts,
): SetupStep {
	const { draft } = facts;
	const wanted = Math.max(0, Math.min(STEP_COUNT - 1, requested ?? draft.step));
	if (draft.cancelledAt) return wanted as SetupStep;
	if (draft.created) return afterRegistration(wanted, draft, facts.nowS);
	return facts.creating ? CREATE_STEP : beforeRegistration(wanted, facts);
}

/** Whether a step other than the shown one was asked for because earlier steps are locked. */
export function lockedOut(requested: number | undefined, draft: SetupDraft) {
	return (
		!!draft.created &&
		!draft.cancelledAt &&
		requested !== undefined &&
		requested < lockedBefore(draft)
	);
}
