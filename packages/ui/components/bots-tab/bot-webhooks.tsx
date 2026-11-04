"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CodeIcon,
	CopyIcon,
	Loader2Icon,
	PlusIcon,
	RefreshCwIcon,
	Trash2Icon,
	XIcon,
} from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import {
	useBotWebhookUrl,
	useBotWebhooks,
	useDeleteBotWebhook,
	useIssueBotWebhook,
} from "../../hooks/use-bots";
import { formatRelativeTime } from "../../lib/date";
import type {
	IBot,
	IBotWebhook,
	IIssuedBotWebhook,
} from "../../state/backend-state/bot-state";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Textarea } from "../ui/textarea";
import {
	authenticatedWebhookUrl,
	webhookCurlExamples,
} from "./bot-webhook-examples";

type Issued = IIssuedBotWebhook & { url: string };

/** Whether the Details tab offers webhooks for this bot. */
export function offersWebhooks(bot: IBot): boolean {
	return !bot.local && bot.capabilities.includes("webhook");
}

function CopyField({
	label,
	value,
	multiline = false,
}: Readonly<{ label: string; value: string; multiline?: boolean }>) {
	const { t } = useTranslation("bots");
	const copy = () => {
		void navigator.clipboard
			.writeText(value)
			.then(() => toast.success(t("panel.webhooks.copied", "Copied")));
	};
	return (
		<div className="flex flex-col gap-1">
			<span className="text-xs text-muted-foreground">{label}</span>
			<div className="flex items-start gap-1.5">
				{multiline ? (
					<pre className="min-w-0 flex-1 overflow-x-auto rounded-md border bg-background p-2 text-xs">
						<code>{value}</code>
					</pre>
				) : (
					<Input
						readOnly
						aria-label={label}
						value={value}
						className="h-8 font-mono text-xs"
					/>
				)}
				<Button
					type="button"
					variant="ghost"
					size="icon"
					className="size-8 shrink-0"
					aria-label={t("panel.webhooks.copy", "Copy {{label}}", { label })}
					onClick={copy}
				>
					<CopyIcon aria-hidden />
				</Button>
			</div>
		</div>
	);
}

export function WebhookExamples({
	url,
	secret,
}: Readonly<{ url: string; secret?: string }>) {
	const { t } = useTranslation("bots");
	const examples = webhookCurlExamples(url, secret ?? "YOUR_WEBHOOK_SECRET");
	return (
		<div
			className="flex min-w-0 flex-col gap-2.5"
			data-slot="bot-webhook-examples"
		>
			<p className="text-xs leading-relaxed text-muted-foreground">
				{t(
					"panel.webhooks.authenticationHint",
					"Send the secret in a header or in the URL. HMAC signing is optional.",
				)}
			</p>
			{!secret && (
				<p className="text-xs leading-relaxed text-muted-foreground">
					{t(
						"panel.webhooks.placeholderHint",
						"Replace YOUR_WEBHOOK_SECRET with your saved secret. Rotate the secret if you lost it.",
					)}
				</p>
			)}
			<CopyField
				label={t("panel.webhooks.headerExample", "Secret in a header")}
				value={examples.header}
				multiline
			/>
			<CopyField
				label={t("panel.webhooks.urlExample", "Secret in the URL")}
				value={examples.url}
				multiline
			/>
			<p className="text-xs leading-relaxed text-muted-foreground">
				{t(
					"panel.webhooks.responseHint",
					"Without wait, the request returns 202 with the turn state. Add ?wait=30 to the URL, or &wait=30 if it already has a query, to wait up to 30 seconds for a reply.",
				)}
			</p>
		</div>
	);
}

