"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { Fingerprint, Link2, LockOpen, RefreshCw } from "lucide-react";
import {
	type ReactNode,
	type Ref,
	useEffect,
	useId,
	useMemo,
	useRef,
	useState,
} from "react";
import {
	groupFingerprint,
	identityFingerprint,
} from "../../../../lib/device-management/fingerprint";
import {
	deviceLabel,
	fleetFacts,
} from "../../../../lib/device-management/model/device-view";
import type {
	AttentionKey,
	DeviceRow,
	DevicesRoute,
	FixAction,
	PreflightCode,
	PreflightId,
} from "../../../../lib/device-management/model/types";
import { keyErrorCode } from "../../../../lib/device-management/workspace/errors";
import type {
	DeviceWorkspace,
	KeySessionSnapshot,
	LiveState,
	Preflight,
	PreflightRow,
	UnlockStep,
} from "../../../../lib/device-management/workspace/types";
import { enumLabel } from "../copy/enum-labels";
import { errorCopy } from "../copy/error-copy";
import { fixLabel } from "../copy/gate-copy";
import { preflightCopy } from "../copy/preflight-copy";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../primitives/area-context";
import { Banner } from "../primitives/banner";
import {
	type CheckSource,
	type CheckState,
	Checklist,
	type ChecklistItem,
} from "../primitives/checklist";
import { CommandBlock } from "../primitives/command-block";
import { DvButton } from "../primitives/dv-button";
import { DvSheet } from "../primitives/dv-sheet";
import { CheckField, Field, SecretInput } from "../primitives/form-fields";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import { useCopy } from "../primitives/use-copy";
import { useRunAttentionTarget } from "../shell/attention-popover";
import { useDeviceWorkspace } from "../workspace/device-workspace-provider";
import type { UnlockRequest } from "../workspace/overlay-store";
import { useAttention, useAttentionState } from "../workspace/use-attention";
import { useKeyChip, useKeySession, usePreflight } from "../workspace/use-keys";
import { useLiveSession } from "../workspace/use-live";
import type { OverlaySheetProps } from "./area-overlays";
import {
	ConnectionProgress,
	connectionSteps,
	progressReport,
} from "./connection-progress";
import { authRejectionText, liveFailure } from "./diagnose-sheet";
import {
	type Attempts,
	DeviceLine,
	FOCUS_RING,
	LineStatus,
	type LockedDevice,
	type Unticked,
	lockedDevices,
	submitOnEnter,
	tryPassword,
	useRunGuard,
	withTick,
} from "./unlock-several-sheet";

export interface UnlockSheetProps extends OverlaySheetProps, UnlockRequest {
	deviceId: string;
	/** The unlock is for calling the device's models: offers keeping it unlocked (plan §9 Q2). */
	forModels?: boolean;
}

/** D1–D7 decide whether the password may be typed (IA §6.4.3); D8 and D9 only warn. */
const KEY_CHECKS = new Set<PreflightId>([
	"D1",
	"D2",
	"D3",
	"D4",
	"D5",
	"D6",
	"D7",
]);
/** Shown before the first read; the account check appears only when it fails. */
const FIRST_CHECKS: readonly PreflightId[] = [
	"D1",
	"D3",
	"D4",
	"D5",
	"D6",
	"D7",
	"D8",
	"D9",
];
const BOTH_PLANES = new Set<PreflightId>(["D7", "D9"]);
const LOCAL_CHECKS = new Set<PreflightId>(["D4", "D5", "D6"]);
const ROW_STATE: Record<PreflightRow["status"], CheckState> = {
	pass: "pass",
	warn: "warn",
	fail: "fail",
	block: "fail",
	checking: "active",
};
/** Fixes the sheet shows elsewhere (the identity banner) or that have no place to open. */
const NO_BUTTON = new Set<FixAction["kind"]>([
	"review_identity",
	"forget_identity",
	"use_full_token",
]);
/** An opt-in backup with the typed password is offered while one of these is open for the device. */
const BACKUP_ITEMS = new Set<AttentionKey>([
	"keys_not_backed_up_to_account",
	"account_backup_upload_pending",
	"account_backup_old_password",
]);
const DEFAULT_IDLE_MIN = 30;
const MINUTE_MS = 60_000;
const SECTION_TITLE = "text-ui font-semibold tracking-normal";

const Mono = ({ children }: Readonly<{ children: ReactNode }>) => (
	<span className="font-mono whitespace-nowrap">{children}</span>
);

/* Checks before the password. */

function checkName(t: DevicesT, id: PreflightId): string {
	const names: Partial<Record<PreflightId, string>> = {
		D1: t("devices:overlay.unlock.check.hub", "Hub supports devices"),
		D2: t("devices:overlay.unlock.check.account", "Your account"),
		D3: t("devices:overlay.unlock.check.access", "Your access"),
		D4: t("devices:overlay.unlock.check.keys", "Keys on this computer"),
		D5: t("devices:overlay.unlock.check.browser", "Browser can protect keys"),
		D6: t("devices:overlay.unlock.check.lock", "Other windows"),
		D7: t("devices:overlay.unlock.check.identity", "Device identity"),
		D8: t("devices:overlay.unlock.check.checkIn", "Check-in"),
		D9: t("devices:overlay.unlock.check.clocks", "Clocks"),
	};
	return names[id] ?? "";
}

