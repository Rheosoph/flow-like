"use client";

import { useTranslation } from "@flow-like/locales";
import { Check, Copy, KeyRound, RefreshCw } from "lucide-react";
import { useState } from "react";
import {
	type DeploymentVariable,
	variableValue,
} from "../../../../lib/device-management/deployment";
import type { ManagementResponse } from "../../../../lib/device-management/types";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../ui/select";
import { errorCopy } from "../copy/error-copy";
import { gateCopy } from "../copy/gate-copy";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { ConsequencePreview } from "../primitives/consequence-preview";
import { DvButton } from "../primitives/dv-button";
import { DvSheet } from "../primitives/dv-sheet";
import { Field, SecretInput, utf8Bytes } from "../primitives/form-fields";
import { useCopy } from "../primitives/use-copy";
import type { DeviceActionOutcome } from "../workspace";
import { SELECT_CONTENT, SELECT_ITEM, SELECT_TRIGGER } from "./config-parts";
import type { SecretWrite } from "./use-service-config";

/* S26: secret values travel only inside the encrypted live session and are never shown again. */

export const SECRET_MAX_BYTES = 4096;
export const TOKEN_MIN_CHARS = 32;
const TOKEN_PATTERN = /^[\x21-\x7e]{32,4096}$/;
const TOKEN_RANDOM_BYTES = 24;

export interface SecretChoice {
	/** Variable id. */
	id: string;
	/** The stored secret's reference in the settings. */
	reference: string;
	label: string;
	definition?: DeploymentVariable;
}

const isPlainText = (definition: DeploymentVariable) =>
	definition.value_type === "Normal" &&
	["String", "PathBuf", "Date"].includes(definition.data_type);

type SecretShape = "list" | "map" | "bool" | "number";

function shapeOf(definition: DeploymentVariable): SecretShape {
	if (["Array", "HashSet"].includes(definition.value_type)) return "list";
	if (definition.value_type === "HashMap") return "map";
	return definition.data_type === "Boolean" ? "bool" : "number";
}

/** The value as the device stores it: JSON of the variable's type. `null` when the text doesn't fit the type. */
export function serializeSecret(
	definition: DeploymentVariable | undefined,
	text: string,
): string | null {
	if (!definition) return JSON.stringify(text);
	try {
		return JSON.stringify(variableValue(definition, text));
	} catch {
		return null;
	}
}

/** How to type a secret that isn't plain text; `null` for text and for a secret whose type isn't known. */
function typeHint(t: DevicesT, definition: DeploymentVariable | undefined) {
	if (!definition || isPlainText(definition)) return null;
	const hints: Record<SecretShape, string> = {
		list: t(
			"devices:serviceConfig.secret.hintList",
			'This secret is a list: enter it as JSON, for example ["a", "b"].',
		),
		map: t(
			"devices:serviceConfig.secret.hintMap",
			'This secret is a set of named values: enter it as JSON, for example {"key": "value"}.',
		),
		bool: t("devices:serviceConfig.secret.hintBool", "Enter true or false."),
		number: t("devices:serviceConfig.secret.hintNumber", "Enter a number."),
	};
	return hints[shapeOf(definition)];
}

/** Why a send didn't happen, in the sheet that started it; `null` when it was sent. */
function useSendError() {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	return (outcome: DeviceActionOutcome<ManagementResponse>): string | null => {
		switch (outcome.status) {
			case "done":
			case "unknown":
			case "busy":
			case "cancelled":
				return null;
			case "gated":
				return gateCopy(t, outcome.gate, time).inline;
			case "rejected":
				return outcome.rejection.error
					? t(
							"serviceConfig.secret.refused",
							"The device refused the new value: “{{reason}}”",
							{ reason: outcome.rejection.error },
						)
					: t(
							"serviceConfig.secret.refusedPlain",
							"The device refused the new value. Nothing changed.",
						);
			default:
				return t(
					"serviceConfig.secret.notSent",
					"The new value wasn't saved. {{why}}",
					{ why: errorCopy(t, outcome.failure.code) },
				);
		}
	};
}

function SheetFoot({
	busy,
	confirmLabel,
	onCancel,
	onConfirm,
}: Readonly<{
	busy: boolean;
	confirmLabel: string;
	onCancel(): void;
	onConfirm(): void;
}>) {
	const { t } = useTranslation("devices");
	return (
		<>
			<DvButton onClick={onCancel} disabled={busy}>
				{t("serviceConfig.sheet.cancel", "Cancel")}
			</DvButton>
			<DvButton
				variant="primary"
				busy={busy}
				data-act="secret-save"
				onClick={onConfirm}
			>
				{confirmLabel}
			</DvButton>
		</>
	);
}

