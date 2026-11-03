"use client";

import { useTranslation } from "@flow-like/locales";
import { useQueryClient } from "@tanstack/react-query";
import {
	ArrowLeftIcon,
	CheckIcon,
	FileIcon,
	Loader2Icon,
	MailIcon,
	ShieldAlertIcon,
	XIcon,
} from "lucide-react";
import Link from "next/link";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import {
	botKeys,
	useBotMailAddress,
	useBotMailThread,
	useBotMailThreads,
	useCheckBotMailAddress,
	useClaimBotMailAddress,
	useReleaseBotMailAddress,
} from "../../hooks/use-bots";
import { formatRelativeTime } from "../../lib/date";
import { getErrorMessage } from "../../lib/error-message";
import { useBackend } from "../../state/backend-state";
import type {
	IBot,
	IBotMail,
	IBotMailAttachment,
	IBotMailThread,
	IMailAddress,
} from "../../state/backend-state/bot-state";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { Input } from "../ui/input";

const ALIAS_PATTERN = /^[a-z0-9](?:[a-z0-9-]{1,62})[a-z0-9]$/;
const CHECK_DELAY_MS = 350;

/** Whether the bot is on an account and has the email ability on. */
export function offersEmail(bot: IBot): boolean {
	return !bot.local && bot.capabilities.includes("email");
}

type Availability = "idle" | "checking" | "free" | "taken";

function useAliasAvailability(botId: string, alias: string, current?: string) {
	const check = useCheckBotMailAddress();
	const [state, setState] = useState<{
		status: Availability;
		reason?: string | null;
	}>({ status: "idle" });
	useEffect(() => {
		const value = alias.trim().toLowerCase();
		if (!value || value === current || !ALIAS_PATTERN.test(value)) {
			setState({ status: "idle" });
			return;
		}
		setState({ status: "checking" });
		let cancelled = false;
		const timer = setTimeout(() => {
			check(botId, value)
				.then((result) => {
					if (cancelled) return;
					setState({
						status: result.available ? "free" : "taken",
						reason: result.reason,
					});
				})
				.catch(() => !cancelled && setState({ status: "idle" }));
		}, CHECK_DELAY_MS);
		return () => {
			cancelled = true;
			clearTimeout(timer);
		};
	}, [alias, botId, check, current]);
	return state;
}

/** Claim, change and release the bot's own address in the Details tab. */
export function BotEmailAddress({ botId }: Readonly<{ botId: string }>) {
	const { t } = useTranslation("bots");
	const address = useBotMailAddress(botId);
	const claim = useClaimBotMailAddress();
	const release = useReleaseBotMailAddress();
	const [alias, setAlias] = useState("");
	const current = address.data?.alias ?? undefined;
	const availability = useAliasAvailability(botId, alias, current);
	const domain = address.data?.domain;
	if (address.data && !address.data.configured) return null;
	const value = alias.trim().toLowerCase();
	const canClaim =
		ALIAS_PATTERN.test(value) &&
		value !== current &&
		availability.status === "free" &&
		!claim.isPending;
	const submit = () =>
		claim.mutate(
			{ botId: botId, alias: value },
			{
				onSuccess: () => {
					setAlias("");
					toast.success(t("panel.mail.claimed", "Email address saved"));
				},
				onError: (error) => toast.error(getErrorMessage(error)),
			},
		);
	return (
		<section
			data-slot="bot-email-address"
			className="flex flex-col gap-2.5"
			aria-label={t("panel.mail.addressTitle", "Email address")}
		>
			<h3 className="text-sm font-medium">
				{t("panel.mail.addressTitle", "Email address")}
			</h3>
			{address.data?.address ? (
				<div className="flex items-center justify-between gap-2 rounded-lg border bg-muted/40 px-3 py-2">
					<span className="min-w-0 truncate font-mono text-sm">
						{address.data.address}
					</span>
					<Button
						type="button"
						variant="ghost"
						size="sm"
						className="h-7 shrink-0 text-xs"
						disabled={release.isPending}
						onClick={() =>
							release.mutate(
								{ botId: botId },
								{ onError: (error) => toast.error(getErrorMessage(error)) },
							)
						}
					>
						{t("panel.mail.release", "Release")}
					</Button>
				</div>
			) : (
				<p className="text-xs text-muted-foreground">
					{t(
						"panel.mail.noAddress",
						"Pick a name and the bot gets its own address. Mail to it wakes the bot; the bot answers with its email tools.",
					)}
				</p>
			)}
			<div className="flex items-center gap-1.5">
				<Input
					value={alias}
					onChange={(event) => setAlias(event.target.value)}
					placeholder={t("panel.mail.aliasPlaceholder", "name")}
					aria-label={t("panel.mail.aliasLabel", "Address name")}
					className="h-8 font-mono text-xs"
					maxLength={64}
				/>
				{domain && (
					<span className="shrink-0 text-xs text-muted-foreground">
						@{domain}
					</span>
				)}
				<Button
					type="button"
					size="sm"
					className="h-8 shrink-0"
					disabled={!canClaim}
					onClick={submit}
				>
					{claim.isPending ? (
						<Loader2Icon className="animate-spin" aria-hidden />
					) : address.data?.address ? (
						t("panel.mail.change", "Change")
					) : (
						t("panel.mail.claim", "Claim")
					)}
				</Button>
			</div>
			<AvailabilityHint availability={availability} alias={value} />
			<p className="text-xs text-muted-foreground">
				{t(
					"panel.mail.localNote",
					"Bots that only live on this computer have no address; move the bot to your account to give it one.",
				)}
			</p>
		</section>
	);
}