/** What a finding means, in one sentence, for the codes that need more than their wording. */
const CODE_NOTES: Partial<Record<PreflightCode, (t: DevicesT) => string>> = {
	lock_held_elsewhere: (t) =>
		t(
			"devices:overlay.unlock.note.heldElsewhere",
			"Only one window can hold these keys at a time. The other window locks itself when you take them here.",
		),
	browser_may_delete_keys: (t) =>
		t(
			"devices:overlay.unlock.note.mayDeleteKeys",
			"Storage isn't persistent here. Back up the keys, or keep them safely.",
		),
	identity_mismatch: (t) =>
		t(
			"devices:overlay.unlock.note.identity",
			"Management is blocked until you compare the fingerprint on the device.",
		),
	clock_computer_off: (t) =>
		t(
			"devices:overlay.unlock.note.computerClock",
			"Connections and signed check-ins can fail. Turn on automatic time sync in this computer's system settings.",
		),
	clock_device_off: (t) =>
		t(
			"devices:overlay.unlock.note.deviceClockEstimated",
			"Estimated from the device's last encrypted status. Turn on automatic time sync on the device.",
		),
};

/** A clock-skew refusal the hub recorded (BG6), when this is the clock check. */
function clockRefusal(row: PreflightRow, device: DeviceRow | undefined) {
	const rejection = device?.auth_rejection;
	return row.id === "D9" && rejection?.code === "clock_skew"
		? rejection
		: undefined;
}

interface NoteContext {
	t: DevicesT;
	time: AreaTime;
	name: string;
	device: DeviceRow | undefined;
}

/** The second line of a check. On hubs that record no refusal the clock check keeps its estimate. */
function rowNote(row: PreflightRow, ctx: NoteContext): string | undefined {
	const refusal = clockRefusal(row, ctx.device);
	if (refusal) return authRejectionText(ctx.t, refusal, ctx.name, ctx.time);
	return CODE_NOTES[row.copy.code]?.(ctx.t);
}

/**
 * The identity pin is stored in milliseconds and reaches the pre-flight row
 * unconverted, while the copy formats unix seconds. No real pin is this late
 * in seconds, so a larger value is milliseconds.
 */
const MS_FLOOR = 100_000_000_000;
export const pinSeconds = (at: number) =>
	at >= MS_FLOOR ? Math.floor(at / 1000) : at;

/** The identity row as the copy takes it: the pin time in seconds, the fingerprint in four groups of four as the device prints it. */
function identityParams(row: PreflightRow): PreflightRow {
	const { fingerprint, since } = row.copy.params ?? {};
	if (fingerprint === undefined && since === undefined) return row;
	return {
		...row,
		copy: {
			...row.copy,
			params: {
				...row.copy.params,
				...(typeof fingerprint === "string"
					? { fingerprint: groupFingerprint(fingerprint) }
					: {}),
				...(typeof since === "number" ? { since: pinSeconds(since) } : {}),
			},
		},
	};
}

function fixesOf(row: PreflightRow): FixAction[] {
	return [row.fix, ...(row.otherFixes ?? [])].filter(
		(fix): fix is FixAction =>
			fix !== undefined &&
			!NO_BUTTON.has(fix.kind) &&
			!(fix.kind === "fix_clock" && !fix.deviceId),
	);
}

/** Fixes that happen in the sheet and change a check: the checks are read again afterwards. */
const LOCAL_FIXES: Partial<
	Record<
		FixAction["kind"],
		(workspace: DeviceWorkspace, deviceId: string) => Promise<unknown>
	>
> = {
	take_over: (workspace, deviceId) => workspace.keys.takeOver(deviceId),
	keep_keys_safely: (workspace) => workspace.local.requestPersistence(),
};

interface RowFixesProps {
	fixes: readonly FixAction[];
	busy: FixAction["kind"] | null;
	onFix(fix: FixAction): void;
}

function RowFixes({ fixes, busy, onFix }: Readonly<RowFixesProps>) {
	const { t } = useTranslation("devices");
	return (
		<>
			{fixes.map((fix, index) => (
				<DvButton
					key={fix.kind}
					size="xs"
					variant={index < 2 ? "default" : "ghost"}
					busy={busy === fix.kind}
					onClick={() => onFix(fix)}
				>
					{fixLabel(t, fix)}
				</DvButton>
			))}
		</>
	);
}

interface PreflightChecksProps {
	deviceId: string;
	name: string;
	device: DeviceRow | undefined;
	rows: readonly PreflightRow[] | undefined;
	/** Fix buttons show only while the form can be edited. */
	editable: boolean;
	refresh(): Promise<void>;
	onLeave(route: DevicesRoute): void;
}

