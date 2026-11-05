"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AlarmClockIcon,
	CheckIcon,
	CopyIcon,
	ExternalLinkIcon,
	FileIcon,
	type LucideIcon,
	MailIcon,
	ShieldAlertIcon,
	ShieldCheckIcon,
	WebhookIcon,
	WorkflowIcon,
} from "lucide-react";
import {
	Fragment,
	type ReactNode,
	useCallback,
	useMemo,
	useState,
} from "react";
import { useUserIdentity } from "../../hooks/use-user-lookup";
import { useClientRouter } from "../../lib/client-navigation";
import { humanFileSize } from "../../lib/utils";
import { IdentityAvatar } from "../identity/identity-avatar";
import { useCopy } from "../settings/devices/primitives/use-copy";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import {
	HoverCard,
	HoverCardContent,
	HoverCardTrigger,
} from "../ui/hover-card";
import { BOTS_ROUTE, dockSelectionQuery } from "./dock-selection";
import type {
	ConversationBotAsk,
	ConversationNote,
	ConversationTrigger,
	TriggerSource,
} from "./fold-entries";
import {
	type MailAddress,
	type MailFacts,
	addressLabel,
	mailFacts,
	payloadText,
	triggerPreview,
	webhookFacts,
} from "./note-facts";

const SOURCE_ICONS: Readonly<Record<TriggerSource, LucideIcon>> = {
	schedule: AlarmClockIcon,
	webhook: WebhookIcon,
	email: MailIcon,
	event: WorkflowIcon,
};

const PILL =
	"inline-flex min-w-0 max-w-full items-center gap-1.5 rounded-full px-2.5 py-1 text-xs text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

function senderOf(address: MailAddress | undefined): string | undefined {
	return address ? (address.name ?? address.email) : undefined;
}

/** One line naming a note, e.g. "Scheduled: Morning brief" or "Scout asked Pebble". */
export function useNoteText() {
	const { t } = useTranslation("bots");
	return useCallback(
		(
			note: ConversationNote,
			nameOf: (id: string) => string,
			viewerBotId?: string,
		): string => {
			if (note.type === "bot_ask") {
				if (viewerBotId === note.askerId) {
					return t("crew.note.messaged", "Messaged {{name}}", {
						name: nameOf(note.calleeId),
					});
				}
				if (viewerBotId === note.calleeId) {
					return t("crew.note.askedBy", "{{name}} asked", {
						name: nameOf(note.askerId),
					});
				}
				return t("crew.note.asked", "{{asker}} asked {{callee}}", {
					asker: nameOf(note.askerId),
					callee: nameOf(note.calleeId),
				});
			}
			const title = note.title || t("crew.note.untitled", "Untitled");
			if (note.source === "email") {
				const from = senderOf(mailFacts(note)?.from[0]);
				return from
					? t("crew.trigger.emailFrom", "Email from {{from}}: {{title}}", {
							from,
							title,
						})
					: t("crew.trigger.email", "Email: {{title}}", { title });
			}
			return {
				schedule: t("crew.trigger.schedule", "Scheduled: {{title}}", { title }),
				webhook: t("crew.trigger.webhook", "Webhook: {{title}}", { title }),
				event: t("crew.trigger.event", "Event: {{title}}", { title }),
			}[note.source];
		},
		[t],
	);
}

function useSourceName() {
	const { t } = useTranslation("bots");
	return useCallback(
		(source: TriggerSource): string =>
			({
				schedule: t("crew.note.source.schedule", "Scheduled task"),
				webhook: t("crew.note.source.webhook", "Webhook"),
				email: t("crew.note.source.email", "Email"),
				event: t("crew.note.source.event", "Event"),
			})[source],
		[t],
	);
}

function useNoteTime(time: number, style: "short" | "long" = "short"): string {
	const { i18n } = useTranslation("bots");
	return useMemo(
		() =>
			new Intl.DateTimeFormat(i18n.language, {
				dateStyle: style === "long" ? "full" : "medium",
				timeStyle: "short",
			}).format(time),
		[i18n.language, time, style],
	);
}

/** Opens a conversation in the Bots tab, from the tab itself or from any chat. */
function useOpenConversation() {
	const router = useClientRouter();
	return useCallback(
		(conversationId: string) => {
			const current =
				window.location.pathname === BOTS_ROUTE ? window.location.search : "";
			const query = dockSelectionQuery(
				{ type: "conversation", conversationId },
				current,
			);
			router.push(`${BOTS_ROUTE}?${query}`);
		},
		[router],
	);
}

