"use client";

import { useTranslation } from "@flow-like/locales";
import { BanIcon, KeyRoundIcon } from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";
import { BOT_LIMITS } from "../../state/backend-state/bot-state";
import {
	RequestCardBody,
	RequestCardFooter,
	requestButtonClass,
} from "../interfaces/chat-default/request-card";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
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

/**
 * The masked input of a bot's secret request, with Save and Decline. The typed value lives only in
 * this component's state: `onSave` receives it once, and it is cleared after a successful save and
 * when the form unmounts. It is never written to a cache, a log or a response.
 */
export function SecretRequestForm({
	secret,
	botName,
	onSave,
	onDecline,
	disabled = false,
}: Readonly<{
	secret: SecretRequestDetails;
	botName: string;
	onSave: (value: string) => Promise<void>;
	onDecline: () => void;
	disabled?: boolean;
}>) {
	const { t } = useTranslation("bots");
	const errorMessage = useBotErrorMessage();
	const ids = useId();
	const [value, setValue] = useState("");
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

	const save = async () => {
		if (problem || busy) return;
		setError(null);
		setSaving(true);
		try {
			await onSave(value);
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

	return (
		<form
			className="contents"
			autoComplete="off"
			onSubmit={(event) => {
				event.preventDefault();
				void save();
			}}
		>
			<RequestCardBody dense className="overflow-y-auto overscroll-contain">
				<div className="flex flex-col gap-1.5">
					<p className="flex flex-wrap items-center gap-1.5 text-xs">
						<KeyRoundIcon
							className="size-3.5 text-muted-foreground"
							aria-hidden
						/>
						<code
							data-slot="secret-name"
							className="rounded-md border border-border/60 bg-muted/60 px-1.5 py-0.5 font-mono text-[11px] text-foreground/80 break-all"
						>
							{secret.name}
						</code>
					</p>
					<p
						data-slot="secret-description"
						className="text-sm leading-snug whitespace-pre-wrap"
					>
						{secret.description}
					</p>
					{secret.replace && (
						<p data-slot="secret-replace" className="text-xs text-warning">
							{t(
								"crew.approval.secret.replaceHint",
								"The stored value was rejected. Enter a new one to replace it.",
							)}
						</p>
					)}
				</div>
				<div className="flex flex-col gap-1.5">
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
						placeholder={t(
							"crew.approval.secret.placeholder",
							"Paste the value",
						)}
						onChange={(event) => setValue(event.target.value)}
					/>
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
				</div>
			</RequestCardBody>
			<RequestCardFooter
				className="px-3.5 py-2.5"
				start={
					error && (
						<p role="alert" className="text-xs text-destructive">
							{error}
						</p>
					)
				}
			>
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
					disabled={busy || problem !== null}
				>
					<KeyRoundIcon aria-hidden />
					{saving
						? t("crew.approval.secret.saving", "Saving...")
						: t("crew.approval.secret.save", "Save secret")}
				</Button>
			</RequestCardFooter>
		</form>
	);
}