/** The pre-flight rows by plane, each with its source on the right and its fix below (never a code, R3). */
function PreflightChecks({
	deviceId,
	name,
	device,
	rows,
	editable,
	refresh,
	onLeave,
}: Readonly<PreflightChecksProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const workspace = useDeviceWorkspace();
	const runTarget = useRunAttentionTarget({ onNavigate: onLeave });
	const [busy, setBusy] = useState<FixAction["kind"] | null>(null);

	const onFix = async (fix: FixAction) => {
		const local = LOCAL_FIXES[fix.kind];
		if (!local) {
			runTarget(fix);
			return;
		}
		setBusy(fix.kind);
		await local(workspace, deviceId).catch(() => undefined);
		await refresh();
		setBusy(null);
	};

	const both: CheckSource = {
		icon: Link2,
		label: t("overlay.unlock.source.both", "Hub + this computer"),
	};
	const sourceOf = (id: PreflightId): ChecklistItem["source"] => {
		if (BOTH_PLANES.has(id)) return both;
		return LOCAL_CHECKS.has(id) ? "local" : "hub";
	};
	const note: NoteContext = { t, time, name, device };

	const found = (row: PreflightRow): ChecklistItem => {
		const state = ROW_STATE[row.status];
		const fixes = editable ? fixesOf(row) : [];
		return {
			id: row.id,
			state: clockRefusal(row, device) && state === "pass" ? "warn" : state,
			label: (
				<>
					<span className="sr-only">{checkName(t, row.id)}: </span>
					{preflightCopy(t, identityParams(row), time).text}
				</>
			),
			source: sourceOf(row.id),
			note: rowNote(row, note),
			fix: fixes.length ? (
				<RowFixes fixes={fixes} busy={busy} onFix={onFix} />
			) : undefined,
		};
	};
	const waiting = (id: PreflightId, index: number): ChecklistItem => ({
		id,
		state: index === 0 ? "active" : "pending",
		label: checkName(t, id),
		source: sourceOf(id),
	});

	const items = rows
		? rows.filter((row) => row.id !== "D2" || row.status !== "pass").map(found)
		: FIRST_CHECKS.map(waiting);
	return (
		<Checklist
			items={items}
			label={t("overlay.unlock.checksLabel", "Checks before unlocking")}
		/>
	);
}

type Waiting = "checking" | "blocked" | "busy" | null;

/** Why the password can't be typed yet; `null` when it can. */
function waitingFor(
	preflight: Preflight | undefined,
	editable: boolean,
): Waiting {
	if (!editable) return "busy";
	if (!preflight) return "checking";
	const checking = preflight.rows.some(
		(row) => KEY_CHECKS.has(row.id) && row.status === "checking",
	);
	if (checking) return "checking";
	return preflight.passwordEnabled ? null : "blocked";
}

/* D7: the identity changed. */

interface IdentityFacts {
	pinnedAt?: number;
	trusted?: string;
	reported?: string;
}

function reportedFingerprint(device: DeviceRow | undefined) {
	if (!device) return undefined;
	try {
		return identityFingerprint(device.identity);
	} catch {
		return undefined;
	}
}

/** What this computer trusted and what the hub reports now, from the blocked key session or the hub row. */
function identityFacts(
	session: KeySessionSnapshot,
	row: PreflightRow,
	device: DeviceRow | undefined,
): IdentityFacts {
	const error =
		session.lastError?.code === "identity_mismatch"
			? session.lastError
			: undefined;
	const { since, fingerprint } = row.copy.params ?? {};
	const pinnedAt = typeof since === "number" ? since : error?.pinnedAt;
	const trusted =
		typeof fingerprint === "string" ? fingerprint : error?.fingerprint;
	return {
		pinnedAt: pinnedAt ? pinSeconds(pinnedAt) : undefined,
		trusted: trusted || undefined,
		reported: error?.reported || reportedFingerprint(device),
	};
}

interface IdentityTitleProps {
	name: string;
	pinnedAt: number | undefined;
}

function IdentityTitle({ name, pinnedAt }: Readonly<IdentityTitleProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const components = { 1: <Mono>{name}</Mono> };
	if (pinnedAt === undefined)
		return (
			<Trans
				t={t}
				i18nKey="overlay.unlock.identity.title"
				defaults="The hub reports different keys for <1/> than the ones you trusted."
				components={components}
			/>
		);
	return (
		<Trans
			t={t}
			i18nKey="overlay.unlock.identity.titleSince"
			defaults="The hub reports different keys for <1/> than the ones you trusted on {{since}}."
			values={{ since: time.at(pinnedAt) }}
			components={components}
		/>
	);
}

interface ComparePanelProps {
	id: string;
	facts: IdentityFacts;
}