/** A centered pill that previews on hover and acts on click. */
function NotePill({
	slot,
	label,
	onSelect,
	card,
	children,
}: Readonly<{
	slot: string;
	label: string;
	onSelect: () => void;
	card: ReactNode;
	children: ReactNode;
}>) {
	return (
		<div data-slot={slot} className="flex w-full justify-center px-3">
			<HoverCard openDelay={250} closeDelay={100}>
				<HoverCardTrigger asChild>
					<button
						type="button"
						aria-label={label}
						onClick={onSelect}
						className={PILL}
					>
						{children}
					</button>
				</HoverCardTrigger>
				<HoverCardContent
					side="top"
					className="w-80 max-w-[calc(100vw-2rem)] p-3"
				>
					{card}
				</HoverCardContent>
			</HoverCard>
		</div>
	);
}

function NoteHeader({
	icon: Icon,
	name,
	time,
}: Readonly<{ icon: LucideIcon; name: string; time: string }>) {
	return (
		<div className="flex items-center gap-2 text-xs text-muted-foreground">
			<Icon className="size-3.5 shrink-0" aria-hidden />
			<span className="font-medium text-foreground">{name}</span>
			<span className="ml-auto shrink-0">{time}</span>
		</div>
	);
}

function DetailSection({
	title,
	action,
	children,
}: Readonly<{ title: string; action?: ReactNode; children: ReactNode }>) {
	return (
		<section className="flex flex-col gap-1.5">
			<div className="flex min-h-7 items-center justify-between gap-2">
				<h4 className="text-xs font-medium text-muted-foreground">{title}</h4>
				{action}
			</div>
			{children}
		</section>
	);
}

function PayloadSection({
	title,
	value,
}: Readonly<{ title: string; value: unknown }>) {
	const { t } = useTranslation("bots");
	const { copied, copy } = useCopy();
	const text = useMemo(() => payloadText(value), [value]);
	return (
		<DetailSection
			title={title}
			action={
				<Button
					type="button"
					variant="ghost"
					size="sm"
					className="h-7 gap-1 px-2 text-xs"
					onClick={() => void copy(text)}
				>
					{copied ? (
						<CheckIcon className="size-3.5" aria-hidden />
					) : (
						<CopyIcon className="size-3.5" aria-hidden />
					)}
					{copied
						? t("crew.note.copied", "Copied")
						: t("crew.note.copy", "Copy")}
				</Button>
			}
		>
			<pre className="max-h-72 overflow-auto rounded-md bg-muted p-3 font-mono text-xs leading-relaxed whitespace-pre-wrap break-words">
				{text}
			</pre>
		</DetailSection>
	);
}

function MailSection({ mail }: Readonly<{ mail: MailFacts }>) {
	const { t } = useTranslation("bots");
	const rows = [
		{ term: t("crew.note.mail.from", "From"), list: mail.from },
		{ term: t("crew.note.mail.to", "To"), list: mail.to },
		{ term: t("crew.note.mail.cc", "Cc"), list: mail.cc },
	].filter((row) => row.list.length > 0);
	const ShieldIcon = mail.authenticated ? ShieldCheckIcon : ShieldAlertIcon;
	return (
		<div className="flex flex-col gap-4">
			<dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-sm">
				{rows.map((row) => (
					<Fragment key={row.term}>
						<dt className="text-muted-foreground">{row.term}</dt>
						<dd className="min-w-0 break-words">
							{row.list.map(addressLabel).join(", ")}
						</dd>
					</Fragment>
				))}
			</dl>
			<p className="flex items-center gap-1.5 text-xs text-muted-foreground">
				<ShieldIcon className="size-3.5 shrink-0" aria-hidden />
				{mail.authenticated
					? t("crew.note.mail.verified", "The sender's domain is verified")
					: t(
							"crew.note.mail.unverified",
							"The sender's domain is not verified; the mail may not come from who it claims",
						)}
			</p>
			{mail.body.trim() && (
				<DetailSection title={t("crew.note.mail.body", "Message")}>
					<p className="whitespace-pre-wrap break-words text-sm">{mail.body}</p>
				</DetailSection>
			)}
			{mail.attachments.length > 0 && (
				<DetailSection title={t("crew.note.mail.attachments", "Attachments")}>
					<ul className="flex flex-col gap-1 text-sm">
						{mail.attachments.map((file) => (
							<li key={file.name} className="flex min-w-0 items-center gap-1.5">
								<FileIcon
									className="size-3.5 shrink-0 text-muted-foreground"
									aria-hidden
								/>
								<span className="truncate">{file.name}</span>
								{file.size !== undefined && (
									<span className="shrink-0 text-xs text-muted-foreground">
										{humanFileSize(file.size)}
									</span>
								)}
								{file.skipped && (
									<span className="shrink-0 text-xs text-muted-foreground">
										{t("crew.note.mail.notKept", "not kept: {{reason}}", {
											reason: file.skipped,
										})}
									</span>
								)}
							</li>
						))}
					</ul>
				</DetailSection>
			)}
		</div>
	);
}

