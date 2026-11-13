"use client";

import { useTranslation } from "@flow-like/locales";
import { BanIcon, KeyRoundIcon } from "lucide-react";
import { type FormEvent, useEffect, useId, useRef, useState } from "react";
import {
	type BotSecretSealing,
	sealsSecret,
} from "../../lib/bot-secret-sealing";
import { BOT_LIMITS } from "../../state/backend-state/bot-state";
import {
	RequestCardBody,
	RequestCardFooter,
	requestButtonClass,
} from "../interfaces/chat-default/request-card";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import { Switch } from "../ui/switch";
import { useBotErrorMessage } from "./editor/bot-editor-copy";

export interface SecretRequestDetails {
	name: string;
	description: string;
	/** The stored value was rejected; the owner is asked for a new one. */
	replace?: boolean;
}

export type SecretValueProblem = "missing" | "tooLarge";

export function secretValueProblem(value: string): SecretValueProblem | null {
	if (!value) return "missing";
	return new TextEncoder().encode(value).length > BOT_LIMITS.secretValueBytes
		? "tooLarge"
		: null;
}

/** The owner's choice to seal the value to the bot's own key, where the bot has one. */
export function SecretSealChoice({
	id,
	botName,
	checked,
	disabled,
	onCheckedChange,
}: Readonly<{
	id: string;
	botName: string;
	checked: boolean;
	disabled?: boolean;
	onCheckedChange: (checked: boolean) => void;
}>) {
	const { t } = useTranslation("bots");
	return (
		<div data-slot="secret-seal" className="flex items-start gap-2">
			<Switch
				id={id}
				checked={checked}
				disabled={disabled}
				onCheckedChange={onCheckedChange}
				className="mt-0.5"
			/>
			<Label
				htmlFor={id}
				className="flex flex-col items-start gap-0.5 text-xs font-normal"
			>
				<span className="font-medium">
					{t("crew.approval.secret.seal", "Seal to {{name}}'s key", {
						name: botName,
					})}
				</span>
				<span className="text-muted-foreground">
					{t(
						"crew.approval.secret.sealHint",
						"Only the desktop or device that runs {{name}} can read it. The server keeps it but can't open it.",
						{ name: botName },
					)}
				</span>
			</Label>
		</div>
	);
}

/**
 * The masked input of a bot's secret request, with Save and Decline. The typed value lives only in
 * this component's state: `onSave` receives it once, and it is cleared after a successful save and
 * when the form unmounts. It is never written to a cache, a log or a response.
 * `compact` renders it as one row of a batch card instead of the card's body and footer.
 * Where `sealing` is available the value is sealed to the bot's key unless the owner turns it off.
 */
