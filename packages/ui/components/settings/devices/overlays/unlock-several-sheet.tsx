"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CircleCheck,
	CircleSlash,
	KeyRound,
	LoaderCircle,
	LockOpen,
	type LucideIcon,
	OctagonX,
} from "lucide-react";
import {
	type KeyboardEvent,
	type ReactNode,
	type Ref,
	useCallback,
	useEffect,
	useId,
	useMemo,
	useRef,
	useState,
} from "react";
import { fleetFacts } from "../../../../lib/device-management/model/device-view";
import type { AttentionInput } from "../../../../lib/device-management/model/types";
import type {
	KeySessionManager,
	KeySessionSnapshot,
	UnlockManyOutcome,
} from "../../../../lib/device-management/workspace/types";
import { enumLabel } from "../copy/enum-labels";
import type { DevicesT } from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import { DvSheet } from "../primitives/dv-sheet";
import { CheckField, Field, SecretInput } from "../primitives/form-fields";
import { InlineResult } from "../primitives/inline-result";
import { cx } from "../primitives/tone";
import { useDeviceWorkspace } from "../workspace/device-workspace-provider";
import { useAttentionState } from "../workspace/use-attention";
import { useKeyChip } from "../workspace/use-keys";
import type { OverlaySheetProps } from "./area-overlays";

/**
 * The shadcn input resets the outline style, so the primitive's 2 px focus
 * ring is never drawn; the wrapper turns it back on for the password field.
 */
export const FOCUS_RING = "[&_input:focus-visible]:outline-solid";

/** Enter in the password field submits, like the primary button. */
export function submitOnEnter(submit: () => void) {
	return (event: KeyboardEvent<HTMLInputElement>) => {
		if (event.key !== "Enter") return;
		event.preventDefault();
		submit();
	};
}

/* One cancellable run at a time. */

export interface RunToken {
	signal: AbortSignal;
	/** False once the sheet closed or a newer run began: its late results are dropped. */
	current(): boolean;
	end(): void;
}

/** Closing the sheet aborts the run in hand, so a late completion changes nothing on screen. */
export function useRunGuard(): () => RunToken {
	const run = useRef<AbortController | null>(null);
	useEffect(
		() => () => {
			run.current?.abort();
			run.current = null;
		},
		[],
	);
	return useMemo(
		() => () => {
			const controller = new AbortController();
			run.current = controller;
			return {
				signal: controller.signal,
				current: () => run.current === controller,
				end: () => {
					if (run.current === controller) run.current = null;
				},
			};
		},
		[],
	);
}

/* Attempts: what one device's line says while and after the password was tried on it. */

export type UnlockAttempt = "trying" | UnlockManyOutcome;
export type Attempts = Readonly<Record<string, UnlockAttempt>>;

/**
 * Tries the password on each device in turn (IA §6.4.4) and reports every
 * line as it changes: the device in hand reads "Trying…".
 */
export async function tryPassword(
	keys: KeySessionManager,
	deviceIds: readonly string[],
	password: string,
	signal: AbortSignal,
	report: (update: Attempts) => void,
): Promise<void> {
	const [first] = deviceIds;
	if (first === undefined) return;
	report({ [first]: "trying" });
	const each = (deviceId: string, outcome: UnlockManyOutcome) => {
		const next = deviceIds[deviceIds.indexOf(deviceId) + 1];
		report({ [deviceId]: outcome, ...(next ? { [next]: "trying" } : {}) });
	};
	await keys
		.unlockMany([...deviceIds], password, each, signal)
		.catch(() => undefined);
}

interface AttemptLook {
	icon: LucideIcon;
	tone: string;
	text: string;
	spin?: boolean;
}

