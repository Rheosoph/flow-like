"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { CloudUpload, Download, KeyRound, Lock } from "lucide-react";
import {
	type FormEvent,
	type RefObject,
	useEffect,
	useRef,
	useState,
} from "react";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../ui/select";
import type { DevicesT } from "../primitives/area-context";
import {
	ConsequencePreview,
	type ConsequenceRows,
} from "../primitives/consequence-preview";
import { DvButton } from "../primitives/dv-button";
import { DvSheet } from "../primitives/dv-sheet";
import { Field, SecretInput, utf8Bytes } from "../primitives/form-fields";
import { InlineResult } from "../primitives/inline-result";
import { PASSWORD_MAX_BYTES, PASSWORD_MIN_BYTES } from "./key-operations";
import { Mono, Note, keyKindLabel } from "./key-parts";
import { type KeyFileLog, rowScope, useKeyResults } from "./key-store";
import { type KeyRow, rowsWithKeys } from "./keys-model";
import { useFlowGuard, useKeyActions } from "./use-key-actions";
import type { KeysRead } from "./use-keys-model";

interface PasswordValues {
	current: string;
	next: string;
	repeat: string;
}

interface FieldErrors {
	current?: string;
	next?: string;
	repeat?: string;
	form?: string;
}

interface Changed {
	at: string;
	file?: Blob;
	/** The account copy was saved again with the new password in this sheet. */
	updated?: { version: number; at: string };
	downloaded?: string;
	followUpError?: string;
}

const EMPTY: PasswordValues = { current: "", next: "", repeat: "" };

function newPasswordError(
	t: DevicesT,
	values: PasswordValues,
): string | undefined {
	const bytes = utf8Bytes(values.next);
	if (bytes < PASSWORD_MIN_BYTES)
		return t(
			"devices:keys.password.tooShort",
			"The new password is {{bytes, number}} bytes. Use at least 12.",
			{ bytes },
		);
	if (bytes > PASSWORD_MAX_BYTES)
		return t(
			"devices:keys.password.tooLong",
			"The new password is {{bytes, number}} bytes. Use at most 4,096.",
			{ bytes },
		);
	return values.next === values.current
		? t(
				"devices:keys.error.samePassword",
				"Choose a different password from the current one.",
			)
		: undefined;
}

/** What is wrong with the three fields before anything is sent. */
function validatePasswords(t: DevicesT, values: PasswordValues): FieldErrors {
	const found: FieldErrors = {};
	if (!values.current)
		found.current = t(
			"devices:keys.password.currentMissing",
			"Enter the current device password first.",
		);
	const next = newPasswordError(t, values);
	if (next) found.next = next;
	else if (values.next !== values.repeat)
		found.repeat = t(
			"devices:keys.password.mismatch",
			"The two new passwords don't match. Type the new one again in both fields.",
		);
	return found;
}

function staysText(t: DevicesT, hub: number, fileSaved: boolean): string {
	if (hub)
		return t(
			"devices:keys.password.oldNote",
			"Your account backup (v{{version}}) and old backup files still open with the old password until you update them.",
			{ version: hub },
		);
	return fileSaved
		? t(
				"devices:keys.password.staysFile",
				"Backup files you saved still open with the old password. There's no account backup yet.",
			)
		: t(
				"devices:keys.password.staysNone",
				"There's no account backup or backup file yet. Back the keys up right after.",
			);
}

function consequenceRows(
	t: DevicesT,
	row: KeyRow,
	fileSaved: boolean,
): ConsequenceRows {
	return {
		what: (
			<Trans
				t={t}
				i18nKey="devices:keys.password.what"
				defaults="This computer's copy of the keys for <1/> is sealed with the new password."
				components={{ 1: <Mono>{row.name}</Mono> }}
			/>
		),
		who: t(
			"devices:keys.password.who",
			"Only this computer. Other computers and profiles keep their own passwords.",
		),
		stays: staysText(t, row.hubRevision ?? 0, fileSaved),
		when: t(
			"devices:keys.password.when",
			"Immediately. Nothing is sent to the device or the hub.",
		),
		undo: {
			reversible: true,
			text: t(
				"devices:keys.password.undo",
				"Change it back the same way, with the new password as the current one.",
			),
		},
	};
}

