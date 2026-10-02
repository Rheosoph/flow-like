"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Check,
	Download,
	FileText,
	KeyRound,
	Pencil,
	Upload,
} from "lucide-react";
import { type ReactNode, useId, useState } from "react";
import {
	type CertificateAuthorityEnvelope,
	certificateAuthorityBackup,
	renewLocalCertificateAuthority,
	restoreCertificateAuthority,
} from "../../../../lib/device-management/certificate-authority";
import { humanFileSize } from "../../../../lib/utils";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Banner } from "../primitives/banner";
import { Checklist } from "../primitives/checklist";
import {
	ConsequencePreview,
	type ConsequenceRows,
} from "../primitives/consequence-preview";
import { DvButton } from "../primitives/dv-button";
import { DvSheet } from "../primitives/dv-sheet";
import {
	CheckField,
	DropZone,
	Field,
	SecretInput,
} from "../primitives/form-fields";
import { GateInline } from "../primitives/gate-notice";
import { InlineResult, type ResultTone } from "../primitives/inline-result";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import { StatusChip } from "../primitives/status-chip";
import { type AuthorityView, DAY_S } from "./certificates-model";
import { dayText, shortId } from "./parts";
import {
	type AuthorityStore,
	MAX_BACKUP_BYTES,
	localAuthority,
	readBackupFile,
	rewrapAuthority,
	saveFile,
	testAuthorityPassword,
	useStillHere,
} from "./use-certificates";

export const MIN_PASSWORD = 12;
export const MAX_PASSWORD_BYTES = 4096;

export interface AuthorityNote {
	tone: ResultTone;
	text: string;
}

export const backupFileName = (authorityId: string) =>
	`${shortId(authorityId)}-authority.json`;

/** "Felix Schultz · api.flow-like.com · profile default": what a backup is bound to. */
export interface AuthorityHome {
	name?: string;
	host: string;
	profile: string;
}

export function homeText(t: DevicesT, home: AuthorityHome): string {
	const { name, host, profile } = home;
	return name
		? t(
				"devices:certificates.authority.home",
				"{{name}} · {{host}} · profile {{profile}}",
				{ name, host, profile },
			)
		: t(
				"devices:certificates.authority.homeAnonymous",
				"your account · {{host}} · profile {{profile}}",
				{ host, profile },
			);
}

interface ChosenFile {
	file: File;
	tooLarge: boolean;
}

function BackupFileField({
	chosen,
	onChosen,
	hint,
	error,
}: Readonly<{
	chosen: ChosenFile | null;
	onChosen(file: ChosenFile): void;
	hint: ReactNode;
	error?: ReactNode;
}>) {
	const { t } = useTranslation("devices");
	const id = useId();
	return (
		<div
			className="flex flex-col gap-1.5"
			data-invalid={error ? "true" : undefined}
		>
			<span className="text-[13px]/[18px] font-medium">
				{t("certificates.backup.fileLabel", "Backup file")}
			</span>
			<DropZone
				id={id}
				accept=".json,application/json"
				title={t(
					"certificates.backup.drop",
					"Drop the authority backup file or choose it",
				)}
				hint={hint}
				files={
					chosen ? (
						<StatusChip tone="outline" icon={FileText}>
							{t("certificates.backup.chosen", "{{file}} · {{size}}", {
								file: chosen.file.name,
								size: humanFileSize(chosen.file.size),
							})}
						</StatusChip>
					) : undefined
				}
				onFiles={([file]) => {
					if (file) onChosen({ file, tooLarge: file.size > MAX_BACKUP_BYTES });
				}}
			/>
			{error ? (
				<p role="alert" className="text-xs text-critical">
					{error}
				</p>
			) : null}
		</div>
	);
}

function tooLargeText(t: DevicesT, chosen: ChosenFile): string {
	return t(
		"devices:certificates.backup.tooLarge",
		"That file is {{size}}. Authority backups are at most 1 MiB.",
		{ size: humanFileSize(chosen.file.size) },
	);
}

interface BackupInputProblems {
	file?: string;
	password?: string;
}

/** What is missing before a backup file and its password can be tried. */
function backupInputProblems(
	t: DevicesT,
	chosen: ChosenFile | null,
	password: string,
	noFile: string,
): BackupInputProblems {
	const problems: BackupInputProblems = {};
	if (!chosen) problems.file = noFile;
	else if (chosen.tooLarge) problems.file = tooLargeText(t, chosen);
	if (!password)
		problems.password = t(
			"devices:certificates.backup.noPassword",
			"Enter the authority password.",
		);
	return problems;
}

