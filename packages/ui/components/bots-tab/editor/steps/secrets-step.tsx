"use client";

import { useTranslation } from "@flow-like/locales";
import { KeyRound, LockKeyhole, Plus } from "lucide-react";
import { useId, useState } from "react";
import { toast } from "sonner";
import { useBotSecretSealing } from "../../../../hooks/use-bot-secret-sealing";
import {
	useBot,
	useBotLabels,
	useBotSecrets,
	useDeleteBotSecret,
	useUpsertBotSecret,
} from "../../../../hooks/use-bots";
import {
	type BotSecretSealing,
	sealsSecret,
} from "../../../../lib/bot-secret-sealing";
import {
	BOT_LIMITS,
	BOT_SECRET_NAME_PATTERN,
	type IBotSecretInfo,
} from "../../../../state/backend-state/bot-state";
import { Button } from "../../../ui/button";
import { Input } from "../../../ui/input";
import { Label } from "../../../ui/label";
import { RelativeTime } from "../../../ui/relative-time";
import { Skeleton } from "../../../ui/skeleton";
import { SecretSealChoice } from "../../secret-request-form";
import { useBotErrorMessage } from "../bot-editor-copy";
import { BotNotice, BotStepPanel, BotTag } from "../bot-editor-parts";

interface SecretFormState {
	name: string;
	value: string;
	/** `null` until the user writes one: the field then shows the description of the secret the name points at. */
	description: string | null;
	/** Replacing keeps the name fixed. */
	replacing: boolean;
}

type SecretFormProblem =
	| "nameInvalid"
	| "sealed"
	| "valueMissing"
	| "valueTooLarge"
	| "limit";

const EMPTY_FORM: SecretFormState = {
	name: "",
	value: "",
	description: null,
	replacing: false,
};

/** Env-style names: upper case, underscores for spaces and dashes. */
export function toBotSecretName(input: string): string {
	return input.toUpperCase().replace(/[\s-]+/g, "_");
}

export function botSecretValueBytes(value: string): number {
	return new TextEncoder().encode(value).length;
}

export function SecretsStep({ botId }: Readonly<{ botId: string }>) {
	const { t } = useTranslation("bots");
	const secrets = useBotSecrets(botId);
	const sealing = useBotSecretSealing(botId);
	const [form, setForm] = useState<SecretFormState | null>(null);
	const list = secrets.data ?? [];
	const full = list.length >= BOT_LIMITS.secrets;
	return (
		<BotStepPanel
			title={t("editor.secrets.title")}
			subtitle={t("editor.secrets.subtitle")}
			actions={
				!form && (
					<Button
						size="sm"
						variant="outline"
						disabled={full || secrets.isLoading}
						onClick={() => setForm(EMPTY_FORM)}
					>
						<Plus />
						{t("editor.secrets.add")}
					</Button>
				)
			}
		>
			<BotNotice>
				<span className="inline-flex items-center gap-1.5">
					<LockKeyhole className="size-3.5" aria-hidden />
					{t("editor.secrets.writeOnly")}
				</span>
			</BotNotice>
			{form && (
				<SecretForm
					key={form.replacing ? form.name : "new"}
					botId={botId}
					initial={form}
					existing={list}
					sealing={sealing}
					onDone={() => setForm(null)}
				/>
			)}
			{full && (
				<p className="text-xs text-muted-foreground">
					{t("editor.secrets.limit", { max: BOT_LIMITS.secrets })}
				</p>
			)}
			<SecretList
				botId={botId}
				secrets={list}
				sealing={sealing}
				isLoading={secrets.isLoading}
				isError={secrets.isError}
				onReplace={(secret) =>
					setForm({
						name: secret.name,
						value: "",
						description: secret.description ?? "",
						replacing: true,
					})
				}
			/>
		</BotStepPanel>
	);
}

function SecretList({
	botId,
	secrets,
	sealing,
	isLoading,
	isError,
	onReplace,
}: Readonly<{
	botId: string;
	secrets: readonly IBotSecretInfo[];
	sealing: BotSecretSealing;
	isLoading: boolean;
	isError: boolean;
	onReplace: (secret: IBotSecretInfo) => void;
}>) {
	const { t } = useTranslation("bots");
	if (isLoading) return <Skeleton className="h-24 w-full" />;
	if (isError)
		return (
			<p role="alert" className="text-xs text-destructive">
				{t("editor.secrets.loadFailed")}
			</p>
		);
	if (!secrets.length)
		return (
			<div className="flex flex-col items-center gap-2 rounded-xl border border-dashed border-border px-4 py-8 text-center">
				<KeyRound className="size-5 text-muted-foreground" aria-hidden />
				<p className="text-xs text-muted-foreground">
					{t("editor.secrets.empty")}
				</p>
			</div>
		);
	return (
		<ul className="divide-y divide-border rounded-xl border border-border bg-card">
			{secrets.map((secret) => (
				<SecretRow
					key={secret.name}
					botId={botId}
					secret={secret}
					resealable={sealing === "available"}
					onReplace={onReplace}
				/>
			))}
		</ul>
	);
}