function AvailabilityHint({
	availability,
	alias,
}: Readonly<{
	availability: { status: Availability; reason?: string | null };
	alias: string;
}>) {
	const { t } = useTranslation("bots");
	if (!alias) return null;
	if (!ALIAS_PATTERN.test(alias)) {
		return (
			<p className="text-xs text-muted-foreground">
				{t(
					"panel.mail.aliasRules",
					"3-64 letters, numbers or hyphens; start and end with a letter or number.",
				)}
			</p>
		);
	}
	if (availability.status === "checking") {
		return (
			<p className="text-xs text-muted-foreground">
				{t("panel.mail.checking", "Checking...")}
			</p>
		);
	}
	if (availability.status === "free") {
		return (
			<p className="flex items-center gap-1 text-xs text-muted-foreground">
				<CheckIcon className="size-3.5" aria-hidden />
				{t("panel.mail.available", "Available")}
			</p>
		);
	}
	if (availability.status === "taken") {
		return (
			<p className="flex items-center gap-1 text-xs text-destructive">
				<XIcon className="size-3.5" aria-hidden />
				{availability.reason ?? t("panel.mail.taken", "Not available")}
			</p>
		);
	}
	return null;
}

function addressLabel(value: IMailAddress | string | undefined): string {
	if (!value) return "";
	if (typeof value === "string") return value;
	return value.name
		? `${value.name} <${value.email ?? ""}>`
		: (value.email ?? "");
}

function addressList(
	value: IMailAddress[] | string[] | string | undefined,
): string {
	if (!value) return "";
	if (typeof value === "string") return value;
	return (value as (IMailAddress | string)[]).map(addressLabel).join(", ");
}

function workspaceRoute(attachment: IBotMailAttachment): string | null {
	const handle = attachment.handle;
	if (!handle) return null;
	const folder = handle.path.split("/").slice(0, -1).join("/");
	return `/library/config/storage?${new URLSearchParams({ id: handle.app_id, path: folder })}`;
}

function AttachmentChip({
	attachment,
}: Readonly<{ attachment: IBotMailAttachment }>) {
	const route = workspaceRoute(attachment);
	const body = (
		<>
			<FileIcon className="size-3.5 shrink-0" aria-hidden />
			<span className="max-w-40 truncate">{attachment.name}</span>
		</>
	);
	const className =
		"inline-flex items-center gap-1 rounded-full border bg-muted/40 px-2.5 py-1 text-xs";
	if (!route) {
		return <span className={`${className} text-muted-foreground`}>{body}</span>;
	}
	return (
		<Link href={route} className={`${className} hover:bg-muted`}>
			{body}
		</Link>
	);
}

export function MailBubble({ mail }: Readonly<{ mail: IBotMail }>) {
	const { t } = useTranslation("bots");
	const outgoing = mail.direction === "out";
	return (
		<article
			data-slot="bot-mail-item"
			data-direction={mail.direction}
			className={`flex flex-col gap-2 rounded-xl border p-3 ${outgoing ? "bg-muted/40" : "bg-background"}`}
		>
			<header className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-xs text-muted-foreground">
				<Badge variant={outgoing ? "secondary" : "outline"}>
					{outgoing
						? t("panel.mail.sent", "Sent")
						: t("panel.mail.received", "Received")}
				</Badge>
				<span className="min-w-0 truncate">
					{addressList(mail.from)}
					{" -> "}
					{addressList(mail.to)}
				</span>
				<time className="ml-auto" dateTime={mail.at}>
					{formatRelativeTime(new Date(mail.at))}
				</time>
			</header>
			{!outgoing && !mail.authenticated && (
				<p className="flex items-center gap-1.5 rounded-md bg-destructive/10 px-2 py-1 text-xs text-destructive">
					<ShieldAlertIcon className="size-3.5" aria-hidden />
					{t(
						"panel.mail.unverified",
						"The sender could not be verified. It may be spoofed.",
					)}
				</p>
			)}
			<p className="text-sm font-medium">{mail.subject}</p>
			<p className="text-sm whitespace-pre-wrap break-words">{mail.text}</p>
			{mail.attachments?.length > 0 && (
				<div className="flex flex-wrap gap-1.5">
					{mail.attachments.map((attachment) => (
						<AttachmentChip
							key={`${attachment.name}-${attachment.handle?.sha256 ?? ""}`}
							attachment={attachment}
						/>
					))}
				</div>
			)}
		</article>
	);
}