export function SecretRequestForm({
	secret,
	botName,
	onSave,
	onDecline,
	disabled = false,
	compact = false,
	sealing = "unavailable",
}: Readonly<{
	secret: SecretRequestDetails;
	botName: string;
	onSave: (value: string, sealed: boolean) => Promise<void>;
	onDecline: () => void;
	disabled?: boolean;
	compact?: boolean;
	sealing?: BotSecretSealing;
}>) {
	const { t } = useTranslation("bots");
	const errorMessage = useBotErrorMessage();
	const ids = useId();
	const [value, setValue] = useState("");
	const [sealChosen, setSealChosen] = useState(true);
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const mounted = useRef(true);
	useEffect(() => {
		mounted.current = true;
		return () => {
			mounted.current = false;
		};
	}, []);
	const problem = secretValueProblem(value);
	const busy = saving || disabled;
	const checking = sealing === "checking";

	const save = async () => {
		if (problem || busy || checking) return;
		setError(null);
		setSaving(true);
		try {
			await onSave(value, sealsSecret(sealing, sealChosen));
			if (mounted.current) setValue("");
		} catch (cause) {
			if (mounted.current)
				setError(
					errorMessage(
						cause,
						t(
							"crew.approval.secret.failed",
							"Couldn't save the secret. Try again.",
						),
					),
				);
		} finally {
			if (mounted.current) setSaving(false);
		}
	};

	const submit = (event: FormEvent) => {
		event.preventDefault();
		void save();
	};
	const name = (
		<code
			data-slot="secret-name"
			className="rounded-md border border-border/60 bg-muted/60 px-1.5 py-0.5 font-mono text-[11px] text-foreground/80 break-all"
		>
			{secret.name}
		</code>
	);
	const replaceHint = secret.replace && (
		<p data-slot="secret-replace" className="text-xs text-warning">
			{t(
				"crew.approval.secret.replaceHint",
				"The stored value was rejected. Enter a new one to replace it.",
			)}
		</p>
	);
	const input = (
		<>
			<Label htmlFor={`${ids}-value`} className="sr-only">
				{t("crew.approval.secret.valueLabel", "Secret value")}
			</Label>
			<Input
				id={`${ids}-value`}
				type="password"
				name={`secret-${ids}`}
				value={value}
				autoComplete="new-password"
				autoCorrect="off"
				autoCapitalize="off"
				spellCheck={false}
				data-1p-ignore
				data-lpignore="true"
				data-form-type="other"
				disabled={busy}
				aria-invalid={problem === "tooLarge" || undefined}
				aria-describedby={`${ids}-hint`}
				placeholder={t("crew.approval.secret.placeholder", "Paste the value")}
				className={compact ? "h-8 min-w-40 flex-1 text-sm" : undefined}
				onChange={(event) => setValue(event.target.value)}
			/>
		</>
	);
	const hint = (
		<p
			id={`${ids}-hint`}
			className={
				problem === "tooLarge"
					? "text-xs text-destructive"
					: "text-xs text-muted-foreground"
			}
		>
			{problem === "tooLarge"
				? t(
						"crew.approval.secret.tooLarge",
						"Too large. The limit is {{max}} KB.",
						{ max: BOT_LIMITS.secretValueBytes / 1024 },
					)
				: t(
						"crew.approval.secret.hint",
						"Goes straight into {{name}}'s secret store. It never appears in the chat and the model never sees it.",
						{ name: botName },
					)}
		</p>
	);
	const sealChoice = sealing === "available" && (
		<SecretSealChoice
			id={`${ids}-seal`}
			botName={botName}
			checked={sealChosen}
			disabled={busy}
			onCheckedChange={setSealChosen}
		/>
	);
	const alert = error && (
		<p role="alert" className="text-xs text-destructive">
			{error}
		</p>
	);
	const actions = (
		<>
			<Button
				type="button"
				size="sm"
				variant="ghost"
				className={requestButtonClass}
				disabled={busy}
				onClick={onDecline}
			>
				<BanIcon aria-hidden />
				{t("crew.approval.secret.decline", "Decline")}
			</Button>
			<Button
				type="submit"
				size="sm"
				className={requestButtonClass}
				disabled={busy || checking || problem !== null}
			>
				<KeyRoundIcon aria-hidden />
				{saving
					? t("crew.approval.secret.saving", "Saving...")
					: t("crew.approval.secret.save", "Save secret")}
			</Button>
		</>
	);

	if (compact) {
		return (
			<form
				data-slot="secret-row"
				className="flex flex-col gap-1.5"
				autoComplete="off"
				onSubmit={submit}
			>
				<p className="flex min-w-0 flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
					<KeyRoundIcon className="size-3.5" aria-hidden />
					{name}
					<span aria-hidden>·</span>
					<span>{t("crew.approval.secret.eyebrow", "Needs a secret")}</span>
				</p>
				<p
					data-slot="secret-description"
					className="text-[13px] leading-snug whitespace-pre-wrap"
				>
					{secret.description}
				</p>
				{replaceHint}
				<div className="flex flex-wrap items-center justify-end gap-2">
					{input}
					<div className="flex shrink-0 items-center gap-2">{actions}</div>
				</div>
				{sealChoice}
				{alert ?? hint}
			</form>
		);
	}

	return (
		<form className="contents" autoComplete="off" onSubmit={submit}>
			<RequestCardBody dense className="overflow-y-auto overscroll-contain">
				<div className="flex flex-col gap-1.5">
					<p className="flex flex-wrap items-center gap-1.5 text-xs">
						<KeyRoundIcon
							className="size-3.5 text-muted-foreground"
							aria-hidden
						/>
						{name}
					</p>
					<p
						data-slot="secret-description"
						className="text-sm leading-snug whitespace-pre-wrap"
					>
						{secret.description}
					</p>
					{replaceHint}
				</div>
				<div className="flex flex-col gap-1.5">
					{input}
					{hint}
				</div>
				{sealChoice}
			</RequestCardBody>
			<RequestCardFooter className="px-3.5 py-2.5" start={alert}>
				{actions}
			</RequestCardFooter>
		</form>
	);
}