function SecretRow({
	botId,
	secret,
	resealable,
	onReplace,
}: Readonly<{
	botId: string;
	secret: IBotSecretInfo;
	/** A sealed value can be replaced here by sealing the new one to the bot's key. */
	resealable: boolean;
	onReplace: (secret: IBotSecretInfo) => void;
}>) {
	const { t } = useTranslation("bots");
	const errorMessage = useBotErrorMessage();
	const remove = useDeleteBotSecret();
	const sealedHintId = useId();
	const [confirming, setConfirming] = useState(false);
	const sealed = secret.encryption === "sealed";
	const locked = sealed && !resealable;
	const deleteSecret = () =>
		remove.mutate(
			{ botId, name: secret.name },
			{
				onSuccess: () =>
					toast.success(t("editor.secrets.deleted", { name: secret.name })),
				onError: (error) =>
					toast.error(errorMessage(error, t("editor.secrets.deleteFailed"))),
				onSettled: () => setConfirming(false),
			},
		);
	return (
		<li className="flex flex-col gap-2 px-4 py-3 @lg:flex-row @lg:items-center">
			<div className="flex min-w-0 flex-1 flex-col gap-1">
				<span className="flex flex-wrap items-center gap-1.5">
					<code className="font-mono text-sm font-medium break-all">
						{secret.name}
					</code>
					{sealed && (
						<BotTag className="text-[0.625rem]">
							{t("editor.secrets.sealed")}
						</BotTag>
					)}
				</span>
				{secret.description && (
					<span className="text-xs text-muted-foreground">
						{secret.description}
					</span>
				)}
				<span className="text-[0.6875rem] text-muted-foreground">
					{t("editor.secrets.updated")}{" "}
					<RelativeTime value={secret.updated_at} />
					{" · "}
					{secret.last_used_at ? (
						<>
							{t("editor.secrets.lastUsed")}{" "}
							<RelativeTime value={secret.last_used_at} />
						</>
					) : (
						t("editor.secrets.neverUsed")
					)}
				</span>
				{locked && (
					<span
						id={sealedHintId}
						className="text-[0.6875rem] text-muted-foreground"
					>
						{t("editor.secrets.sealedHint")}
					</span>
				)}
			</div>
			{confirming ? (
				<div className="flex flex-wrap items-center gap-2">
					<span className="text-xs">
						{t("editor.secrets.deleteConfirm", { name: secret.name })}
					</span>
					<Button
						size="sm"
						variant="ghost"
						onClick={() => setConfirming(false)}
					>
						{t("editor.actions.cancel")}
					</Button>
					<Button
						size="sm"
						variant="destructive"
						disabled={remove.isPending}
						onClick={deleteSecret}
					>
						{t("editor.secrets.delete")}
					</Button>
				</div>
			) : (
				<div className="flex items-center gap-1">
					<Button
						size="sm"
						variant="ghost"
						disabled={locked}
						aria-describedby={locked ? sealedHintId : undefined}
						onClick={() => onReplace(secret)}
					>
						{t("editor.secrets.replaceValue")}
					</Button>
					<Button
						size="sm"
						variant="ghost"
						className="text-destructive"
						onClick={() => setConfirming(true)}
					>
						{t("editor.secrets.delete")}
					</Button>
				</div>
			)}
		</li>
	);
}

/**
 * A sealed secret is only readable by the bot's host; without sealing here a new value would be
 * stored readable by the server, so it is replaced only where it can be sealed again.
 */
export function secretFormProblem(
	form: SecretFormState,
	existing: readonly IBotSecretInfo[],
	sealing: BotSecretSealing = "unavailable",
): SecretFormProblem | null {
	if (!BOT_SECRET_NAME_PATTERN.test(form.name)) return "nameInvalid";
	const match = existing.find((secret) => secret.name === form.name);
	if (match?.encryption === "sealed" && sealing !== "available")
		return "sealed";
	if (!form.value) return "valueMissing";
	if (botSecretValueBytes(form.value) > BOT_LIMITS.secretValueBytes)
		return "valueTooLarge";
	if (!match && existing.length >= BOT_LIMITS.secrets) return "limit";
	return null;
}

/** A blank description follows the name again, so naming an existing secret shows (and keeps) its description. */
function renameSecretForm(
	form: SecretFormState,
	name: string,
): SecretFormState {
	return {
		...form,
		name,
		description: form.description?.trim() ? form.description : null,
	};
}

function SecretNameHint({
	id,
	name,
	problem,
	taken,
}: Readonly<{
	id: string;
	name: string;
	problem: SecretFormProblem | null;
	taken: boolean;
}>) {
	const { t } = useTranslation("bots");
	if (problem === "sealed")
		return (
			<p id={id} className="text-xs text-destructive">
				{t("editor.secrets.nameSealed", { name })}
			</p>
		);
	if (problem === "nameInvalid" && name)
		return (
			<p id={id} className="text-xs text-destructive">
				{t("editor.secrets.nameInvalid")}
			</p>
		);
	return (
		<p id={id} className="text-xs text-muted-foreground">
			{taken
				? t("editor.secrets.nameTaken", { name })
				: t("editor.secrets.nameHint")}
		</p>
	);
}