/** Download the new backup, tick that it is saved, then use it. Nothing is stored before the tick. */
export function BackupSaveStep({
	envelope,
	downloaded,
	onDownloaded,
	saved,
	onSaved,
	downloadLabel,
	savedLabel,
	downloadFirst,
}: Readonly<{
	envelope: CertificateAuthorityEnvelope;
	downloaded: boolean;
	onDownloaded(): void;
	saved: boolean;
	onSaved(saved: boolean): void;
	downloadLabel: string;
	savedLabel: string;
	downloadFirst: string;
}>) {
	const { t } = useTranslation("devices");
	const id = useId();
	const name = backupFileName(envelope.public_bundle.authority_id);
	const [size, setSize] = useState<number | null>(null);
	const download = () => {
		const blob = certificateAuthorityBackup(envelope);
		saveFile(name, blob);
		setSize(blob.size);
		onDownloaded();
	};
	return (
		<>
			<div>
				<DvButton icon={Download} onClick={download}>
					{downloadLabel}
				</DvButton>
			</div>
			{downloaded ? (
				<InlineResult tone="good">
					{t(
						"certificates.backup.downloaded",
						"{{file}} was handed to your browser ({{size}}). Nothing is uploaded.",
						{ file: name, size: humanFileSize(size ?? 0) },
					)}
				</InlineResult>
			) : null}
			<span className="flex flex-col items-start gap-1">
				<CheckField
					id={id}
					checked={saved}
					disabled={!downloaded}
					onCheckedChange={onSaved}
				>
					{savedLabel}
				</CheckField>
				{downloaded ? null : (
					<GateInline kind="busy">{downloadFirst}</GateInline>
				)}
			</span>
		</>
	);
}

interface SheetProps {
	view: AuthorityView;
	store: AuthorityStore;
	open: boolean;
	onClose(): void;
	/** The result line shown on the authority's card. */
	onDone(note: AuthorityNote): void;
}

type Phase = "input" | "running" | "backup" | "saving";