/** BG21: the device prints its fingerprint with `status`; an older agent prints none. */
function ComparePanel({ id, facts }: Readonly<ComparePanelProps>) {
	const { t } = useTranslation("devices");
	const print = (fingerprint: string | undefined) =>
		fingerprint
			? groupFingerprint(fingerprint)
			: t("overlay.unlock.identity.unknown", "Not known");
	return (
		<section
			id={id}
			aria-label={t("overlay.unlock.identity.compare", "Compare on the device")}
			className="flex flex-col gap-2.5 rounded-lg border border-hairline bg-surface-sunken px-3.5 py-3 text-ui"
		>
			<p className="text-ui">
				{t(
					"overlay.unlock.identity.howTo",
					"Run this on the device and compare the fingerprint it prints with the two below.",
				)}
			</p>
			<CommandBlock
				command="flow-like-standalone status"
				note={t(
					"overlay.unlock.identity.commandNote",
					"Prints the device's identity fingerprint next to its connection state.",
				)}
			/>
			<KeyValueList>
				<KvRow
					label={t(
						"overlay.unlock.identity.trusted",
						"Trusted on this computer",
					)}
					className="font-mono"
				>
					{print(facts.trusted)}
				</KvRow>
				<KvRow
					label={t(
						"overlay.unlock.identity.reported",
						"Reported by the hub now",
					)}
					className="font-mono"
				>
					{print(facts.reported)}
				</KvRow>
			</KeyValueList>
			<p className="text-ui">
				{t(
					"overlay.unlock.identity.verdict",
					"If the device prints the fingerprint the hub reports, it was set up again: review the identity to trust its new keys. If it prints anything else, don't connect and tell the hub's operator.",
				)}
			</p>
			<p className="text-xs text-muted-foreground">
				{t(
					"overlay.unlock.identity.olderAgent",
					"An agent that prints no fingerprint is too old for this check. Update it on the device, then run the command again.",
				)}
			</p>
		</section>
	);
}

interface IdentityBlockProps {
	name: string;
	facts: IdentityFacts;
	onReview(): void;
}

/** D7 hard block: the keys the hub reports differ from the ones trusted here. */
function IdentityBlock({
	name,
	facts,
	onReview,
}: Readonly<IdentityBlockProps>) {
	const { t } = useTranslation("devices");
	const [comparing, setComparing] = useState(false);
	const panelId = useId();
	return (
		<>
			<Banner
				tone="critical"
				title={<IdentityTitle name={name} pinnedAt={facts.pinnedAt} />}
				actions={
					<>
						<DvButton
							size="sm"
							icon={Fingerprint}
							aria-expanded={comparing}
							aria-controls={panelId}
							onClick={() => setComparing((open) => !open)}
						>
							{t("overlay.unlock.identity.compare", "Compare on the device")}
						</DvButton>
						<DvButton size="sm" onClick={onReview}>
							{t("overlay.unlock.identity.review", "Review identity")}
						</DvButton>
					</>
				}
			>
				{t(
					"overlay.unlock.identity.text",
					"The device may have been set up again, or someone may be impersonating it. Management is blocked.",
				)}
			</Banner>
			{comparing ? <ComparePanel id={panelId} facts={facts} /> : null}
		</>
	);
}

/* The run: one password, this device first, then the others. */

type Phase = "form" | "running" | "done";

interface UnlockRunRequest {
	password: string;
	connectLive: boolean;
	backupToAccount: boolean;
	/** `undefined` keeps the session's own choice. */
	keepUnlocked: boolean | undefined;
	others: readonly LockedDevice[];
}

interface UnlockRunOptions {
	deviceId: string;
	alreadyOpen: boolean;
	connectLive: boolean | undefined;
	/** The keys are open: the opener's target can be revealed. */
	onUnlocked(): void;
	/** The attempt failed: a check may have changed (another window took the lock, the identity changed). */
	onFailed(): void;
}

function useUnlockRun(options: UnlockRunOptions) {
	const { t } = useTranslation("devices");
	const { keys } = useDeviceWorkspace();
	const begin = useRunGuard();
	const { deviceId } = options;
	const [phase, setPhase] = useState<Phase>(
		options.alreadyOpen ? "done" : "form",
	);
	const [failure, setFailure] = useState<string | null>(null);
	const [ranLive, setRanLive] = useState(options.connectLive === true);
	const [keySteps, setKeySteps] = useState<UnlockStep[]>([]);
	const [tried, setTried] = useState<readonly LockedDevice[]>([]);
	const [attempts, setAttempts] = useState<Attempts>({});

	const failureText = () => {
		const keyError = keys.snapshot(deviceId).lastError;
		if (keyError) return errorCopy(t, keyErrorCode(keyError));
		return t(
			"overlay.unlock.failed",
			"The keys couldn't be unlocked. Try again; if it repeats, copy the diagnostics for support.",
		);
	};

	const start = async (request: UnlockRunRequest) => {
		const run = begin();
		setFailure(null);
		setTried(request.others);
		setAttempts({});
		setRanLive(request.connectLive);
		setKeySteps([{ id: "unlocking_keys", state: "active" }]);
		setPhase("running");
		const onProgress = (step: UnlockStep) => {
			if (!run.current()) return;
			setKeySteps((previous) => [
				...previous.filter((row) => row.id !== step.id),
				step,
			]);
		};
		const opened = await keys
			.unlock(deviceId, request.password, {
				connectLive: request.connectLive,
				backupToAccount: request.backupToAccount,
				...(request.keepUnlocked === undefined
					? {}
					: { keepUnlocked: request.keepUnlocked }),
				signal: run.signal,
				onProgress,
			})
			.then(
				() => true,
				() => false,
			);
		if (!run.current()) return;
		if (!opened) {
			run.end();
			setFailure(failureText());
			setPhase("form");
			options.onFailed();
			return;
		}
		options.onUnlocked();
		const ids = request.others.map((row) => row.deviceId);
		await tryPassword(keys, ids, request.password, run.signal, (update) => {
			if (run.current())
				setAttempts((previous) => ({ ...previous, ...update }));
		});
		if (!run.current()) return;
		run.end();
		setPhase("done");
	};

	return { phase, failure, ranLive, keySteps, tried, attempts, start };
}