/** SPEC §3.10: Trying… → Unlocked / "This password didn't open these keys" / "Skipped: …". */
export function attemptLook(t: DevicesT, attempt: UnlockAttempt): AttemptLook {
	const looks: Record<UnlockAttempt, AttemptLook> = {
		trying: {
			icon: LoaderCircle,
			tone: "text-info",
			text: t("devices:overlay.several.trying", "Trying…"),
			spin: true,
		},
		unlocked: {
			icon: CircleCheck,
			tone: "text-good",
			text: t("devices:overlay.several.unlocked", "Unlocked"),
		},
		wrong_password: {
			icon: OctagonX,
			tone: "text-critical",
			text: t(
				"devices:overlay.several.wrongPassword",
				"This password didn't open these keys",
			),
		},
		no_vault: {
			icon: CircleSlash,
			tone: "text-muted-foreground",
			text: t("devices:overlay.several.noKeys", "Skipped: no keys here"),
		},
		held_elsewhere: {
			icon: CircleSlash,
			tone: "text-muted-foreground",
			text: t(
				"devices:overlay.several.heldElsewhere",
				"Skipped: unlocked in another window",
			),
		},
		error: {
			icon: OctagonX,
			tone: "text-critical",
			text: t(
				"devices:overlay.several.failed",
				"Couldn't be unlocked. Open Unlock for this device to see why.",
			),
		},
	};
	return looks[attempt];
}

interface LineStatusProps {
	attempt: UnlockAttempt | undefined;
	/** What the line says before the password was tried: the kind of keys, or why it is skipped. */
	children: ReactNode;
}

/** The right-hand side of a device's line. */
export function LineStatus({ attempt, children }: Readonly<LineStatusProps>) {
	const { t } = useTranslation("devices");
	if (!attempt)
		return <span className="text-sm text-muted-foreground">{children}</span>;
	const look = attemptLook(t, attempt);
	const Icon = look.icon;
	return (
		<span
			data-attempt={attempt}
			className={cx("inline-flex items-start gap-1.5 text-sm", look.tone)}
		>
			<Icon
				aria-hidden
				className={cx(
					"mt-0.5 size-4 shrink-0",
					look.spin && "animate-spin motion-reduce:animate-none",
				)}
			/>
			{look.text}
		</span>
	);
}

/* Ticks: every listed device is selected until it is unticked. */

export type Unticked = Readonly<Record<string, true>>;

export function withTick(
	unticked: Unticked,
	deviceId: string,
	checked: boolean,
): Unticked {
	const { [deviceId]: _, ...rest } = unticked;
	return checked ? rest : { ...rest, [deviceId]: true };
}

export interface DeviceLineProps {
	fieldId: string;
	deviceId: string;
	name: string;
	checked: boolean;
	disabled: boolean;
	status: ReactNode;
	onCheckedChange(checked: boolean): void;
}

/** One device with its checkbox on the left and its state on the right. */
export function DeviceLine(props: Readonly<DeviceLineProps>) {
	return (
		<li
			data-device={props.deviceId}
			className="flex flex-wrap items-center gap-x-3 gap-y-1.5 border-t border-hairline py-2 first:border-t-0"
		>
			<CheckField
				id={props.fieldId}
				checked={props.checked}
				disabled={props.disabled}
				onCheckedChange={props.onCheckedChange}
				className="min-w-0"
			>
				<span className="font-mono wrap-anywhere">{props.name}</span>
			</CheckField>
			<span className="min-w-2 flex-1" />
			{props.status}
		</li>
	);
}

/* Candidates: every device of the hub's list that is not open. */

type CandidateKind =
	| "locked"
	| "revoked"
	| "held_elsewhere"
	| "blocked"
	| "none";

interface Candidate {
	deviceId: string;
	name: string;
	kind: CandidateKind;
	role?: KeySessionSnapshot["role"];
}

/** Lines for devices without keys here are capped: at 200 devices they would bury the ones a password can open (R11). */
const NO_KEYS_CAP = 3;

const KIND_OF_STATE: Record<KeySessionSnapshot["state"], CandidateKind> = {
	none: "none",
	stale: "revoked",
	held_elsewhere: "held_elsewhere",
	blocked: "blocked",
	locked: "locked",
	unlocking: "locked",
	unlocked: "locked",
};

/** A key session only knows "revoked" after a pre-flight; the hub row says it at once. */
function kindOf(
	active: boolean,
	session: KeySessionSnapshot | undefined,
): CandidateKind {
	const kind = session ? KIND_OF_STATE[session.state] : "none";
	return kind !== "none" && !active ? "revoked" : kind;
}