/** Renew the signing key from the backup's root key: needs the file and the password, and yields a new backup. */
export function RenewKeySheet({
	view,
	store,
	open,
	onClose,
	onDone,
}: Readonly<SheetProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const stillHere = useStillHere();
	const passwordId = useId();
	const [phase, setPhase] = useState<Phase>("input");
	const [chosen, setChosen] = useState<ChosenFile | null>(null);
	const [password, setPassword] = useState("");
	const [error, setError] = useState<{ file?: string; password?: string }>({});
	const [envelope, setEnvelope] = useState<CertificateAuthorityEnvelope | null>(
		null,
	);
	const [downloaded, setDownloaded] = useState(false);
	const [saved, setSaved] = useState(false);
	const newExpiry = Math.min(view.rootExpiresAt, time.nowS + 365 * DAY_S);

	const rows: ConsequenceRows = {
		what: t(
			"certificates.renewKey.what",
			"A new signing key replaces the current one on this computer, valid until {{date}} (1 year at most, never past the root).",
			{ date: dayText(time, newExpiry) },
		),
		who: t(
			"certificates.renewKey.who",
			"Nobody notices. Certificates and renewal authorities it already signed keep working.",
		),
		stays: t(
			"certificates.renewKey.stays",
			"The root, its fingerprint and the trust installed on clients stay the same.",
		),
		when: t(
			"certificates.renewKey.when",
			"Now. Opening the backup takes a few seconds.",
		),
		undo: {
			reversible: false,
			text: t(
				"certificates.renewKey.undo",
				"The old signing key is replaced, and your current backup file stops matching. You download a new backup in the next step.",
			),
		},
		first: t(
			"certificates.renewKey.first",
			"Have the backup file and the authority password ready.",
		),
	};

	const run = async () => {
		const secret = password;
		const failures = backupInputProblems(
			t,
			chosen,
			secret,
			t(
				"certificates.renewKey.noFile",
				"Choose the backup file first. It holds the root key that signs the new signing key.",
			),
		);
		setError(failures);
		if (!chosen || failures.file || failures.password) return;
		const here = stillHere();
		setPassword("");
		setPhase("running");
		try {
			const text = (await readBackupFile(chosen.file)) ?? "";
			const crypto = await store.crypto();
			const renewed = await renewLocalCertificateAuthority(
				store.scope,
				view.authority,
				text,
				secret,
				crypto,
			);
			if (!here()) return;
			setEnvelope(renewed);
			setPhase("backup");
		} catch {
			if (!here()) return;
			setPhase("input");
			setError({
				password: t(
					"certificates.renewKey.failed",
					"That file and password don't open the backup of {{label}}. Nothing changed.",
					{ label: view.label },
				),
			});
		}
	};

	const use = async () => {
		if (!envelope || !saved) return;
		const here = stillHere();
		setPhase("saving");
		const stored = await store.replace(
			view.authority,
			localAuthority(envelope),
			t("certificates.renewKey.label", "Renew the signing key of {{label}}", {
				label: view.label,
			}),
		);
		if (!here()) return;
		if (!stored) {
			setPhase("backup");
			setError({
				file: t(
					"certificates.renewKey.storeFailed",
					"This computer couldn't save the renewed signing key. Nothing changed; try again.",
				),
			});
			return;
		}
		onDone({
			tone: "good",
			text: t(
				"certificates.renewKey.done",
				"Signing key renewed at {{time}}. It now expires {{date}}. Keep the new backup; older backup files no longer match.",
				{
					time: time.clock(time.nowS),
					date: dayText(time, envelope.public_bundle.issuer_not_after),
				},
			),
		});
		onClose();
	};

	const inBackup = phase === "backup" || phase === "saving";
	return (
		<DvSheet
			open={open}
			onOpenChange={(next) => {
				if (!next && phase !== "running" && phase !== "saving") onClose();
			}}
			icon={KeyRound}
			eyebrow={t("certificates.sheet.before", "Before this runs")}
			title={t(
				"certificates.renewKey.title",
				"Renew the signing key of {{label}}?",
				{ label: view.label },
			)}
			sub={t(
				"certificates.renewKey.subtitle",
				"Needs your backup file and the authority password",
			)}
			wide
			closeOnOutside={false}
			foot={
				inBackup ? (
					<>
						<DvButton onClick={onClose} disabled={phase === "saving"}>
							{t("certificates.backup.closeUnused", "Close without using it")}
						</DvButton>
						<DvButton
							variant="primary"
							icon={Check}
							disabled={!saved}
							busy={phase === "saving"}
							onClick={() => void use()}
						>
							{t("certificates.renewKey.use", "Use renewed signing key")}
						</DvButton>
					</>
				) : (
					<>
						<DvButton onClick={onClose} disabled={phase === "running"}>
							{t("certificates.sheet.cancel", "Cancel")}
						</DvButton>
						<DvButton
							variant="primary"
							icon={KeyRound}
							busy={phase === "running"}
							onClick={() => void run()}
						>
							{t("certificates.renewKey.submit", "Renew signing key")}
						</DvButton>
					</>
				)
			}
		>
			{phase === "running" ? (
				<Checklist
					items={[
						{
							id: "open",
							state: "active",
							label: t(
								"certificates.renewKey.stepOpen",
								"Opening the backup with your password",
							),
							source: "local",
						},
						{
							id: "match",
							state: "pending",
							label: t(
								"certificates.renewKey.stepMatch",
								"Checking it belongs to {{label}}",
								{ label: view.label },
							),
							source: "local",
						},
						{
							id: "create",
							state: "pending",
							label: t(
								"certificates.renewKey.stepCreate",
								"Creating a new signing key, valid until {{date}}",
								{ date: dayText(time, newExpiry) },
							),
							source: "local",
						},
					]}
				/>
			) : inBackup && envelope ? (
				<>
					<Banner
						tone="warning"
						title={t(
							"certificates.renewKey.backupTitle",
							"Your old backup file no longer matches {{label}}.",
							{ label: view.label },
						)}
					>
						{t(
							"certificates.renewKey.backupText",
							"Download the new backup and store it offline with the password. If you close now, nothing changes: the current signing key and your old backup stay valid.",
						)}
					</Banner>
					<KeyValueList>
						<KvRow label={t("certificates.renewKey.newKey", "New signing key")}>
							{t("certificates.renewKey.validUntil", "valid until {{date}}", {
								date: dayText(time, envelope.public_bundle.issuer_not_after),
							})}
						</KvRow>
						<KvRow label={t("certificates.backup.file", "File")}>
							<span className="font-mono text-xs">
								{backupFileName(view.id)}
							</span>
						</KvRow>
						<KvRow label={t("certificates.backup.root", "Root")}>
							{t(
								"certificates.renewKey.rootSame",
								"unchanged · same fingerprint, clients need nothing new",
							)}
						</KvRow>
					</KeyValueList>
					<BackupSaveStep
						envelope={envelope}
						downloaded={downloaded}
						onDownloaded={() => setDownloaded(true)}
						saved={saved}
						onSaved={setSaved}
						downloadLabel={t(
							"certificates.backup.downloadNew",
							"Download new authority backup",
						)}
						savedLabel={t(
							"certificates.backup.savedNew",
							"I saved the new backup and its password",
						)}
						downloadFirst={t(
							"certificates.backup.downloadNewFirst",
							"Download the new backup first.",
						)}
					/>
					{error.file ? (
						<InlineResult tone="critical">{error.file}</InlineResult>
					) : null}
				</>
			) : (
				<>
					<ConsequencePreview rows={rows} />
					<BackupFileField
						chosen={chosen}
						onChosen={(next) => {
							setChosen(next);
							setError({});
						}}
						hint={t("certificates.backup.hintNamed", "{{file}} · up to 1 MiB", {
							file: backupFileName(view.id),
						})}
						error={error.file}
					/>
					<Field
						id={passwordId}
						label={t(
							"certificates.backup.passwordFor",
							"Authority password for {{label}}",
							{ label: view.label },
						)}
						error={error.password}
					>
						<SecretInput value={password} onValueChange={setPassword} />
					</Field>
				</>
			)}
		</DvSheet>
	);
}