function SecretForm({
	botId,
	initial,
	existing,
	sealing,
	onDone,
}: Readonly<{
	botId: string;
	initial: SecretFormState;
	existing: readonly IBotSecretInfo[];
	sealing: BotSecretSealing;
	onDone: () => void;
}>) {
	const { t } = useTranslation("bots");
	const errorMessage = useBotErrorMessage();
	const upsert = useUpsertBotSecret();
	const bot = useBot(botId);
	const labels = useBotLabels();
	const ids = useId();
	const [form, setForm] = useState(initial);
	const [sealChosen, setSealChosen] = useState(true);
	const [error, setError] = useState<string | null>(null);
	const problem = secretFormProblem(form, existing, sealing);
	const match = existing.find((secret) => secret.name === form.name);
	const keepsSealed = match?.encryption === "sealed";
	const sealed = sealsSecret(sealing, keepsSealed || sealChosen);
	const description = form.description ?? match?.description ?? "";
	const nameInvalid =
		problem === "sealed" || (problem === "nameInvalid" && !!form.name);
	const save = () => {
		if (problem || sealing === "checking") return;
		setError(null);
		upsert.mutate(
			{
				botId,
				name: form.name,
				request: {
					value: form.value,
					description: description.trim() || null,
				},
				sealed,
			},
			{
				onSuccess: () => {
					toast.success(t("editor.secrets.saved", { name: form.name }));
					onDone();
				},
				onError: (cause) =>
					setError(errorMessage(cause, t("editor.secrets.saveFailed"))),
			},
		);
	};
	return (
		<form
			className="flex flex-col gap-4 rounded-xl border border-border bg-muted/40 p-4"
			onSubmit={(event) => {
				event.preventDefault();
				save();
			}}
		>
			<div className="grid gap-4 @lg:grid-cols-2">
				<div className="flex flex-col gap-1.5">
					<Label htmlFor={`${ids}-name`}>{t("editor.secrets.name")}</Label>
					<Input
						id={`${ids}-name`}
						value={form.name}
						readOnly={form.replacing}
						autoComplete="off"
						spellCheck={false}
						aria-invalid={nameInvalid || undefined}
						aria-describedby={`${ids}-name-hint`}
						placeholder="API_KEY"
						className="font-mono"
						onChange={(event) =>
							setForm(
								renameSecretForm(form, toBotSecretName(event.target.value)),
							)
						}
					/>
					<SecretNameHint
						id={`${ids}-name-hint`}
						name={form.name}
						problem={problem}
						taken={!form.replacing && !!match}
					/>
				</div>
				<div className="flex flex-col gap-1.5">
					<Label htmlFor={`${ids}-value`}>{t("editor.secrets.value")}</Label>
					<Input
						id={`${ids}-value`}
						type="password"
						value={form.value}
						autoComplete="new-password"
						spellCheck={false}
						aria-invalid={problem === "valueTooLarge" || undefined}
						aria-describedby={`${ids}-value-hint`}
						placeholder={t("editor.secrets.valuePlaceholder")}
						onChange={(event) =>
							setForm({ ...form, value: event.target.value })
						}
					/>
					<p
						id={`${ids}-value-hint`}
						className={
							problem === "valueTooLarge"
								? "text-xs text-destructive"
								: "text-xs text-muted-foreground"
						}
					>
						{problem === "valueTooLarge"
							? t("editor.secrets.valueTooLarge", {
									max: BOT_LIMITS.secretValueBytes / 1024,
								})
							: t("editor.secrets.valueHint")}
					</p>
				</div>
			</div>
			<div className="flex flex-col gap-1.5">
				<Label htmlFor={`${ids}-description`}>
					{t("editor.secrets.description")}
				</Label>
				<Input
					id={`${ids}-description`}
					value={description}
					placeholder={t("editor.secrets.descriptionPlaceholder")}
					onChange={(event) =>
						setForm({ ...form, description: event.target.value })
					}
				/>
			</div>
			{problem === "limit" && (
				<p className="text-xs text-destructive">
					{t("editor.secrets.limit", { max: BOT_LIMITS.secrets })}
				</p>
			)}
			{sealing === "available" && (
				<SecretSealChoice
					id={`${ids}-seal`}
					botName={bot.data ? labels(bot.data).name : ""}
					checked={keepsSealed || sealChosen}
					disabled={keepsSealed || upsert.isPending}
					onCheckedChange={setSealChosen}
				/>
			)}
			{error && (
				<p role="alert" className="text-xs text-destructive">
					{error}
				</p>
			)}
			<div className="flex justify-end gap-2">
				<Button type="button" variant="ghost" size="sm" onClick={onDone}>
					{t("editor.actions.cancel")}
				</Button>
				<Button
					type="submit"
					size="sm"
					disabled={!!problem || sealing === "checking" || upsert.isPending}
				>
					{upsert.isPending
						? t("editor.actions.saving")
						: t("editor.secrets.save")}
				</Button>
			</div>
		</form>
	);
}