/**
 * Keys for a device the hub doesn't list (an access request that isn't
 * approved yet) are left out: there is nothing to read with them. A device
 * this sheet opened stays listed with its result.
 */
function candidatesOf(
	input: AttentionInput,
	sessions: readonly KeySessionSnapshot[],
	attempts: Attempts = {},
): Candidate[] {
	const byId = new Map(sessions.map((row) => [row.deviceId, row]));
	const rows: Candidate[] = [];
	for (const device of fleetFacts(input).devices) {
		const session = byId.get(device.id);
		const open = session?.state === "unlocked" && !attempts[device.id];
		if (open) continue;
		rows.push({
			deviceId: device.id,
			name: device.name,
			kind: kindOf(device.active, session),
			...(session ? { role: session.role } : {}),
		});
	}
	return rows;
}

const canTry = (row: Candidate) => row.kind === "locked";

export interface LockedDevice {
	deviceId: string;
	name: string;
	role: KeySessionSnapshot["role"];
}

/** Devices a password can open now: listed by the hub, not revoked, keys here and closed. */
export function lockedDevices(
	input: AttentionInput,
	sessions: readonly KeySessionSnapshot[],
): LockedDevice[] {
	return candidatesOf(input, sessions).flatMap((row) =>
		canTry(row) && row.role
			? [{ deviceId: row.deviceId, name: row.name, role: row.role }]
			: [],
	);
}

function skippedText(t: DevicesT, kind: CandidateKind): string {
	const texts: Record<Exclude<CandidateKind, "locked">, string> = {
		revoked: t(
			"devices:overlay.several.skippedStale",
			"Skipped: keys for a revoked device",
		),
		held_elsewhere: attemptLook(t, "held_elsewhere").text,
		blocked: t(
			"devices:overlay.several.skippedIdentity",
			"Skipped: its identity changed",
		),
		none: attemptLook(t, "no_vault").text,
	};
	return kind === "locked" ? "" : texts[kind];
}

/** Before a try: the kind of keys for a device the password can open, else why it is skipped. */
function candidateNote(t: DevicesT, row: Candidate): string {
	return canTry(row) && row.role
		? enumLabel(t, "vaultKind", row.role)
		: skippedText(t, row.kind);
}

type Phase = "form" | "running" | "done";

interface Outcome {
	tried: number;
	unlocked: number;
	/** Names of the devices the password didn't open. */
	stillLocked: string[];
}

function outcomeOf(rows: readonly Candidate[], attempts: Attempts): Outcome {
	const finished = rows.filter((row) => {
		const attempt = attempts[row.deviceId];
		return attempt !== undefined && attempt !== "trying";
	});
	const stillLocked = finished
		.filter((row) => attempts[row.deviceId] !== "unlocked")
		.map((row) => row.name);
	return {
		tried: finished.length,
		unlocked: finished.length - stillLocked.length,
		stillLocked,
	};
}

function SeveralResult({ outcome }: Readonly<{ outcome: Outcome }>) {
	const { t, i18n } = useTranslation("devices");
	const { tried, unlocked, stillLocked } = outcome;
	const names = new Intl.ListFormat(i18n?.language, {
		type: "conjunction",
	}).format(stillLocked);
	const summary = t("overlay.several.result", {
		count: tried,
		unlocked,
		defaultValue_one:
			"{{unlocked, number}} of {{count, number}} device unlocked.",
		defaultValue_other:
			"{{unlocked, number}} of {{count, number}} devices unlocked.",
	});
	if (stillLocked.length === 0)
		return <InlineResult tone="good">{summary}</InlineResult>;
	return (
		<InlineResult tone="warning">
			{summary}{" "}
			{t("overlay.several.stillLocked", "Still locked: {{names}}.", { names })}
		</InlineResult>
	);
}

interface NothingToTryProps {
	/** Keys exist here, all of them open; false when this computer holds none. */
	hasKeys: boolean;
	onOpenKeys(): void;
}