function DeviceSelect({
	rows,
	value,
	disabled,
	onChange,
}: Readonly<{
	rows: readonly KeyRow[];
	value: string;
	disabled: boolean;
	onChange(deviceId: string): void;
}>) {
	const { t } = useTranslation("devices");
	return (
		<Field
			id="keys-password-device"
			label={t("keys.password.device", "Device")}
		>
			<Select value={value} onValueChange={onChange} disabled={disabled}>
				<SelectTrigger
					id="keys-password-device"
					className="h-8.5 w-full rounded-lg border-input bg-card text-[13px]/[18px] shadow-none"
				>
					<SelectValue />
				</SelectTrigger>
				<SelectContent className="border-border-strong bg-popover shadow-none backdrop-blur-none">
					{rows.map((row) => (
						<SelectItem
							key={row.deviceId}
							value={row.deviceId}
							className="text-[13px]/[18px] focus:bg-row-hover focus:text-foreground"
						>
							<span className="font-mono">{row.name}</span>
							<span className="text-muted-foreground">
								{" · "}
								{keyKindLabel(t, row.vault?.role ?? "owner")}
							</span>
						</SelectItem>
					))}
				</SelectContent>
			</Select>
		</Field>
	);
}

function PasswordFields({
	values,
	errors,
	disabled,
	onChange,
}: Readonly<{
	values: PasswordValues;
	errors: FieldErrors;
	disabled: boolean;
	onChange(key: keyof PasswordValues, value: string): void;
}>) {
	const { t } = useTranslation("devices");
	return (
		<>
			<Field
				id="keys-password-current"
				label={t("keys.password.current", "Current device password")}
				error={errors.current}
			>
				<SecretInput
					value={values.current}
					onValueChange={(value) => onChange("current", value)}
					autoComplete="current-password"
					disabled={disabled}
				/>
			</Field>
			<div className="@container/pw">
				<div className="grid grid-cols-2 gap-3 @max-[560px]/pw:grid-cols-1">
					<Field
						id="keys-password-new"
						label={t("keys.password.new", "New device password")}
						error={errors.next}
						hint={t(
							"keys.password.newHint",
							"If you forget it, nobody can recover it.",
						)}
					>
						<SecretInput
							value={values.next}
							onValueChange={(value) => onChange("next", value)}
							autoComplete="new-password"
							minBytes={PASSWORD_MIN_BYTES}
							maxBytes={PASSWORD_MAX_BYTES}
							disabled={disabled}
						/>
					</Field>
					<Field
						id="keys-password-repeat"
						label={t("keys.password.repeatNew", "Repeat the new password")}
						error={errors.repeat}
						hint={t("keys.password.repeatHint", "Type it again to check it.")}
					>
						<SecretInput
							value={values.repeat}
							onValueChange={(value) => onChange("repeat", value)}
							autoComplete="new-password"
							disabled={disabled}
						/>
					</Field>
				</div>
			</div>
		</>
	);
}

interface FormProps {
	rows: readonly KeyRow[];
	row: KeyRow;
	fileSaved: boolean;
	onDevice(deviceId: string): void;
	/** The change is committed: the new password is handed over once, for the backup offer. */
	onChanged(next: string, change: Changed): void;
	onClose(): void;
}