function SecretPicker({
	secrets,
	value,
	onChange,
}: Readonly<{
	secrets: readonly SecretChoice[];
	value: string;
	onChange(id: string): void;
}>) {
	const { t } = useTranslation("devices");
	const label = t("serviceConfig.secret.which", "Secret");
	if (secrets.length === 1)
		return (
			<div className="flex flex-col gap-1.5">
				<span className="text-[13px]/[18px] font-medium">{label}</span>
				<span data-secret-name="" className="text-ui">
					{secrets[0]?.label}
				</span>
			</div>
		);
	return (
		<Field id="svc-secret-name" label={label}>
			<Select value={value} onValueChange={onChange}>
				<SelectTrigger id="svc-secret-name" className={SELECT_TRIGGER}>
					<SelectValue />
				</SelectTrigger>
				<SelectContent className={SELECT_CONTENT}>
					{secrets.map((secret) => (
						<SelectItem
							key={secret.id}
							value={secret.id}
							className={SELECT_ITEM}
						>
							{secret.label}
						</SelectItem>
					))}
				</SelectContent>
			</Select>
		</Field>
	);
}

/** "Change secret value…": pick a stored secret, type its new value, send it. Works while the service runs. */
export function ChangeSecretSheet({
	open,
	onOpenChange,
	serviceId,
	deviceLabel,
	secrets,
	revision,
	writer,
	onSent,
}: Readonly<{
	open: boolean;
	onOpenChange(open: boolean): void;
	serviceId: string;
	deviceLabel: string;
	secrets: readonly SecretChoice[];
	revision: number;
	writer: SecretWrite;
	onSent(label: string): void;
}>) {
	const { t } = useTranslation("devices");
	const sendError = useSendError();
	const [picked, setPicked] = useState(secrets[0]?.id ?? "");
	const [value, setValue] = useState("");
	const [error, setError] = useState<string | null>(null);
	const secret = secrets.find((entry) => entry.id === picked) ?? secrets[0];
	const close = () => {
		setValue("");
		setError(null);
		onOpenChange(false);
	};
	const save = async () => {
		if (!secret) return;
		if (!value) {
			setError(t("serviceConfig.secret.empty", "Enter the new value first."));
			return;
		}
		const serialized = serializeSecret(secret.definition, value);
		if (serialized === null) {
			setError(
				t(
					"serviceConfig.secret.wrongType",
					"That doesn't fit {{name}}. {{hint}}",
					{
						name: secret.label,
						hint: typeHint(t, secret.definition) ?? "",
					},
				),
			);
			return;
		}
		if (utf8Bytes(serialized) > SECRET_MAX_BYTES) {
			setError(
				t(
					"serviceConfig.secret.tooLong",
					"The value is {{count, number}} bytes. A secret holds up to {{max, number}}.",
					{ count: utf8Bytes(serialized), max: SECRET_MAX_BYTES },
				),
			);
			return;
		}
		setError(null);
		const outcome = await writer.write({
			reference: secret.reference,
			label: secret.label,
			value: serialized,
			revision,
		});
		const failed = sendError(outcome);
		if (failed) {
			setError(failed);
			return;
		}
		if (outcome.status === "busy" || outcome.status === "cancelled") return;
		setValue("");
		onOpenChange(false);
		onSent(secret.label);
	};
	const hint = typeHint(t, secret?.definition);
	return (
		<DvSheet
			open={open}
			onOpenChange={(next) => (next ? onOpenChange(true) : close())}
			icon={KeyRound}
			title={t("serviceConfig.secret.title", "Change secret value")}
			sub={t(
				"serviceConfig.secret.subtitle",
				"{{service}} · {{device}} · allowed while it runs",
				{ service: serviceId, device: deviceLabel },
			)}
			foot={
				<SheetFoot
					busy={writer.pending}
					confirmLabel={t("serviceConfig.secret.save", "Save new value")}
					onCancel={close}
					onConfirm={() => void save()}
				/>
			}
		>
			<SecretPicker
				secrets={secrets}
				value={secret?.id ?? ""}
				onChange={(id) => {
					setPicked(id);
					setError(null);
				}}
			/>
			<Field
				id="svc-secret-value"
				label={t("serviceConfig.secret.value", "New value")}
				error={error ?? undefined}
				hint={
					<>
						{hint ? <>{hint} </> : null}
						{t(
							"serviceConfig.secret.hint",
							"Sent to the device encrypted. Nobody can read it back, you included. Running instances may keep the old value until they restart.",
						)}
					</>
				}
			>
				<SecretInput
					value={value}
					onValueChange={(next) => {
						setValue(next);
						setError(null);
					}}
					autoComplete="off"
					maxBytes={SECRET_MAX_BYTES}
				/>
			</Field>
		</DvSheet>
	);
}