function NothingToTry({ hasKeys, onOpenKeys }: Readonly<NothingToTryProps>) {
	const { t } = useTranslation("devices");
	return (
		<div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 text-muted-foreground">
			<p className="min-w-0 flex-1 text-ui">
				{hasKeys
					? t(
							"overlay.several.allOpen",
							"Every device with keys here is already unlocked.",
						)
					: t(
							"overlay.several.noKeysHere",
							"There are no device keys on this computer for this account.",
						)}
			</p>
			<DvButton size="xs" icon={KeyRound} onClick={onOpenKeys}>
				{t("overlay.several.openKeys", "Open Keys & recovery")}
			</DvButton>
		</div>
	);
}

interface SeveralFootProps {
	phase: Phase;
	ready: boolean;
	count: number;
	canRetry: boolean;
	doneButton: Ref<HTMLButtonElement>;
	onClose(): void;
	onRetry(): void;
	onSubmit(): void;
}

function SeveralFoot(props: Readonly<SeveralFootProps>) {
	const { t } = useTranslation("devices");
	if (props.phase === "done")
		return (
			<>
				{props.canRetry ? (
					<DvButton onClick={props.onRetry}>
						{t("overlay.several.retry", "Try another password")}
					</DvButton>
				) : null}
				<DvButton
					ref={props.doneButton}
					variant="primary"
					onClick={props.onClose}
				>
					{t("overlay.several.done", "Done")}
				</DvButton>
			</>
		);
	return (
		<>
			<DvButton onClick={props.onClose}>
				{t("overlay.several.cancel", "Cancel")}
			</DvButton>
			<DvButton
				variant="primary"
				icon={LockOpen}
				busy={props.phase === "running"}
				aria-disabled={props.phase === "form" && !props.ready}
				onClick={props.onSubmit}
			>
				{t("overlay.several.submit", {
					count: props.count,
					defaultValue_one: "Unlock {{count, number}} device",
					defaultValue_other: "Unlock {{count, number}} devices",
				})}
			</DvButton>
		</>
	);
}