/* The form. */

interface PasswordFieldProps {
	id: string;
	name: string;
	value: string;
	waiting: Waiting;
	failure: string | null;
	onValueChange(value: string): void;
	onSubmit(): void;
	onForgot(): void;
}

function passwordHint(t: DevicesT, waiting: Waiting): string {
	if (waiting === "checking")
		return t(
			"devices:overlay.unlock.hint.checking",
			"Enabled once the checks above pass.",
		);
	if (waiting === "blocked")
		return t(
			"devices:overlay.unlock.hint.blocked",
			"Fix the failed check above first.",
		);
	return t(
		"devices:overlay.unlock.hint.ready",
		"The password never leaves this computer and the hub never sees it.",
	);
}

function PasswordField(props: Readonly<PasswordFieldProps>) {
	const { t } = useTranslation("devices");
	return (
		<Field
			id={props.id}
			label={
				<Trans
					t={t}
					i18nKey="overlay.unlock.passwordLabel"
					defaults="Device password for <1/>"
					components={{ 1: <Mono>{props.name}</Mono> }}
				/>
			}
			error={
				props.failure ? <span role="alert">{props.failure}</span> : undefined
			}
			hint={
				<>
					{passwordHint(t, props.waiting)}{" "}
					<DvButton
						variant="link"
						size="xs"
						className="text-xs"
						onClick={props.onForgot}
					>
						{t("overlay.unlock.forgot", "Forgot it?")}
					</DvButton>
				</>
			}
		>
			<SecretInput
				value={props.value}
				onValueChange={props.onValueChange}
				disabled={props.waiting !== null}
				className={FOCUS_RING}
				onKeyDown={submitOnEnter(props.onSubmit)}
			/>
		</Field>
	);
}

interface Option {
	checked: boolean;
	onChange(checked: boolean): void;
}

export interface KeepForModelsProps {
	id: string;
	option: Option;
	disabled: boolean;
}

/** Plan §9 Q2: a device unlocked to call its models may stay unlocked until the window closes. */
export function KeepForModels({
	id,
	option,
	disabled,
}: Readonly<KeepForModelsProps>) {
	const { t } = useTranslation("devices");
	const { deps } = useDeviceWorkspace();
	return (
		<CheckField
			id={id}
			checked={option.checked}
			disabled={disabled}
			onCheckedChange={option.onChange}
		>
			{t("devices:models.use.unlock.keep", "Keep unlocked for model access")}
			<span className="block text-xs text-muted-foreground">
				{deps.platform === "desktop"
					? t(
							"devices:models.use.unlock.keepHintDesktop",
							"Your flows on this computer can call its models until you lock it or quit Flow-Like. It won't lock after 30 min unused.",
						)
					: t(
							"devices:models.use.unlock.keepHintWeb",
							"Its models stay reachable from this window until you lock it or close the window. It won't lock after 30 min unused.",
						)}
			</span>
		</CheckField>
	);
}

interface UnlockOptionsProps {
	formId: string;
	name: string;
	editable: boolean;
	/** False when the device is offline: only encrypted snapshots can be read. */
	online: boolean;
	live: Option;
	/** Offered when the unlock is for calling the device's models. */
	keep: Option | null;
	/** Offered while the device's account backup is missing, pending or sealed with an older password. */
	backup: Option | null;
	/** Offered while other devices with keys here are locked. */
	others: Option | null;
}

function UnlockOptions(props: Readonly<UnlockOptionsProps>) {
	const { t } = useTranslation("devices");
	const { formId, editable, online, live, keep, backup, others } = props;
	return (
		<>
			<CheckField
				id={`${formId}-live`}
				checked={live.checked}
				disabled={!editable || !online}
				onCheckedChange={live.onChange}
			>
				{t("overlay.unlock.connectLive", "Connect live now")}
				{online ? null : (
					<span className="text-muted-foreground">
						{" "}
						<Trans
							t={t}
							i18nKey="overlay.unlock.connectLiveOffline"
							defaults="(<1/> is offline, so only encrypted snapshots can be read)"
							components={{ 1: <Mono>{props.name}</Mono> }}
						/>
					</span>
				)}
			</CheckField>
			{keep ? (
				<KeepForModels
					id={`${formId}-keep`}
					option={keep}
					disabled={!editable}
				/>
			) : null}
			{backup ? (
				<CheckField
					id={`${formId}-backup`}
					checked={backup.checked}
					disabled={!editable}
					onCheckedChange={backup.onChange}
				>
					{t("overlay.unlock.saveBackup", "Also save to your account backup")}
					<span className="block text-xs text-muted-foreground">
						{t(
							"overlay.unlock.saveBackupHint",
							"The keys are sealed with this password and uploaded, so another computer can restore them.",
						)}
					</span>
				</CheckField>
			) : null}
			{others ? (
				<CheckField
					id={`${formId}-more`}
					checked={others.checked}
					disabled={!editable}
					onCheckedChange={others.onChange}
				>
					{t(
						"overlay.unlock.unlockOthers",
						"Unlock other devices with this password",
					)}
				</CheckField>
			) : null}
		</>
	);
}