function ThreadView({
	bot,
	thread,
	onBack,
}: Readonly<{ bot: IBot; thread: IBotMailThread; onBack: () => void }>) {
	const { t } = useTranslation("bots");
	const view = useBotMailThread(bot.id, thread.id);
	const inbox = useBackend().inboxState;
	const queryClient = useQueryClient();
	useEffect(() => {
		if (thread.unread === 0) return;
		void inbox
			?.markRead(thread.id)
			.then(() =>
				queryClient.invalidateQueries({
					queryKey: botKeys.mailThreads(bot.id),
				}),
			)
			.catch(() => undefined);
	}, [bot.id, inbox, queryClient, thread.id, thread.unread]);
	return (
		<div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-4 pb-6">
			<div className="flex items-center gap-2">
				<Button
					type="button"
					variant="ghost"
					size="icon"
					className="size-8 shrink-0"
					onClick={onBack}
					aria-label={t("panel.mail.back", "Back to threads")}
				>
					<ArrowLeftIcon aria-hidden />
				</Button>
				<div className="min-w-0">
					<h3 className="truncate text-sm font-medium">{thread.subject}</h3>
					<p className="truncate text-xs text-muted-foreground">
						{thread.participants.join(", ")}
					</p>
				</div>
			</div>
			{view.isLoading && (
				<Loader2Icon
					className="mx-auto mt-6 animate-spin text-muted-foreground"
					aria-hidden
				/>
			)}
			{view.data?.mails.map((mail) => (
				<MailBubble key={mail.id} mail={mail} />
			))}
		</div>
	);
}

export function ThreadRow({
	thread,
	onOpen,
}: Readonly<{ thread: IBotMailThread; onOpen: () => void }>) {
	return (
		<button
			type="button"
			data-slot="bot-mail-thread"
			onClick={onOpen}
			className="flex w-full flex-col gap-0.5 rounded-lg px-3 py-2 text-left hover:bg-muted/60"
		>
			<span className="flex items-center gap-2">
				<span
					className={`min-w-0 flex-1 truncate text-sm ${thread.unread > 0 ? "font-semibold" : "font-medium"}`}
				>
					{thread.subject}
				</span>
				{thread.unread > 0 && (
					<Badge className="shrink-0">{thread.unread}</Badge>
				)}
				<time
					className="shrink-0 text-xs text-muted-foreground"
					dateTime={thread.last_message_at}
				>
					{formatRelativeTime(new Date(thread.last_message_at))}
				</time>
			</span>
			<span className="truncate text-xs text-muted-foreground">
				{thread.participants.join(", ")}
			</span>
			{thread.last_snippet && (
				<span className="truncate text-xs text-muted-foreground">
					{thread.last_snippet}
				</span>
			)}
		</button>
	);
}

/** The Mail tab: every thread of the bot's mail, and a thread's mails in both directions. */
export function BotMailPane({ bot }: Readonly<{ bot: IBot }>) {
	const { t } = useTranslation("bots");
	const threads = useBotMailThreads(bot.id);
	const address = useBotMailAddress(bot.id);
	const [openId, setOpenId] = useState<string | null>(null);
	const open = threads.data?.find((thread) => thread.id === openId);
	if (open) {
		return (
			<ThreadView bot={bot} thread={open} onBack={() => setOpenId(null)} />
		);
	}
	return (
		<div
			data-slot="bot-mail-pane"
			className="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto px-2 pb-6"
		>
			{address.data?.address && (
				<p className="flex items-center gap-1.5 px-3 text-xs text-muted-foreground">
					<MailIcon className="size-3.5" aria-hidden />
					<span className="truncate font-mono">{address.data.address}</span>
				</p>
			)}
			{threads.isLoading && (
				<Loader2Icon
					className="mx-auto mt-6 animate-spin text-muted-foreground"
					aria-hidden
				/>
			)}
			{threads.data?.length === 0 && (
				<p className="px-4 pt-6 text-center text-sm text-muted-foreground">
					{address.data?.address
						? t(
								"panel.mail.empty",
								"No mail yet. Mail sent to the address shows up here.",
							)
						: t(
								"panel.mail.emptyNoAddress",
								"Claim an address in Details to let the bot receive mail.",
							)}
				</p>
			)}
			{threads.data?.map((thread) => (
				<ThreadRow
					key={thread.id}
					thread={thread}
					onOpen={() => setOpenId(thread.id)}
				/>
			))}
		</div>
	);
}