/** IA §6.4.4: one password tried on each selected device's keys, in turn; failures are skipped and stay listed. */
export function UnlockSeveralSheet({
	onNavigate,
	onClose,
}: Readonly<OverlaySheetProps>) {
	const { t } = useTranslation("devices");
	const { keys } = useDeviceWorkspace();
	const { input } = useAttentionState();
	const { sessions } = useKeyChip();
	const begin = useRunGuard();
	const formId = useId();
	const passwordId = `${formId}-password`;
	const [phase, setPhase] = useState<Phase>("form");
	const [password, setPassword] = useState("");
	const [submitted, setSubmitted] = useState(0);
	const [unticked, setUnticked] = useState<Unticked>({});
	const [attempts, setAttempts] = useState<Attempts>({});
	const doneButton = useRef<HTMLButtonElement | null>(null);

	const rows = useMemo(
		() => candidatesOf(input, sessions, attempts),
		[input, sessions, attempts],
	);
	const withKeys = rows.filter((row) => row.kind !== "none");
	const withoutKeys = rows.filter((row) => row.kind === "none");
	/** Devices an earlier password of this sheet opened stay listed, but are not tried again. */
	const opened = (row: Candidate) => attempts[row.deviceId] === "unlocked";
	const pending = (row: Candidate) => canTry(row) && !opened(row);
	const selected = withKeys.filter(
		(row) => pending(row) && !unticked[row.deviceId],
	);
	const editable = phase === "form";
	const askPassword = editable && withKeys.some(pending);
	const ready = editable && selected.length > 0 && password.length > 0;
	const outcome = outcomeOf(withKeys, attempts);
	const hidden = Math.max(0, withoutKeys.length - NO_KEYS_CAP);

	/* The sheet's content mounts after this component's first effects, so the field takes the focus when it appears. */
	const focusOnMount = useCallback((field: HTMLInputElement | null) => {
		field?.focus();
	}, []);
	useEffect(() => {
		if (askPassword) globalThis.document?.getElementById(passwordId)?.focus();
	}, [askPassword, passwordId]);

	useEffect(() => {
		if (phase === "done") doneButton.current?.focus({ preventScroll: true });
	}, [phase]);

	const submit = async () => {
		if (!ready) return;
		const secret = password;
		const ids = selected.map((row) => row.deviceId);
		const run = begin();
		setPassword("");
		setSubmitted(ids.length);
		setPhase("running");
		await tryPassword(keys, ids, secret, run.signal, (update) => {
			if (run.current())
				setAttempts((previous) => ({ ...previous, ...update }));
		});
		if (!run.current()) return;
		run.end();
		setPhase("done");
	};

	const retry = () => {
		setAttempts((previous) =>
			Object.fromEntries(
				Object.entries(previous).filter(
					([, attempt]) => attempt === "unlocked",
				),
			),
		);
		setPhase("form");
	};

	const openKeys = () => {
		onClose();
		onNavigate({ screen: "keys" });
	};

	return (
		<DvSheet
			open
			onOpenChange={(open) => {
				if (!open) onClose();
			}}
			icon={LockOpen}
			title={t("overlay.several.title", "Unlock several devices")}
			sub={t(
				"overlay.several.subtitle",
				"One password, tried on each selected device's keys on this computer.",
			)}
			footNote={t(
				"overlay.several.footNote",
				"Each device takes about a second. Devices where the password fails stay locked.",
			)}
			foot={
				<SeveralFoot
					phase={phase}
					ready={ready}
					count={editable ? selected.length : submitted}
					canRetry={outcome.stillLocked.length > 0}
					doneButton={doneButton}
					onClose={onClose}
					onRetry={retry}
					onSubmit={submit}
				/>
			}
		>
			<p className="max-w-[72ch] text-sm">
				{t(
					"overlay.several.intro",
					"Tries one password against each device's keys on this computer, one after another.",
				)}
			</p>
			{phase === "done" ? <SeveralResult outcome={outcome} /> : null}
			{editable && !askPassword ? (
				<NothingToTry hasKeys={sessions.length > 0} onOpenKeys={openKeys} />
			) : null}
			<ul
				aria-label={t("overlay.several.listLabel", "Devices to unlock")}
				className="flex max-h-[min(46dvh,420px)] shrink-0 flex-col overflow-y-auto"
			>
				{[...withKeys, ...withoutKeys.slice(0, NO_KEYS_CAP)].map((row) => {
					const attempt = attempts[row.deviceId];
					const selectable = canTry(row) || attempt !== undefined;
					return (
						<DeviceLine
							key={row.deviceId}
							fieldId={`${formId}-${row.deviceId}`}
							deviceId={row.deviceId}
							name={row.name}
							checked={selectable && !unticked[row.deviceId]}
							disabled={!selectable || !editable || opened(row)}
							status={
								<LineStatus attempt={attempt}>
									{candidateNote(t, row)}
								</LineStatus>
							}
							onCheckedChange={(checked) =>
								setUnticked((previous) =>
									withTick(previous, row.deviceId, checked),
								)
							}
						/>
					);
				})}
				{hidden > 0 ? (
					<li className="border-t border-hairline py-2 text-xs text-muted-foreground">
						{t("overlay.several.moreWithoutKeys", {
							count: hidden,
							defaultValue_one:
								"{{count, number}} more device has no keys on this computer.",
							defaultValue_other:
								"{{count, number}} more devices have no keys on this computer.",
						})}
					</li>
				) : null}
			</ul>
			{phase === "done" ? null : (
				<Field
					id={passwordId}
					label={t("overlay.several.passwordLabel", "Device password")}
					hint={t(
						"overlay.several.passwordHint",
						"It never leaves this computer. Devices where it fails stay locked.",
					)}
				>
					<SecretInput
						ref={focusOnMount}
						value={password}
						onValueChange={setPassword}
						disabled={!askPassword}
						className={FOCUS_RING}
						onKeyDown={submitOnEnter(() => void submit())}
					/>
				</Field>
			)}
		</DvSheet>
	);
}