function TriggerSummary({
	trigger,
}: Readonly<{ trigger: ConversationTrigger }>) {
	const { t } = useTranslation("bots");
	const sourceName = useSourceName()(trigger.source);
	const time = useNoteTime(trigger.time);
	const mail = mailFacts(trigger);
	const sender = mail?.from[0];
	const heading = mail?.subject || trigger.title;
	const preview = triggerPreview(trigger).trim();
	const monospace = webhookFacts(trigger) !== null;
	return (
		<div className="flex flex-col gap-2 text-sm">
			<NoteHeader
				icon={SOURCE_ICONS[trigger.source]}
				name={sourceName}
				time={time}
			/>
			{heading && <p className="font-medium leading-snug">{heading}</p>}
			{sender && (
				<p className="truncate text-xs text-muted-foreground">
					{t("crew.note.from", "From {{sender}}", {
						sender: addressLabel(sender),
					})}
				</p>
			)}
			{preview && (
				<p
					className={`line-clamp-4 whitespace-pre-wrap break-words text-xs text-muted-foreground${monospace ? " font-mono" : ""}`}
				>
					{preview}
				</p>
			)}
			<p className="text-[11px] text-muted-foreground">
				{t("crew.note.clickForDetails", "Click for details")}
			</p>
		</div>
	);
}

function TriggerDetails({
	trigger,
	label,
	open,
	onOpenChange,
}: Readonly<{
	trigger: ConversationTrigger;
	label: string;
	open: boolean;
	onOpenChange: (open: boolean) => void;
}>) {
	const { t } = useTranslation("bots");
	const sourceName = useSourceName()(trigger.source);
	const when = useNoteTime(trigger.time, "long");
	const mail = mailFacts(trigger);
	const hook = webhookFacts(trigger);
	const Icon = SOURCE_ICONS[trigger.source];
	const otherPayload = !mail && !hook && trigger.payload !== undefined;
	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="sm:max-w-xl" data-slot="trigger-details">
				<DialogHeader>
					<DialogTitle className="flex min-w-0 items-center gap-2">
						<Icon className="size-4 shrink-0" aria-hidden />
						<span className="truncate">{label}</span>
					</DialogTitle>
					<DialogDescription>{`${sourceName} · ${when}`}</DialogDescription>
				</DialogHeader>
				<div className="flex max-h-[65vh] flex-col gap-4 overflow-y-auto">
					{mail && <MailSection mail={mail} />}
					{!mail && trigger.text.trim() && (
						<DetailSection
							title={t("crew.note.instruction", "What the bot was asked")}
						>
							<p className="whitespace-pre-wrap break-words text-sm">
								{trigger.text}
							</p>
						</DetailSection>
					)}
					{hook && (
						<PayloadSection
							title={
								hook.contentType
									? t("crew.note.bodyOfType", "Request body ({{type}})", {
											type: hook.contentType,
										})
									: t("crew.note.body", "Request body")
							}
							value={hook.body}
						/>
					)}
					{otherPayload && (
						<PayloadSection
							title={t("crew.note.payload", "Payload")}
							value={trigger.payload}
						/>
					)}
					{trigger.payloadFile && (
						<DetailSection title={t("crew.note.payloadFile", "Payload file")}>
							<p className="flex min-w-0 items-center gap-1.5 text-sm">
								<FileIcon
									className="size-3.5 shrink-0 text-muted-foreground"
									aria-hidden
								/>
								<span className="truncate font-mono text-xs">
									{trigger.payloadFile.path}
								</span>
								<span className="shrink-0 text-xs text-muted-foreground">
									{humanFileSize(trigger.payloadFile.bytes)}
								</span>
							</p>
						</DetailSection>
					)}
				</div>
			</DialogContent>
		</Dialog>
	);
}