interface PasswordDraft {
	current: string;
	next: string;
	again: string;
}

const NO_PASSWORDS: PasswordDraft = { current: "", next: "", again: "" };

/**
 * Change the authority password. The signing key here and the root key in the
 * backup are sealed together, so the backup file is needed and a new backup
 * (opening with the new password) is the result.
 */
export function ChangePasswordSheet({
	view,
	store,
	open,
	onClose,
	onDone,
}: Readonly<SheetProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const stillHere = useStillHere();
	const ids = { current: useId(), next: useId(), again: useId() };
	const [phase, setPhase] = useState<Phase>("input");
	const [chosen, setChosen] = useState<ChosenFile | null>(null);
	const [draft, setDraft] = useState<PasswordDraft>(NO_PASSWORDS);
	const [error, setError] = useState<
		Partial<Record<keyof PasswordDraft | "file", string>>
	>({});
	const [envelope, setEnvelope] = useState<CertificateAuthorityEnvelope | null>(
		null,
	);
	const [downloaded, setDownloaded] = useState(false);
	const [saved, setSaved] = useState(false);

	const rows: ConsequenceRows = {
		what: t(
			"certificates.changePassword.what",
			"The signing key on this computer and both keys in a new backup file are encrypted with the new password.",
		),
		who: t(
			"certificates.changePassword.who",
			"Only this computer. Other computers and profiles keep their own copy and password.",
		),
		stays: t(
			"certificates.changePassword.stays",
			"The keys themselves and everything they signed. Backup files you saved earlier still open with the old password.",
		),
		when: t(
			"certificates.changePassword.when",
			"Now. Opening the backup takes a few seconds.",
		),
		undo: {
			reversible: true,
			text: t(
				"certificates.changePassword.undo",
				"Change it back the same way.",
			),
		},
		first: t(
			"certificates.changePassword.first",
			"Have the backup file and the current password ready: the root key in the backup is sealed again too.",
		),
	};

	const check = (): typeof error => {
		const failures: typeof error = {};
		if (!chosen)
			failures.file = t(
				"certificates.changePassword.noFile",
				"Choose the backup file first. Its root key gets the new password too.",
			);
		else if (chosen.tooLarge) failures.file = tooLargeText(t, chosen);
		if (!draft.current)
			failures.current = t(
				"certificates.changePassword.noCurrent",
				"Enter the current authority password.",
			);
		if (draft.next.length < MIN_PASSWORD)
			failures.next = t(
				"certificates.password.tooShort",
				"The password has {{count, number}} characters. Use at least {{min, number}}.",
				{ count: draft.next.length, min: MIN_PASSWORD },
			);
		else if (draft.next === draft.current)
			failures.next = t(
				"certificates.changePassword.same",
				"The new password is the same as the current one.",
			);
		else if (draft.again !== draft.next)
			failures.again = t(
				"certificates.password.mismatch",
				"The two passwords don't match. Type the password again in both fields.",
			);
		return failures;
	};

	const run = async () => {
		const failures = check();
		setError(failures);
		if (Object.keys(failures).length || !chosen) return;
		const secrets = { current: draft.current, next: draft.next };
		const here = stillHere();
		setDraft(NO_PASSWORDS);
		setPhase("running");
		const text = (await readBackupFile(chosen.file).catch(() => null)) ?? "";
		const result = await store
			.crypto()
			.then((crypto) =>
				rewrapAuthority(store.scope, view.authority, text, secrets, crypto),
			)
			.catch(() => ({ ok: false as const, reason: "password" as const }));
		if (!here()) return;
		if (result.ok) {
			setEnvelope(result.envelope);
			setPhase("backup");
			return;
		}
		setPhase("input");
		setError(
			result.reason === "backup"
				? {
						file: t(
							"certificates.changePassword.wrongFile",
							"That file isn't the backup of {{label}}. Nothing changed.",
							{ label: view.label },
						),
					}
				: {
						current: t(
							"certificates.changePassword.wrongPassword",
							"That isn't the current password: it doesn't open the signing key and the backup. Nothing changed.",
						),
					},
		);
	};

	const use = async () => {
		if (!envelope || !saved) return;
		const here = stillHere();
		setPhase("saving");
		const stored = await store.replace(
			view.authority,
			localAuthority(envelope),
			t(
				"certificates.changePassword.label",
				"Change the password of {{label}}",
				{ label: view.label },
			),
		);
		if (!here()) return;
		if (!stored) {
			setPhase("backup");
			setError({
				file: t(
					"certificates.changePassword.storeFailed",
					"This computer couldn't save the signing key under the new password. Nothing changed; try again.",
				),
			});
			return;
		}
		onDone({
			tone: "good",
			text: t(
				"certificates.changePassword.done",
				"Password of {{label}} changed at {{time}}. Backup files you saved earlier still open with the old password.",
				{ label: view.label, time: time.clock(time.nowS) },
			),
		});
		onClose();
	};

	const inBackup = phase === "backup" || phase === "saving";
	const set = (key: keyof PasswordDraft) => (value: string) =>
		setDraft((current) => ({ ...current, [key]: value }));
	return (
		<DvSheet
			open={open}
			onOpenChange={(next) => {
				if (!next && phase !== "running" && phase !== "saving") onClose();
			}}
			icon={Pencil}
			eyebrow={t("certificates.sheet.before", "Before this runs")}
			title={t(
				"certificates.changePassword.title",
				"Change the password of {{label}}?",
				{ label: view.label },
			)}
			sub={t(
				"certificates.changePassword.subtitle",
				"A different secret from device passwords",
			)}
			wide
			closeOnOutside={false}
			foot={
				inBackup ? (
					<>
						<DvButton onClick={onClose} disabled={phase === "saving"}>
							{t("certificates.backup.closeUnused", "Close without using it")}
						</DvButton>
						<DvButton
							variant="primary"
							icon={Check}
							disabled={!saved}
							busy={phase === "saving"}
							onClick={() => void use()}
						>
							{t("certificates.changePassword.use", "Use the new password")}
						</DvButton>
					</>
				) : (
					<>
						<DvButton onClick={onClose} disabled={phase === "running"}>
							{t("certificates.sheet.cancel", "Cancel")}
						</DvButton>
						<DvButton
							variant="primary"
							busy={phase === "running"}
							onClick={() => void run()}
						>
							{t("certificates.changePassword.submit", "Change password")}
						</DvButton>
					</>
				)
			}
		>
			{inBackup && envelope ? (
				<>
					<Banner
						tone="warning"
						title={t(
							"certificates.changePassword.backupTitle",
							"Only the new backup opens with the new password.",
						)}
					>
						{t(
							"certificates.changePassword.backupText",
							"Download it and store it offline with the new password. If you close now, nothing changes: the signing key keeps its current password.",
						)}
					</Banner>
					<BackupSaveStep
						envelope={envelope}
						downloaded={downloaded}
						onDownloaded={() => setDownloaded(true)}
						saved={saved}
						onSaved={setSaved}
						downloadLabel={t(
							"certificates.backup.downloadNew",
							"Download new authority backup",
						)}
						savedLabel={t(
							"certificates.backup.savedNew",
							"I saved the new backup and its password",
						)}
						downloadFirst={t(
							"certificates.backup.downloadNewFirst",
							"Download the new backup first.",
						)}
					/>
					{error.file ? (
						<InlineResult tone="critical">{error.file}</InlineResult>
					) : null}
				</>
			) : (
				<>
					<ConsequencePreview rows={rows} />
					<BackupFileField
						chosen={chosen}
						onChosen={(next) => {
							setChosen(next);
							setError({});
						}}
						hint={t("certificates.backup.hintNamed", "{{file}} · up to 1 MiB", {
							file: backupFileName(view.id),
						})}
						error={error.file}
					/>
					<Field
						id={ids.current}
						label={t(
							"certificates.changePassword.current",
							"Current authority password",
						)}
						error={error.current}
					>
						<SecretInput value={draft.current} onValueChange={set("current")} />
					</Field>
					<Field
						id={ids.next}
						label={t(
							"certificates.changePassword.new",
							"New authority password",
						)}
						error={error.next}
					>
						<SecretInput
							value={draft.next}
							onValueChange={set("next")}
							autoComplete="new-password"
							minBytes={MIN_PASSWORD}
							maxBytes={MAX_PASSWORD_BYTES}
						/>
					</Field>
					<Field
						id={ids.again}
						label={t(
							"certificates.changePassword.again",
							"Type the new password again",
						)}
						error={error.again}
					>
						<SecretInput
							value={draft.again}
							onValueChange={set("again")}
							autoComplete="new-password"
						/>
					</Field>
				</>
			)}
		</DvSheet>
	);
}