interface OtherDevicesProps {
	formId: string;
	rows: readonly LockedDevice[];
	attempts: Attempts;
	unticked: Unticked;
	editable: boolean;
	showHint: boolean;
	onTick(deviceId: string, checked: boolean): void;
}

function OtherDevices(props: Readonly<OtherDevicesProps>) {
	const { t } = useTranslation("devices");
	return (
		<div className="flex flex-col gap-1.5 pl-5.5">
			<ul
				aria-label={t("overlay.unlock.othersLabel", "Other devices")}
				className="flex flex-col"
			>
				{props.rows.map((row) => (
					<DeviceLine
						key={row.deviceId}
						fieldId={`${props.formId}-other-${row.deviceId}`}
						deviceId={row.deviceId}
						name={row.name}
						checked={!props.unticked[row.deviceId]}
						disabled={!props.editable}
						status={
							<LineStatus attempt={props.attempts[row.deviceId]}>
								{enumLabel(t, "vaultKind", row.role)}
							</LineStatus>
						}
						onCheckedChange={(checked) => props.onTick(row.deviceId, checked)}
					/>
				))}
			</ul>
			{props.showHint ? (
				<p className="text-xs text-muted-foreground">
					{t(
						"overlay.unlock.othersHint",
						"Each device takes about a second. Devices where the password fails stay locked.",
					)}
				</p>
			) : null}
		</div>
	);
}

/* The result. */

interface LiveLine {
	tone: "good" | "info" | "warning";
	text: string;
	retry: boolean;
}

function liveOpenText(t: DevicesT, live: LiveState): string | undefined {
	if (live.kind !== "live" && live.kind !== "renewing") return undefined;
	return live.transport === "webrtc"
		? t(
				"devices:overlay.unlock.result.liveDirect",
				"Live connection open (direct).",
			)
		: t(
				"devices:overlay.unlock.result.liveRelayed",
				"Live connection open (relayed through the hub).",
			);
}

/** The keys are open either way; this says what became of the live connection. */
function liveLine(t: DevicesT, live: LiveState, wanted: boolean): LiveLine {
	if (!wanted)
		return {
			tone: "good",
			retry: false,
			text: t(
				"devices:overlay.unlock.result.snapshots",
				"Reading encrypted snapshots; no live connection.",
			),
		};
	const reason = liveFailure(t, live);
	if (reason)
		return {
			tone: "warning",
			retry: true,
			text: t(
				"devices:overlay.unlock.result.liveFailed",
				"{{reason}} The keys are unlocked; encrypted snapshots still work.",
				{ reason },
			),
		};
	const open = liveOpenText(t, live);
	if (open) return { tone: "good", retry: false, text: open };
	return {
		tone: "info",
		retry: false,
		text: t("devices:overlay.unlock.result.connecting", "Connecting live…"),
	};
}

/** Minutes until the idle lock, from the session's own clock; 30 by default. */
function idleMinutes(session: KeySessionSnapshot): number {
	const { idleLocksAt, lastUsedAt } = session;
	if (idleLocksAt === undefined || lastUsedAt === undefined)
		return DEFAULT_IDLE_MIN;
	return Math.max(1, Math.round((idleLocksAt - lastUsedAt) / MINUTE_MS));
}

interface ResultTitleProps {
	name: string;
	session: KeySessionSnapshot;
	alreadyOpen: boolean;
}

function ResultTitle({
	name,
	session,
	alreadyOpen,
}: Readonly<ResultTitleProps>) {
	const { t } = useTranslation("devices");
	const components = { 1: <Mono>{name}</Mono> };
	const values = { minutes: idleMinutes(session) };
	if (session.keepUnlocked)
		return (
			<Trans
				t={t}
				i18nKey="overlay.unlock.result.titleKept"
				defaults="Unlocked. <1/> stays unlocked while this window is open."
				components={components}
			/>
		);
	if (alreadyOpen)
		return (
			<Trans
				t={t}
				i18nKey="overlay.unlock.result.titleAlready"
				defaults="<1/> is already unlocked. It locks after {{minutes, number}} min unused."
				values={values}
				components={components}
			/>
		);
	return (
		<Trans
			t={t}
			i18nKey="overlay.unlock.result.title"
			defaults="Unlocked. <1/> locks after {{minutes, number}} min unused."
			values={values}
			components={components}
		/>
	);
}

interface UnlockResultProps extends ResultTitleProps {
	live: LiveState;
	wantedLive: boolean;
	onRetry(): void;
}