/** Every submit clears the three fields, whether it was refused, failed or went through. */
function PasswordForm({
	rows,
	row,
	fileSaved,
	onDevice,
	onChanged,
	onClose,
}: Readonly<FormProps>) {
	const { t } = useTranslation("devices");
	const actions = useKeyActions();
	const results = useKeyResults();
	const guard = useFlowGuard();
	const working = useRef(false);
	const [values, setValues] = useState(EMPTY);
	const [errors, setErrors] = useState<FieldErrors>({});
	const [busy, setBusy] = useState(false);
	const { deviceId, name } = row;
	const hub = row.hubRevision ?? 0;

	const submit = async (event: FormEvent) => {
		event.preventDefault();
		if (working.current) return;
		const found = validatePasswords(t, values);
		const { current, next } = values;
		setValues(EMPTY);
		setErrors(found);
		if (Object.keys(found).length) return;
		const flow = guard();
		working.current = true;
		setBusy(true);
		const outcome = await actions.changePassword(
			{ deviceId, name },
			current,
			next,
			flow.signal,
		);
		if (!flow.alive()) return;
		working.current = false;
		setBusy(false);
		if (!outcome.ok) {
			const field = outcome.code === "wrong_password" ? "current" : "form";
			setErrors(outcome.text ? { [field]: outcome.text } : {});
			return;
		}
		const at = actions.timeNow();
		results.put(
			rowScope(deviceId),
			"good",
			hub
				? t(
						"keys.result.passwordChanged",
						"Password changed at {{at}}. Your account backup (v{{version}}) still opens with the old password until you update it.",
						{ at, version: hub },
					)
				: t(
						"keys.result.passwordChangedNoBackup",
						"Password changed at {{at}}. Back the keys up to your account with the new password.",
						{ at },
					),
		);
		onChanged(next, { at, ...outcome.value });
	};

	const formId = "keys-password-form";
	return (
		<DvSheet
			open
			onOpenChange={(next) => {
				if (!next && !busy) onClose();
			}}
			icon={KeyRound}
			title={t("keys.password.title", "Change device password")}
			sub={t(
				"keys.password.subtitle",
				"One password per device on this computer. It isn't your account password.",
			)}
			footNote={t(
				"keys.backupSheet.footNote",
				"The password never leaves this computer.",
			)}
			foot={
				<>
					<DvButton onClick={onClose} aria-disabled={busy || undefined}>
						{t("keys.sheet.cancel", "Cancel")}
					</DvButton>
					<DvButton variant="primary" type="submit" form={formId} busy={busy}>
						{t("keys.password.go", "Change password")}
					</DvButton>
				</>
			}
		>
			<form
				id={formId}
				className="flex flex-col gap-3.5"
				onSubmit={(event) => void submit(event)}
			>
				<DeviceSelect
					rows={rows}
					value={deviceId}
					disabled={busy || rows.length < 2}
					onChange={onDevice}
				/>
				{row.session?.state === "unlocked" ? (
					<Note icon={Lock}>
						<Trans
							t={t}
							i18nKey="keys.password.unlockedNote"
							defaults="<1/> is unlocked and stays unlocked. The new password applies the next time you unlock it."
							components={{ 1: <Mono>{name}</Mono> }}
						/>
					</Note>
				) : null}
				<PasswordFields
					values={values}
					errors={errors}
					disabled={busy}
					onChange={(key, value) =>
						setValues((current) => ({ ...current, [key]: value }))
					}
				/>
				{errors.form ? (
					<InlineResult tone="critical">{errors.form}</InlineResult>
				) : null}
				<ConsequencePreview compact rows={consequenceRows(t, row, fileSaved)} />
			</form>
		</DvSheet>
	);
}

interface DoneProps {
	row: KeyRow;
	changed: Changed;
	/** The new password, held for "Update account backup" until the sheet closes. */
	fresh: RefObject<string>;
	onChanged(patch: Partial<Changed>): void;
	onClose(): void;
}

function BackupNote({
	row,
	changed,
}: Readonly<Pick<DoneProps, "row" | "changed">>) {
	const { t } = useTranslation("devices");
	if (changed.updated)
		return (
			<Note tone="good">
				{t(
					"keys.password.updatedNote",
					"Your account backup (v{{version}}) opens with the new password. Backup files saved before {{at}} still open with the old one.",
					{ version: changed.updated.version, at: changed.at },
				)}
			</Note>
		);
	return (
		<Note tone="warning">
			{row.hubRevision
				? t(
						"keys.password.oldNote",
						"Your account backup (v{{version}}) and old backup files still open with the old password until you update them.",
						{ version: row.hubRevision },
					)
				: t(
						"keys.password.noBackupNote",
						"These keys still exist only on this computer. Back them up now, with the new password.",
					)}
		</Note>
	);
}