/** "Scheduled: Morning brief" between messages; hover previews it, click shows everything. */
export function TriggerPill({
	trigger,
}: Readonly<{ trigger: ConversationTrigger }>) {
	const noteText = useNoteText();
	const [open, setOpen] = useState(false);
	const label = noteText(trigger, (id) => id);
	const Icon = SOURCE_ICONS[trigger.source];
	return (
		<>
			<NotePill
				slot="trigger-pill"
				label={label}
				onSelect={() => setOpen(true)}
				card={<TriggerSummary trigger={trigger} />}
			>
				<Icon className="size-3.5 shrink-0" aria-hidden />
				<span className="truncate">{label}</span>
			</NotePill>
			<TriggerDetails
				trigger={trigger}
				label={label}
				open={open}
				onOpenChange={setOpen}
			/>
		</>
	);
}

function BotAskLabel({
	ask,
	viewerBotId,
	asker,
	callee,
}: Readonly<{
	ask: ConversationBotAsk;
	viewerBotId?: string;
	asker: string;
	callee: string;
}>) {
	const { t } = useTranslation("bots");
	if (viewerBotId === ask.askerId) {
		return (
			<>
				<span className="shrink-0">
					{t("crew.note.messagedPrefix", "Messaged")}
				</span>
				<IdentityAvatar subject={ask.calleeId} size={16} />
				<span className="truncate">{callee}</span>
			</>
		);
	}
	return (
		<>
			<IdentityAvatar subject={ask.askerId} size={16} />
			<span className="truncate">
				{viewerBotId === ask.calleeId
					? t("crew.note.askedBy", "{{name}} asked", { name: asker })
					: t("crew.note.asked", "{{asker}} asked {{callee}}", {
							asker,
							callee,
						})}
			</span>
		</>
	);
}

/** "Messaged Pebble" or "Scout asked" between messages; click opens the exchange. */
export function BotAskPill({
	ask,
	viewerBotId,
}: Readonly<{ ask: ConversationBotAsk; viewerBotId?: string }>) {
	const { t } = useTranslation("bots");
	const noteText = useNoteText();
	const openConversation = useOpenConversation();
	const asker = useUserIdentity(ask.askerId).label;
	const callee = useUserIdentity(ask.calleeId).label;
	const names = useCallback(
		(id: string) => (id === ask.askerId ? asker : callee),
		[ask.askerId, asker, callee],
	);
	const label = noteText(ask, names, viewerBotId);
	const time = useNoteTime(ask.time);
	return (
		<NotePill
			slot="bot-ask-pill"
			label={label}
			onSelect={() => openConversation(ask.exchangeId)}
			card={
				<div className="flex flex-col gap-2 text-sm">
					<div className="flex items-center gap-2 text-xs text-muted-foreground">
						<IdentityAvatar subject={ask.askerId} size={16} />
						<span className="min-w-0 truncate font-medium text-foreground">
							{t("crew.note.asked", "{{asker}} asked {{callee}}", {
								asker,
								callee,
							})}
						</span>
						<span className="ml-auto shrink-0">{time}</span>
					</div>
					{ask.question && (
						<p className="line-clamp-6 whitespace-pre-wrap break-words text-xs text-muted-foreground">
							{ask.question}
						</p>
					)}
					<p className="flex items-center gap-1 text-[11px] text-muted-foreground">
						<ExternalLinkIcon className="size-3" aria-hidden />
						{t("exchange.open", "Open the full exchange")}
					</p>
				</div>
			}
		>
			<BotAskLabel
				ask={ask}
				viewerBotId={viewerBotId}
				asker={asker}
				callee={callee}
			/>
		</NotePill>
	);
}

/** A trigger or bot-ask line; `viewerBotId` is the bot whose chat shows it, if any. */
export function ConversationNotePill({
	note,
	viewerBotId,
}: Readonly<{ note: ConversationNote; viewerBotId?: string }>) {
	return note.type === "trigger" ? (
		<TriggerPill trigger={note} />
	) : (
		<BotAskPill ask={note} viewerBotId={viewerBotId} />
	);
}