function UnlockResult({
	live,
	wantedLive,
	onRetry,
	...title
}: Readonly<UnlockResultProps>) {
	const { t } = useTranslation("devices");
	const line = liveLine(t, live, wantedLive);
	return (
		<Banner
			tone={line.tone}
			title={<ResultTitle {...title} />}
			actions={
				line.retry ? (
					<DvButton size="sm" icon={RefreshCw} onClick={onRetry}>
						{t("overlay.unlock.result.retry", "Try connecting again")}
					</DvButton>
				) : undefined
			}
		>
			{line.text}
		</Banner>
	);
}

interface ReportFacts {
	preflight: Preflight | undefined;
	session: KeySessionSnapshot;
	live: LiveState;
	steps: readonly UnlockStep[];
}

/** Copy diagnostics: English and machine-oriented on purpose, the only place the check codes appear (R3). */
function unlockReport({
	preflight,
	session,
	live,
	steps,
}: ReportFacts): string {
	const keyError = session.lastError ? ` (${session.lastError.code})` : "";
	return [
		preflight?.diagnostics() ?? "Flow-Like device pre-flight · not finished",
		`key session: ${session.state}${keyError}`,
		`live: ${live.kind}`,
		...progressReport(steps),
	].join("\n");
}

/**
 * SPEC §3.10 / IA §6.4: pre-flight checks by plane, then the device password,
 * then the connection progress. One password opens this device's key session
 * in this window; the typed value is handed to the key session and dropped.
 */