export function IssuedCredentials({
	issued,
	onDone,
}: Readonly<{ issued: Issued; onDone: () => void }>) {
	const { t } = useTranslation("bots");
	return (
		<div
			data-slot="bot-webhook-issued"
			className="flex min-w-0 flex-col gap-2.5 rounded-lg border bg-muted/40 p-3"
		>
			<p className="text-sm font-medium">
				{t("panel.webhooks.issued", "{{name}} is ready", {
					name: issued.webhook.name,
				})}
			</p>
			<p className="text-xs leading-relaxed text-muted-foreground">
				{t(
					"panel.webhooks.once",
					"Copy the secret now. It is shown only once; rotate the webhook if you lose it.",
				)}
			</p>
			<CopyField label={t("panel.webhooks.url", "URL")} value={issued.url} />
			<CopyField
				label={t("panel.webhooks.secret", "Secret")}
				value={issued.secret}
			/>
			<CopyField
				label={t("panel.webhooks.authenticatedUrl", "URL with secret")}
				value={authenticatedWebhookUrl(issued.url, issued.secret)}
			/>
			<p className="text-xs leading-relaxed text-muted-foreground">
				{t(
					"panel.webhooks.authenticatedUrlHint",
					"For services that only accept a destination URL, use this URL. Anyone with it can call the bot, so keep it private.",
				)}
			</p>
			<h4 className="text-sm font-medium">
				{t("panel.webhooks.examples", "curl examples")}
			</h4>
			<WebhookExamples url={issued.url} secret={issued.secret} />
			<details className="min-w-0 text-xs">
				<summary className="cursor-pointer py-1 font-medium">
					{t("panel.webhooks.optionalSigning", "Optional HMAC signing")}
				</summary>
				<div className="flex flex-col gap-2 pt-2">
					<p className="leading-relaxed text-muted-foreground">
						{t(
							"panel.webhooks.signingHint",
							"Use the signing key as text to sign the exact request body with HMAC-SHA256. Send the lowercase hex digest in X-Flow-Like-Signature: sha256=<hex>. No secret header or URL parameter is needed when signing.",
						)}
					</p>
					<CopyField
						label={t("panel.webhooks.signingKey", "Signing key (HMAC-SHA256)")}
						value={issued.signing_key}
					/>
				</div>
			</details>
			<Button type="button" size="sm" className="self-end" onClick={onDone}>
				{t("panel.webhooks.done", "I saved it")}
			</Button>
		</div>
	);
}

function NewWebhook({
	busy,
	onCreate,
}: Readonly<{
	busy: boolean;
	onCreate: (request: { name: string; instruction: string }) => void;
}>) {
	const { t } = useTranslation("bots");
	const [name, setName] = useState("");
	const [instruction, setInstruction] = useState("");
	return (
		<form
			className="flex flex-col gap-2"
			onSubmit={(event) => {
				event.preventDefault();
				onCreate({ name, instruction });
				setName("");
				setInstruction("");
			}}
		>
			<Input
				value={name}
				maxLength={64}
				onChange={(event) => setName(event.target.value)}
				placeholder={t("panel.webhooks.name", "Name, e.g. New order")}
				aria-label={t("panel.webhooks.nameLabel", "Webhook name")}
			/>
			<Textarea
				value={instruction}
				maxLength={4000}
				rows={2}
				onChange={(event) => setInstruction(event.target.value)}
				placeholder={t(
					"panel.webhooks.instruction",
					"What should the bot do with each call?",
				)}
				aria-label={t("panel.webhooks.instructionLabel", "Instruction")}
			/>
			<Button
				type="submit"
				size="sm"
				className="self-start"
				disabled={busy || !name.trim()}
			>
				{busy ? (
					<Loader2Icon className="animate-spin" aria-hidden />
				) : (
					<PlusIcon aria-hidden />
				)}
				{t("panel.webhooks.create", "Create webhook")}
			</Button>
		</form>
	);
}

function WebhookRow({
	webhook,
	busy,
	onExamples,
	onRotate,
	onDelete,
}: Readonly<{
	webhook: IBotWebhook;
	busy: boolean;
	onExamples: (webhook: IBotWebhook) => void;
	onRotate: (webhook: IBotWebhook) => void;
	onDelete: (webhook: IBotWebhook) => void;
}>) {
	const { t } = useTranslation("bots");
	const lastCalled = webhook.last_called_at
		? t("panel.webhooks.lastCalled", "Last called {{when}}", {
				when: formatRelativeTime(webhook.last_called_at, "long", ""),
			})
		: t("panel.webhooks.neverCalled", "Never called");
	return (
		<li className="flex items-center gap-2 rounded-lg bg-muted/60 py-1.5 pr-1.5 pl-3">
			<div className="min-w-0 flex-1">
				<p className="truncate text-sm font-medium">{webhook.name}</p>
				<p className="truncate text-xs text-muted-foreground">{lastCalled}</p>
			</div>
			<Button
				type="button"
				variant="ghost"
				size="icon"
				className="size-7 shrink-0 text-muted-foreground"
				aria-label={t("panel.webhooks.examples", "curl examples")}
				onClick={() => onExamples(webhook)}
			>
				<CodeIcon aria-hidden />
			</Button>
			<Button
				type="button"
				variant="ghost"
				size="icon"
				className="size-7 shrink-0 text-muted-foreground"
				disabled={busy}
				aria-label={t(
					"panel.webhooks.rotate",
					"Rotate the secret of {{name}}",
					{
						name: webhook.name,
					},
				)}
				onClick={() => onRotate(webhook)}
			>
				<RefreshCwIcon aria-hidden />
			</Button>
			<Button
				type="button"
				variant="ghost"
				size="icon"
				className="size-7 shrink-0 text-muted-foreground"
				disabled={busy}
				aria-label={t("panel.webhooks.delete", "Delete {{name}}", {
					name: webhook.name,
				})}
				onClick={() => onDelete(webhook)}
			>
				<Trash2Icon aria-hidden />
			</Button>
		</li>
	);
}