/** Checks that a password opens the signing key on this computer. Nothing changes. */
export function TestPasswordSheet({
	view,
	store,
	open,
	onClose,
}: Readonly<Omit<SheetProps, "onDone">>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const stillHere = useStillHere();
	const id = useId();
	const [password, setPassword] = useState("");
	const [busy, setBusy] = useState(false);
	const [result, setResult] = useState<AuthorityNote | null>(null);

	const test = async () => {
		const secret = password;
		if (!secret) {
			setResult({
				tone: "warning",
				text: t(
					"certificates.testPassword.empty",
					"Type the password to test it.",
				),
			});
			return;
		}
		const here = stillHere();
		setPassword("");
		setBusy(true);
		const opens = await store
			.crypto()
			.then((crypto) =>
				testAuthorityPassword(store.scope, view.authority, secret, crypto),
			)
			.catch(() => false);
		if (!here()) return;
		setBusy(false);
		const at = time.clock(time.nowS);
		setResult(
			opens
				? {
						tone: "good",
						text: t(
							"certificates.testPassword.opens",
							"That password opens the signing key of {{label}}. Checked at {{time}}; nothing changed.",
							{ label: view.label, time: at },
						),
					}
				: {
						tone: "critical",
						text: t(
							"certificates.testPassword.fails",
							"That password doesn't open the signing key of {{label}}. Checked at {{time}}; nothing changed.",
							{ label: view.label, time: at },
						),
					},
		);
	};

	return (
		<DvSheet
			open={open}
			onOpenChange={(next) => {
				if (!next) onClose();
			}}
			icon={Check}
			title={t(
				"certificates.testPassword.title",
				"Test the password of {{label}}",
				{ label: view.label },
			)}
			sub={t(
				"certificates.testPassword.subtitle",
				"Checks that the password opens the signing key on this computer. Nothing changes.",
			)}
			foot={
				<>
					<DvButton onClick={onClose}>
						{t("certificates.sheet.close", "Close")}
					</DvButton>
					<DvButton variant="primary" busy={busy} onClick={() => void test()}>
						{t("certificates.testPassword.submit", "Test password")}
					</DvButton>
				</>
			}
		>
			<Field
				id={id}
				label={t("certificates.testPassword.label", "Authority password")}
			>
				<SecretInput value={password} onValueChange={setPassword} />
			</Field>
			{result ? (
				<InlineResult tone={result.tone}>{result.text}</InlineResult>
			) : null}
			<p className="text-xs text-muted-foreground">
				{t(
					"certificates.testPassword.hint",
					"Use this now and then so you notice a forgotten password while the backup can still be opened with it.",
				)}
			</p>
		</DvSheet>
	);
}