/** The result, with both backups offered at once: they still open with the old password. */
function PasswordChanged({
	row,
	changed,
	fresh,
	onChanged,
	onClose,
}: Readonly<DoneProps>) {
	const { t } = useTranslation("devices");
	const actions = useKeyActions();
	const results = useKeyResults();
	const guard = useFlowGuard();
	const working = useRef(false);
	const [busy, setBusy] = useState(false);
	const target = { deviceId: row.deviceId, name: row.name };

	const updateBackup = async () => {
		const secret = fresh.current;
		if (working.current || !secret) return;
		const flow = guard();
		working.current = true;
		setBusy(true);
		const outcome = await actions.saveBackup(target, secret, flow.signal);
		if (!flow.alive()) return;
		working.current = false;
		setBusy(false);
		if (!outcome.ok) {
			onChanged({ followUpError: outcome.text || undefined });
			return;
		}
		fresh.current = "";
		const updated = { version: outcome.value, at: actions.timeNow() };
		onChanged({ updated, followUpError: undefined });
		results.put(
			rowScope(row.deviceId),
			"good",
			t(
				"keys.result.updated",
				"Account backup updated as version {{version}} at {{at}}. It opens with the password you just typed.",
				updated,
			),
		);
	};

	const download = () => {
		if (changed.file)
			onChanged({ downloaded: actions.saveSealedFile(target, changed.file) });
	};

	return (
		<DvSheet
			open
			onOpenChange={(next) => {
				if (!next && !busy) onClose();
			}}
			icon={KeyRound}
			title={t("keys.password.title", "Change device password")}
			sub={t(
				"keys.password.subtitle",
				"One password per device on this computer. It isn't your account password.",
			)}
			footNote={
				changed.updated
					? undefined
					: t(
							"keys.password.later",
							"You can also do this later from the device's row.",
						)
			}
			foot={
				<DvButton
					variant={changed.updated ? "primary" : "default"}
					onClick={onClose}
					aria-disabled={busy || undefined}
				>
					{t("keys.sheet.done", "Done")}
				</DvButton>
			}
		>
			<InlineResult tone="good">
				<Trans
					t={t}
					i18nKey="keys.password.done"
					defaults="Password changed for <1/> at {{at}}. Use the new password to unlock it on this computer."
					values={{ at: changed.at }}
					components={{ 1: <Mono>{row.name}</Mono> }}
				/>
			</InlineResult>
			<BackupNote row={row} changed={changed} />
			<div className="flex flex-wrap items-center gap-2">
				{changed.updated ? null : (
					<DvButton
						variant="primary"
						icon={CloudUpload}
						busy={busy}
						onClick={() => void updateBackup()}
					>
						{row.hubRevision
							? t("keys.backup.update", "Update account backup")
							: t("keys.backup.backUp", "Back up to account")}
					</DvButton>
				)}
				{changed.file ? (
					<DvButton icon={Download} onClick={download}>
						{t("keys.password.download", "Download new backup file")}
					</DvButton>
				) : null}
			</div>
			<FollowUpResults changed={changed} />
		</DvSheet>
	);
}

/**
 * SPEC §5.9 "Change device password…": a real flow. It works while the
 * device is unlocked, and offers the two backups right after, because both
 * still open with the old password.
 */
export function ChangePasswordSheet({
	read,
	files,
	deviceId: initialDeviceId,
	onClose,
}: Readonly<{
	read: KeysRead;
	files: KeyFileLog;
	deviceId?: string;
	onClose(): void;
}>) {
	const candidates = rowsWithKeys(read.model);
	const [deviceId, setDeviceId] = useState(
		candidates.find((row) => row.deviceId === initialDeviceId)?.deviceId ??
			candidates[0]?.deviceId,
	);
	const [changed, setChanged] = useState<Changed>();
	/* Held only between the change and closing the sheet, for "Update account backup"; never rendered. */
	const fresh = useRef("");
	useEffect(
		() => () => {
			fresh.current = "";
		},
		[],
	);
	const row = candidates.find((entry) => entry.deviceId === deviceId);
	if (!row) return null;

	const close = () => {
		fresh.current = "";
		onClose();
	};
	if (changed)
		return (
			<PasswordChanged
				row={row}
				changed={changed}
				fresh={fresh}
				onChanged={(patch) =>
					setChanged((current) =>
						current ? { ...current, ...patch } : current,
					)
				}
				onClose={close}
			/>
		);
	return (
		<PasswordForm
			key={row.deviceId}
			rows={candidates}
			row={row}
			fileSaved={files[row.deviceId]?.savedAt !== undefined}
			onDevice={setDeviceId}
			onChanged={(next, change) => {
				fresh.current = next;
				setChanged(change);
			}}
			onClose={close}
		/>
	);
}

function FollowUpResults({ changed }: Readonly<{ changed: Changed }>) {
	const { t } = useTranslation("devices");
	return (
		<>
			{changed.file ? null : (
				<InlineResult tone="warning">
					{t(
						"keys.password.fileFailed",
						"The new backup file couldn't be prepared. Keep this computer's storage until you have saved a new backup.",
					)}
				</InlineResult>
			)}
			{changed.followUpError ? (
				<InlineResult tone="critical">{changed.followUpError}</InlineResult>
			) : null}
			{changed.updated ? (
				<InlineResult tone="good">
					{t(
						"keys.result.updated",
						"Account backup updated as version {{version}} at {{at}}. It opens with the password you just typed.",
						changed.updated,
					)}
				</InlineResult>
			) : null}
			{changed.downloaded ? (
				<InlineResult tone="good">
					<Trans
						t={t}
						i18nKey="keys.password.downloaded"
						defaults="Saved <1/>, sealed with the new password."
						components={{ 1: <Mono>{changed.downloaded}</Mono> }}
					/>
				</InlineResult>
			) : null}
		</>
	);
}