export function UnlockSheet({
	deviceId,
	connectLive,
	returnTo,
	forModels = false,
	onNavigate,
	onClose,
}: Readonly<UnlockSheetProps>) {
	const { t } = useTranslation("devices");
	const { input } = useAttentionState();
	const session = useKeySession(deviceId);
	const { sessions, setKeepUnlocked } = useKeyChip();
	const { preflight, refresh } = usePreflight(deviceId);
	const live = useLiveSession(deviceId);
	const attention = useAttention({ deviceId });
	const { copied, copy } = useCopy();
	const formId = useId();
	const passwordId = `${formId}-password`;
	const [alreadyOpen] = useState(() => session.state === "unlocked");
	const [password, setPassword] = useState("");
	const [liveChoice, setLiveChoice] = useState<boolean | null>(null);
	const [keep, setKeep] = useState(true);
	const [saveBackup, setSaveBackup] = useState(false);
	const [more, setMore] = useState(false);
	const [unticked, setUnticked] = useState<Unticked>({});
	const focused = useRef(false);
	const result = useRef<HTMLDivElement | null>(null);
	const doneButton = useRef<HTMLButtonElement | null>(null);

	const run = useUnlockRun({
		deviceId,
		alreadyOpen,
		connectLive,
		onUnlocked: () => {
			if (returnTo) onNavigate(returnTo);
		},
		onFailed: () => {
			focused.current = false;
			void refresh();
		},
	});
	const { phase } = run;
	const editable = phase === "form";

	const facts = fleetFacts(input).byId.get(deviceId);
	const device = facts?.row;
	const name = facts?.name ?? deviceLabel(input, deviceId);
	const identityRow = preflight?.rows.find(
		(row) => row.copy.code === "identity_mismatch",
	);
	const waiting = waitingFor(preflight, editable);
	const online = preflight?.suggestConnectLive !== false;
	const wantLive = (liveChoice ?? connectLive ?? true) && online;
	const backupOffered =
		session.role === "owner" &&
		attention.some((item) => BACKUP_ITEMS.has(item.key));
	const lockable = useMemo(
		() =>
			lockedDevices(input, sessions).filter((row) => row.deviceId !== deviceId),
		[input, sessions, deviceId],
	);
	const chosen = more ? lockable.filter((row) => !unticked[row.deviceId]) : [];
	const others = editable ? lockable : run.tried;
	const ready = waiting === null && password.length > 0;
	const steps = alreadyOpen
		? live.steps
		: connectionSteps(run.keySteps, live.steps, run.ranLive);
	const showProgress = !editable && (!alreadyOpen || run.ranLive);

	useEffect(() => {
		if (waiting !== null || focused.current) return;
		focused.current = true;
		globalThis.document?.getElementById(passwordId)?.focus();
	}, [waiting, passwordId]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: runs once for a device that was open when the sheet appeared
	useEffect(() => {
		if (alreadyOpen && connectLive) void live.connect().catch(() => undefined);
	}, []);

	useEffect(() => {
		if (phase !== "done") return;
		result.current?.scrollIntoView?.({ block: "nearest" });
		doneButton.current?.focus({ preventScroll: true });
	}, [phase]);

	const leaveTo = (route: DevicesRoute) => {
		onClose();
		onNavigate(route);
	};

	const submit = () => {
		if (!ready) return;
		const secret = password;
		setPassword("");
		void run.start({
			password: secret,
			connectLive: wantLive,
			backupToAccount: backupOffered && saveBackup,
			keepUnlocked: forModels ? keep : undefined,
			others: chosen,
		});
	};

	const copyDiagnostics = () =>
		void copy(
			unlockReport({
				preflight,
				session,
				live: live.state,
				steps: editable ? [] : steps,
			}),
		);

	return (
		<DvSheet
			open
			onOpenChange={(open) => {
				if (!open) onClose();
			}}
			icon={LockOpen}
			title={
				<Trans
					t={t}
					i18nKey="overlay.unlock.title"
					defaults="Unlock <1/>"
					components={{ 1: <Mono>{name}</Mono> }}
				/>
			}
			sub={t(
				"overlay.unlock.subtitle",
				"One password opens this device's keys on this computer for this window. It never leaves this computer and the hub never sees it.",
			)}
			footNote={
				<DvButton variant="link" size="xs" onClick={copyDiagnostics}>
					{copied
						? t("overlay.unlock.diagnosticsCopied", "Diagnostics copied")
						: t("overlay.unlock.copyDiagnostics", "Copy diagnostics")}
				</DvButton>
			}
			foot={
				<UnlockFoot
					phase={phase}
					ready={ready}
					count={(editable ? chosen : run.tried).length + 1}
					doneButton={doneButton}
					onClose={onClose}
					onSubmit={submit}
				/>
			}
		>
			{identityRow && phase !== "done" ? (
				<IdentityBlock
					name={name}
					facts={identityFacts(session, identityRow, device)}
					onReview={() => leaveTo({ screen: "device", deviceId, tab: "keys" })}
				/>
			) : null}
			<h3 className={SECTION_TITLE}>
				{t("overlay.unlock.checksTitle", "Checks before you type the password")}
			</h3>
			<PreflightChecks
				deviceId={deviceId}
				name={name}
				device={device}
				rows={preflight?.rows}
				editable={editable}
				refresh={refresh}
				onLeave={leaveTo}
			/>
			{phase === "done" ? null : (
				<>
					<PasswordField
						id={passwordId}
						name={name}
						value={password}
						waiting={waiting}
						failure={run.failure}
						onValueChange={setPassword}
						onSubmit={submit}
						onForgot={() =>
							leaveTo({ screen: "keys", guide: "forgot-password" })
						}
					/>
					<UnlockOptions
						formId={formId}
						name={name}
						editable={editable}
						online={online}
						live={{ checked: wantLive, onChange: setLiveChoice }}
						keep={forModels ? { checked: keep, onChange: setKeep } : null}
						backup={
							backupOffered
								? { checked: saveBackup, onChange: setSaveBackup }
								: null
						}
						others={
							more || lockable.length > 0
								? { checked: more, onChange: setMore }
								: null
						}
					/>
				</>
			)}
			{more && others.length > 0 ? (
				<OtherDevices
					formId={formId}
					rows={others}
					attempts={run.attempts}
					unticked={unticked}
					editable={editable}
					showHint={phase !== "done"}
					onTick={(otherId, checked) =>
						setUnticked((previous) => withTick(previous, otherId, checked))
					}
				/>
			) : null}
			{showProgress ? (
				<>
					<h3 className={SECTION_TITLE}>
						{t("overlay.unlock.progressTitle", "Unlocking")}
					</h3>
					<ConnectionProgress steps={steps} />
				</>
			) : null}
			{phase === "done" ? (
				<div ref={result}>
					<UnlockResult
						name={name}
						session={session}
						alreadyOpen={alreadyOpen}
						live={live.state}
						wantedLive={run.ranLive}
						onRetry={() => void live.retry()}
					/>
					{forModels ? (
						<KeepForModels
							id={`${formId}-keep`}
							option={{
								checked: session.keepUnlocked,
								onChange: (checked) => setKeepUnlocked(deviceId, checked),
							}}
							disabled={session.state !== "unlocked"}
						/>
					) : null}
				</div>
			) : null}
		</DvSheet>
	);
}

/* The foot. Declared last: Codacy's Lizard pass misreads JSX closing tags and counts whatever follows a component like this one as part of it. */

interface UnlockFootProps {
	phase: Phase;
	ready: boolean;
	/** Devices the password will be tried on, this one included. */
	count: number;
	doneButton: Ref<HTMLButtonElement>;
	onClose(): void;
	onSubmit(): void;
}

function UnlockFoot(props: Readonly<UnlockFootProps>) {
	const { t } = useTranslation("devices");
	if (props.phase === "done")
		return (
			<DvButton
				ref={props.doneButton}
				variant="primary"
				onClick={props.onClose}
			>
				{t("overlay.unlock.done", "Done")}
			</DvButton>
		);
	const several = t(
		"overlay.unlock.submitSeveral",
		"Unlock {{count, number}} devices",
		{ count: props.count },
	);
	return (
		<>
			<DvButton onClick={props.onClose}>
				{t("overlay.unlock.cancel", "Cancel")}
			</DvButton>
			<DvButton
				variant="primary"
				icon={LockOpen}
				busy={props.phase === "running"}
				aria-disabled={props.phase === "form" && !props.ready}
				onClick={props.onSubmit}
			>
				{props.count > 1 ? several : t("overlay.unlock.submit", "Unlock")}
			</DvButton>
		</>
	);
}