/** Restore an authority from its backup: only the signing key is saved here; the root key stays in the file. */
export function RestoreSheet({
	store,
	home,
	open,
	onClose,
	onDone,
}: Readonly<{
	store: AuthorityStore;
	home: AuthorityHome;
	open: boolean;
	onClose(): void;
	/** `restored` is false when the authority was already on this computer. */
	onDone(result: {
		authorityId: string;
		restored: boolean;
		note: AuthorityNote;
	}): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const stillHere = useStillHere();
	const passwordId = useId();
	const [chosen, setChosen] = useState<ChosenFile | null>(null);
	const [password, setPassword] = useState("");
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<{ file?: string; password?: string }>({});

	/** Opens the backup with the password; throws the library's fixed sentence when it doesn't open. */
	const openBackup = async (file: File, secret: string) => {
		const text = (await readBackupFile(file)) ?? "";
		const crypto = await store.crypto();
		return restoreCertificateAuthority(store.scope, text, secret, crypto);
	};

	const restore = async () => {
		const secret = password;
		const failures = backupInputProblems(
			t,
			chosen,
			secret,
			t("certificates.restore.noFile", "Choose the backup file first."),
		);
		setError(failures);
		if (!chosen || failures.file || failures.password) return;
		const here = stillHere();
		setPassword("");
		setBusy(true);
		try {
			const authority = await openBackup(chosen.file, secret);
			const { authority_id: authorityId, label } = authority.public_bundle;
			const present = store.authorities.some(
				(entry) => entry.public_bundle.authority_id === authorityId,
			);
			const restored =
				!present &&
				(await store.add(
					authority,
					t("certificates.restore.label", "Restore {{label}}", { label }),
				));
			if (!here()) return;
			if (!present && !restored) {
				setError({
					file: t(
						"certificates.restore.storeFailed",
						"This computer couldn't save the signing key. Nothing was saved; try again.",
					),
				});
				return;
			}
			onDone({
				authorityId,
				restored,
				note: restored
					? {
							tone: "good",
							text: t(
								"certificates.restore.done",
								"Restored {{label}} at {{time}}. Its signing key is saved on this computer, encrypted with the authority password. Keep the backup file offline: it's the only copy of the root key.",
								{ label, time: time.clock(time.nowS) },
							),
						}
					: {
							tone: "unknown",
							text: t(
								"certificates.restore.present",
								"This backup is for {{label}}, which is already on this computer. Nothing changed.",
								{ label },
							),
						},
			});
			onClose();
		} catch {
			if (!here()) return;
			setError({
				password: t(
					"certificates.restore.failed",
					"The backup couldn't be opened for this account and hub. Check the file and its password.",
				),
			});
		} finally {
			if (here()) setBusy(false);
		}
	};

	return (
		<DvSheet
			open={open}
			onOpenChange={(next) => {
				if (!next && !busy) onClose();
			}}
			icon={Upload}
			title={t(
				"certificates.restore.title",
				"Restore an authority from its backup",
			)}
			sub={t(
				"certificates.restore.subtitle",
				"Saves its signing key on this computer, encrypted with the authority password",
			)}
			closeOnOutside={false}
			foot={
				<>
					<DvButton onClick={onClose} disabled={busy}>
						{t("certificates.sheet.cancel", "Cancel")}
					</DvButton>
					<DvButton
						variant="primary"
						icon={Upload}
						busy={busy}
						onClick={() => void restore()}
					>
						{t("certificates.restore.submit", "Restore authority")}
					</DvButton>
				</>
			}
		>
			<BackupFileField
				chosen={chosen}
				onChosen={(next) => {
					setChosen(next);
					setError({});
				}}
				hint={t("certificates.backup.hint", ".json · up to 1 MiB")}
				error={error.file}
			/>
			<Field
				id={passwordId}
				label={t("certificates.testPassword.label", "Authority password")}
				error={error.password}
			>
				<SecretInput value={password} onValueChange={setPassword} />
			</Field>
			<KeyValueList>
				<KvRow label={t("certificates.restore.into", "Restores into")}>
					{homeText(t, home)}
					<span className="mt-0.5 block text-xs text-muted-foreground">
						{t(
							"certificates.restore.intoHint",
							"A backup only opens for the account, hub and profile it was made in.",
						)}
					</span>
				</KvRow>
				<KvRow label={t("certificates.restore.rootKey", "Root key")}>
					{t(
						"certificates.restore.rootStays",
						"stays in the file; only the signing key is saved here",
					)}
				</KvRow>
			</KeyValueList>
		</DvSheet>
	);
}