/** Public URLs that let other systems wake the bot; the secret is shown once. */
export function BotWebhooks({ bot }: Readonly<{ bot: IBot }>) {
	const { t } = useTranslation("bots");
	const webhooks = useBotWebhooks(bot.id);
	const issue = useIssueBotWebhook();
	const remove = useDeleteBotWebhook();
	const urlOf = useBotWebhookUrl();
	const [issued, setIssued] = useState<Issued | null>(null);
	const [example, setExample] = useState<{
		webhook: IBotWebhook;
		url: string;
	} | null>(null);
	const show = async (result?: IIssuedBotWebhook) => {
		if (!result) return;
		setIssued({ ...result, url: await urlOf(bot.id, result.webhook) });
		setExample(null);
	};
	const onError = (error: Error) => toast.error(error.message);
	return (
		<section className="flex flex-col gap-3" data-slot="bot-webhooks">
			<h3 className="text-xs text-muted-foreground">
				{t("panel.webhooks.title", "Webhooks")}
			</h3>
			{issued && (
				<IssuedCredentials issued={issued} onDone={() => setIssued(null)} />
			)}
			{example && (
				<div className="flex min-w-0 flex-col gap-2.5 rounded-lg border p-3">
					<div className="flex items-center justify-between gap-2">
						<h4 className="text-sm font-medium">
							{example.webhook.name}:{" "}
							{t("panel.webhooks.examples", "curl examples")}
						</h4>
						<Button
							type="button"
							variant="ghost"
							size="icon"
							className="size-7 shrink-0"
							aria-label={t("panel.webhooks.closeExamples", "Close examples")}
							onClick={() => setExample(null)}
						>
							<XIcon aria-hidden />
						</Button>
					</div>
					<WebhookExamples url={example.url} />
				</div>
			)}
			<ul className="flex flex-col gap-1.5">
				{(webhooks.data ?? []).map((webhook) => (
					<WebhookRow
						key={webhook.id}
						webhook={webhook}
						busy={issue.isPending || remove.isPending}
						onExamples={(webhook) => {
							if (example?.webhook.id === webhook.id) {
								setExample(null);
								return;
							}
							void urlOf(bot.id, webhook)
								.then((url) => setExample({ webhook, url }))
								.catch(onError);
						}}
						onRotate={(hook) =>
							issue.mutate(
								{ botId: bot.id, hookId: hook.id },
								{ onSuccess: show, onError },
							)
						}
						onDelete={(hook) =>
							remove.mutate(
								{ botId: bot.id, hookId: hook.id },
								{
									onError,
									onSuccess: () => {
										setExample((current) =>
											current?.webhook.id === hook.id ? null : current,
										);
										setIssued((current) =>
											current?.webhook.id === hook.id ? null : current,
										);
									},
								},
							)
						}
					/>
				))}
			</ul>
			{webhooks.data?.length === 0 && (
				<p className="text-sm text-muted-foreground">
					{t(
						"panel.webhooks.none",
						"No webhooks yet. A webhook gives other systems a URL that wakes this bot with their data.",
					)}
				</p>
			)}
			<NewWebhook
				busy={issue.isPending}
				onCreate={(request) =>
					issue.mutate({ botId: bot.id, request }, { onSuccess: show, onError })
				}
			/>
		</section>
	);
}