function randomToken(): string {
	const bytes = new Uint8Array(TOKEN_RANDOM_BYTES);
	crypto.getRandomValues(bytes);
	return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join(
		"",
	);
}

function TokenTools({
	token,
	onGenerate,
}: Readonly<{ token: string; onGenerate(): void }>) {
	const { t } = useTranslation("devices");
	const { copied, copy } = useCopy();
	return (
		<div className="flex flex-wrap items-center gap-2">
			<DvButton
				size="xs"
				icon={RefreshCw}
				data-act="token-generate"
				onClick={onGenerate}
			>
				{t("serviceConfig.token.generate", "Generate one")}
			</DvButton>
			<DvButton
				size="xs"
				variant="ghost"
				icon={copied ? Check : Copy}
				aria-disabled={!token || undefined}
				data-act="token-copy"
				onClick={() => void copy(token)}
			>
				{copied
					? t("serviceConfig.token.copied", "Copied")
					: t("serviceConfig.token.copy", "Copy token")}
			</DvButton>
		</div>
	);
}

/** "Set a new token…": the service page's access token, replaced in place. */
export function NewTokenSheet({
	open,
	onOpenChange,
	serviceId,
	deviceLabel,
	reference,
	revision,
	writer,
	onSent,
}: Readonly<{
	open: boolean;
	onOpenChange(open: boolean): void;
	serviceId: string;
	deviceLabel: string;
	/** `hosting.auth_secret`. */
	reference: string;
	revision: number;
	writer: SecretWrite;
	onSent(): void;
}>) {
	const { t } = useTranslation("devices");
	const sendError = useSendError();
	const [token, setToken] = useState("");
	const [error, setError] = useState<string | null>(null);
	const label = t("serviceConfig.token.name", "Access token");
	const close = () => {
		setToken("");
		setError(null);
		onOpenChange(false);
	};
	const save = async () => {
		if (!TOKEN_PATTERN.test(token)) {
			setError(
				t(
					"serviceConfig.token.invalid",
					"The token is {{count, number}} characters. Use at least {{min, number}} letters, digits or symbols without spaces, or generate one.",
					{ count: token.length, min: TOKEN_MIN_CHARS },
				),
			);
			return;
		}
		setError(null);
		const outcome = await writer.write({
			reference,
			label,
			value: token,
			revision,
		});
		const failed = sendError(outcome);
		if (failed) {
			setError(failed);
			return;
		}
		if (outcome.status === "busy" || outcome.status === "cancelled") return;
		setToken("");
		onOpenChange(false);
		onSent();
	};
	return (
		<DvSheet
			open={open}
			onOpenChange={(next) => (next ? onOpenChange(true) : close())}
			icon={KeyRound}
			title={t("serviceConfig.token.title", "Set a new access token")}
			sub={t("serviceConfig.token.subtitle", "{{service}} · {{device}}", {
				service: serviceId,
				device: deviceLabel,
			})}
			foot={
				<SheetFoot
					busy={writer.pending}
					confirmLabel={t("serviceConfig.token.save", "Set new token")}
					onCancel={close}
					onConfirm={() => void save()}
				/>
			}
		>
			<Field
				id="svc-token-value"
				label={t("serviceConfig.token.value", "New token")}
				error={error ?? undefined}
				hint={t(
					"serviceConfig.token.hint",
					"Copy it before you save: it can't be read back afterwards.",
				)}
			>
				<SecretInput
					value={token}
					onValueChange={(next) => {
						setToken(next);
						setError(null);
					}}
					autoComplete="off"
				/>
			</Field>
			<TokenTools
				token={token}
				onGenerate={() => {
					setToken(randomToken());
					setError(null);
				}}
			/>
			<ConsequencePreview
				compact
				rows={{
					what: t(
						"serviceConfig.token.what",
						"The service page asks for the new token. The old one stops working once the device saves the new one.",
					),
					who: t(
						"serviceConfig.token.who",
						"Everyone who uses the old token, including open chats and scripts calling the REST or MCP endpoints.",
					),
					when: t(
						"serviceConfig.token.when",
						"When the device saves it, usually within seconds. Activity shows it.",
					),
					undo: {
						reversible: false,
						text: t(
							"serviceConfig.token.undo",
							"The old token can't be read back. Set another new one instead.",
						),
					},
				}}
			/>
			<p className="text-xs text-muted-foreground">
				{t(
					"serviceConfig.token.private",
					"The token goes to the device inside the encrypted live connection. The hub never sees it.",
				)}
			</p>
		</DvSheet>
	);
}
