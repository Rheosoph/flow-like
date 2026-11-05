"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowRight, ArrowUp, MessageSquarePlus } from "lucide-react";
import dynamic from "next/dynamic";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { type ReactNode, useCallback, useMemo, useState } from "react";
import { botPresence, useBotPresence } from "../../../hooks/use-bot-presence";
import { cn } from "../../../lib/utils";
import type { IBot } from "../../../state/backend-state/bot-state";
import type { IConversation } from "../../../state/backend-state/inbox-state";
import { useGlobalChatStore } from "../../../state/global-chat/global-chat-store";
import { useBotStatusText } from "../../bots-tab/bot-labels";
import {
	type CrewData,
	CrewDataScope,
	revealPromptChat,
	useBotIds,
	useCrewPickerSource,
	useNeedsYou,
} from "../../bots-tab/crew-data";
import {
	CREW_GRID_CLASS,
	CrewBotCard,
	CrewGridSkeleton,
	type CrewMember,
	NewBotTile,
	useCrewMembers,
} from "../../bots-tab/crew-grid";
import {
	crewPrincipalIds,
	hasBotsHostedElsewhere,
	splitConversations,
} from "../../bots-tab/crew-model";
import { CrewSection } from "../../bots-tab/crew-section";
import {
	BOTS_ROUTE,
	type DockSelection,
	chatBotIdForSelection,
	dockRoute,
	dockSelectionForBot,
	dockSelectionForChatBot,
} from "../../bots-tab/dock-selection";
import { NeedsYouSection } from "../../bots-tab/needs-you-section";
import { NewMessageDialog } from "../../bots-tab/new-message-dialog";
import { NewRoomDialog } from "../../bots-tab/new-room-dialog";
import {
	ConversationRow,
	type RoomCardContext,
	RoomsSection,
	type RoomsStatus,
	RoomsStatusLine,
} from "../../bots-tab/rooms-section";
import { useConversationText } from "../../bots-tab/use-conversation-text";
import { useCrewDirectory } from "../../bots-tab/use-crew-directory";
import { BotAvatar } from "../../bots/bot-avatar";
import { ChatBotAvatar } from "../../global-chat/chat-bot-avatar";
import { Button } from "../../ui/button";
import { Textarea } from "../../ui/textarea";
import {
	type HomeBotMessageFilter,
	type HomeContentProps,
	homeBotMessageFilter,
	numberConfig,
	textConfig,
} from "./config";

const BotEditorDialog = dynamic(
	() =>
		import("../../bots-tab/editor").then((module) => module.BotEditorDialog),
	{ ssr: false },
);

/** The crew member a widget names: a built-in key, a bot id, or the Assistant when unset. */
export function homeCrewMember(
	crew: readonly CrewMember[],
	botId: string,
): CrewMember | undefined {
	const wanted = botId || "general";
	return (
		crew.find(
			(member) =>
				member.key === wanted ||
				member.bot?.id === wanted ||
				(member.system === wanted && !member.bot?.local),
		) ??
		crew.find((member) => member.system === "general") ??
		crew[0]
	);
}

function useOpenDock() {
	const router = useRouter();
	return useCallback(
		(selection: DockSelection) => router.push(dockRoute(selection)),
		[router],
	);
}

function OpenBotsLink() {
	const { t } = useTranslation("bots");
	return (
		<Button
			asChild
			variant="ghost"
			size="sm"
			className="h-8 rounded-[9px] text-muted-foreground hover:text-foreground"
		>
			<Link href={BOTS_ROUTE}>
				{t("home.openBots", "Open Bots")}
				<ArrowRight aria-hidden />
			</Link>
		</Button>
	);
}

/**
 * The width rules the Bots tab's cards follow. A borderless widget lines its content up with the
 * edges of the cards beside it, a framed one insets it like every other card.
 */
function CrewStage({
	widget,
	children,
}: Readonly<{ widget: HomeContentProps["widget"]; children: ReactNode }>) {
	return (
		<div
			className={cn(
				"@container/crew flex min-w-0 flex-col gap-6",
				widget.appearance.variant === "borderless" ? "px-1 py-2" : "p-5",
			)}
		>
			{children}
		</div>
	);
}

/** A Bots tab section on Home: the tab's own heading over its own cards. */
function HomeCrewSection({
	widget,
	title,
	aside,
	actions,
	children,
}: Readonly<{
	widget: HomeContentProps["widget"];
	title: ReactNode;
	aside?: ReactNode;
	actions?: ReactNode;
	children: ReactNode;
}>) {
	return (
		<CrewStage widget={widget}>
			<CrewSection title={title} aside={aside} actions={actions}>
				{children}
			</CrewSection>
		</CrewStage>
	);
}

function HomeCrewNotice({ children }: Readonly<{ children: ReactNode }>) {
	return (
		<p className="rounded-2xl border border-dashed px-4 py-5 text-center text-sm text-muted-foreground">
			{children}
		</p>
	);
}

function BotChips({
	crew,
	selectedKey,
	onSelect,
	disabled,
}: Readonly<{
	crew: readonly CrewMember[];
	selectedKey?: string;
	onSelect: (member: CrewMember) => void;
	disabled: boolean;
}>) {
	const { t } = useTranslation("bots");
	return (
		<fieldset className="flex min-w-0 flex-wrap items-center gap-1.5">
			<legend className="sr-only">
				{t("home.chat.pickBot", "Who to talk to")}
			</legend>
			{crew.map((member) => {
				const selected = member.key === selectedKey;
				return (
					<button
						key={member.key}
						type="button"
						aria-pressed={selected}
						disabled={disabled}
						onClick={() => onSelect(member)}
						className={cn(
							"inline-flex h-8 max-w-44 items-center gap-1.5 rounded-full border py-0.5 pr-3 pl-1 text-xs font-medium outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60",
							selected
								? "border-[color-mix(in_srgb,var(--home-accent)_45%,transparent)] bg-[color-mix(in_srgb,var(--home-accent)_12%,transparent)] text-foreground"
								: "border-border/60 bg-background/40 text-muted-foreground hover:bg-muted/60 hover:text-foreground",
						)}
					>
						<span className="flex size-6 shrink-0 items-end justify-center overflow-hidden rounded-full bg-muted">
							<BotAvatar
								appearance={member.appearance}
								size={24}
								animated={false}
								decorative
							/>
						</span>
						<span className="truncate">{member.name}</span>
					</button>
				);
			})}
		</fieldset>
	);
}

/** Talk to any bot from Home: the message goes to the bot's chat in the Bots tab and is sent there. */
function BotChat({
	data,
	widget,
	editing,
}: Readonly<{ data: CrewData } & HomeContentProps>) {
	const { t } = useTranslation("bots");
	const openDock = useOpenDock();
	const setDraft = useGlobalChatStore((state) => state.setDraft);
	const statusText = useBotStatusText();
	const crew = useCrewMembers(data.bots);
	const hero = textConfig(widget.config, "mode", "hero") !== "bar";
	const showCrew = widget.config.crew !== false;
	const [pickedKey, setPickedKey] = useState<string | null>(null);
	const [value, setValue] = useState("");
	const member =
		crew.find((item) => item.key === pickedKey) ??
		homeCrewMember(crew, textConfig(widget.config, "botId"));
	const status = member ? statusText(member.system, member.location) : null;
	const waiting = data.botsLoading;
	const canSend = !editing && !waiting && Boolean(member) && !!value.trim();
	const name = member?.name ?? "";
	const placeholder =
		textConfig(widget.config, "placeholder") ||
		t("home.chat.placeholder", "Message {{name}}…", { name });
	const chips = useMemo(
		() => crew.slice(0, numberConfig(widget.config, "limit", 6)),
		[crew, widget.config],
	);

	const send = () => {
		const prompt = value.trim();
		if (!canSend || !member) return;
		const botId = chatBotIdForSelection(member.selection, data.bots);
		if (botId === undefined) return;
		setDraft({ prompt, botId });
		openDock(member.selection);
	};

	const composer = (
		<div
			className={cn(
				"flex min-w-0 gap-3 rounded-2xl border border-border/70 bg-background/60 p-2.5 transition-shadow focus-within:border-[color-mix(in_srgb,var(--home-accent)_45%,transparent)] focus-within:ring-2 focus-within:ring-[color-mix(in_srgb,var(--home-accent)_12%,transparent)]",
				hero ? "items-end" : "items-center",
			)}
		>
			{!hero && member && (
				<span className="flex size-9 shrink-0 items-end justify-center overflow-hidden rounded-xl bg-[color-mix(in_srgb,var(--home-accent)_10%,transparent)]">
					<ChatBotAvatar
						draft={value}
						appearance={member.appearance}
						size={36}
					/>
				</span>
			)}
			{hero ? (
				<Textarea
					value={value}
					onChange={(event) => setValue(event.target.value)}
					placeholder={placeholder}
					aria-label={t("home.chat.message", "Message {{name}}", { name })}
					rows={2}
					className="max-h-40 min-h-14 min-w-0 resize-none border-0 bg-transparent p-1.5 text-sm shadow-none focus-visible:ring-0 dark:bg-transparent"
					onKeyDown={(event) => {
						if (event.key === "Enter" && !event.shiftKey) {
							event.preventDefault();
							send();
						}
					}}
				/>
			) : (
				<input
					value={value}
					onChange={(event) => setValue(event.target.value)}
					placeholder={placeholder}
					aria-label={t("home.chat.message", "Message {{name}}", { name })}
					className="h-9 min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground sm:text-base"
				/>
			)}
			<Button
				type="submit"
				size="icon"
				className="size-9 shrink-0 rounded-xl bg-[var(--home-accent)] text-[var(--home-accent-foreground)] hover:bg-[var(--home-accent)] hover:brightness-95"
				disabled={!canSend}
				aria-label={t("home.chat.send", "Send to {{name}}", { name })}
			>
				<ArrowUp className="size-4" />
			</Button>
		</div>
	);

	return (
		<form
			data-home-bot-chat={hero ? "hero" : "bar"}
			onSubmit={(event) => {
				event.preventDefault();
				send();
			}}
			className={cn(
				"relative flex min-w-0 flex-col",
				hero
					? "flex-1 justify-center gap-4 overflow-hidden bg-gradient-to-br from-[color-mix(in_srgb,var(--home-accent)_9%,transparent)] via-transparent to-[color-mix(in_srgb,var(--home-accent)_4%,transparent)] p-5 sm:p-6"
					: "gap-2 px-1 py-1",
			)}
		>
			{hero && member && (
				<div className="flex min-w-0 items-center gap-5">
					<span className="relative flex size-24 shrink-0 items-end justify-center">
						<span
							aria-hidden
							className="absolute inset-2 rounded-full bg-[color-mix(in_srgb,var(--home-accent)_22%,transparent)] blur-2xl"
						/>
						<ChatBotAvatar
							draft={value}
							appearance={member.appearance}
							size={96}
							className="relative"
						/>
					</span>
					<div className="min-w-0">
						<p className="mb-1 flex min-w-0 items-center gap-1.5 text-[10px] font-medium uppercase tracking-[0.16em] text-muted-foreground">
							<span className="truncate">{name}</span>
							{status && (
								<>
									<span aria-hidden>·</span>
									<span className="truncate normal-case tracking-normal">
										{status.label}
									</span>
								</>
							)}
						</p>
						<h2 className="text-2xl font-semibold tracking-tight">
							{widget.title ||
								t("home.chat.heading", "What should we work on?")}
						</h2>
						{member.description && (
							<p className="mt-1 line-clamp-2 max-w-2xl text-xs leading-relaxed text-muted-foreground">
								{member.description}
							</p>
						)}
					</div>
				</div>
			)}
			{composer}
			{showCrew && chips.length > 1 && (
				<BotChips
					crew={chips}
					selectedKey={member?.key}
					onSelect={(next) => setPickedKey(next.key)}
					disabled={editing === true}
				/>
			)}
		</form>
	);
}

/** Your bots as the Bots tab shows them; a card opens its chat, the tile creates one. */
function BotCrew({
	data,
	widget,
}: Readonly<{ data: CrewData } & HomeContentProps>) {
	const { t } = useTranslation("bots");
	const openDock = useOpenDock();
	const crew = useCrewMembers(data.bots);
	const presence = useBotPresence(hasBotsHostedElsewhere(data.bots)).data;
	const [creating, setCreating] = useState(false);
	const canCreate = widget.config.newBot !== false && data.mode !== "guest";
	const shown = crew.slice(0, numberConfig(widget.config, "limit", 8));
	const openSaved = useCallback(
		(bot: IBot) => openDock(dockSelectionForBot(bot, data.bots)),
		[data.bots, openDock],
	);
	return (
		<HomeCrewSection
			widget={widget}
			title={widget.title || t("crew.yourCrew", "Your crew")}
			aside={
				data.bots && (
					<span className="text-[13px] font-normal text-muted-foreground">
						{t("crew.summary.bots", "{{count}} bots", {
							count: crew.length,
							defaultValue_one: "{{count}} bot",
						})}
					</span>
				)
			}
			actions={<OpenBotsLink />}
		>
			{data.botsFailed && (
				<HomeCrewNotice>
					{t(
						"crew.loadFailedFlux",
						"Couldn't load your bots. Flux still works.",
					)}
				</HomeCrewNotice>
			)}
			<div className={CREW_GRID_CLASS}>
				{data.botsLoading ? (
					<CrewGridSkeleton />
				) : (
					shown.map((member) => (
						<CrewBotCard
							key={member.key}
							member={member}
							presence={botPresence(member.bot, presence)}
							selected={false}
							onSelect={(picked) => openDock(picked.selection)}
						/>
					))
				)}
				{canCreate && (
					<NewBotTile disabled={!data.bots} onClick={() => setCreating(true)} />
				)}
			</div>
			{canCreate && creating && (
				<BotEditorDialog
					open={creating}
					onOpenChange={setCreating}
					botId={null}
					onSaved={openSaved}
					onDuplicated={openSaved}
				/>
			)}
		</HomeCrewSection>
	);
}

function conversationsFor(
	filter: HomeBotMessageFilter,
	conversations: readonly IConversation[],
): IConversation[] {
	const { rooms, direct } = splitConversations(conversations);
	if (filter === "rooms") return rooms;
	if (filter === "direct") return direct;
	if (filter === "unread")
		return conversations.filter((item) => item.unread_count > 0);
	return [...conversations];
}

/** Current messages, rooms or what needs you, with the Bots tab's own cards and pickers. */
function BotMessages({
	data,
	widget,
	editing,
}: Readonly<{ data: CrewData } & HomeContentProps>) {
	const { t } = useTranslation("bots");
	const openDock = useOpenDock();
	const { me, bots, conversations } = data;
	const filter = homeBotMessageFilter(widget.config);
	const limit = numberConfig(widget.config, "limit", 6);
	const showNeeds =
		filter === "needs" ||
		(filter !== "rooms" && widget.config.needsYou !== false);
	const myBotIds = useBotIds(bots);
	const { items: needs, invitations } = useNeedsYou(conversations, {
		me,
		myBotIds,
		openConversationId: null,
		showingGlobalChat: false,
	});
	const principalIds = useMemo(
		() => crewPrincipalIds(conversations, invitations),
		[conversations, invitations],
	);
	const directory = useCrewDirectory(principalIds, { me, bots });
	const text = useConversationText(directory, me);
	const pickerSource = useCrewPickerSource(data, directory);
	const [dialog, setDialog] = useState<"room" | "message" | null>(null);

	const openConversation = useCallback(
		(conversationId: string) =>
			openDock({ type: "conversation", conversationId }),
		[openDock],
	);
	const openPromptChat = useCallback(
		() => revealPromptChat((botId) => openDock(dockSelectionForChatBot(botId))),
		[openDock],
	);
	const openPicked = useCallback(
		({ bot, conversationId }: { bot?: IBot; conversationId?: string }) => {
			if (bot) openDock(dockSelectionForBot(bot, bots));
			else if (conversationId) openConversation(conversationId);
		},
		[bots, openConversation, openDock],
	);
	const context: RoomCardContext = {
		me,
		text,
		directory,
		onOpen: openConversation,
	};
	const status: RoomsStatus = {
		searching: false,
		loading: data.conversationsLoading,
		failed: data.conversationsFailed,
		onRetry: data.retryConversations,
		emptyText:
			filter === "rooms"
				? t(
						"home.rooms.empty",
						"No rooms yet. Bring people and bots together in one.",
					)
				: filter === "unread"
					? t("home.messages.caughtUp", "You're all caught up.")
					: undefined,
	};

	const needIds = new Set(
		needs.flatMap((item) =>
			item.type === "conversation" ? [item.conversation.id] : [],
		),
	);
	const listed = conversationsFor(filter, conversations)
		.filter((item) => !showNeeds || !needIds.has(item.id))
		.slice(0, limit);
	const title =
		widget.title ||
		(filter === "rooms"
			? t("home.rooms.title", "Rooms")
			: filter === "needs"
				? t("crew.needs.title", "Needs you")
				: t("home.messages.title", "Messages"));

	if (data.mode === "guest") {
		// One sign-in hint per Home: rooms and requests stay out of the way until Customize.
		if ((filter === "rooms" || filter === "needs") && !editing) return null;
		return (
			<HomeCrewSection widget={widget} title={title} actions={<OpenBotsLink />}>
				<HomeCrewNotice>
					{t(
						"home.messages.signIn",
						"Sign in to message people and share rooms with your bots.",
					)}
				</HomeCrewNotice>
			</HomeCrewSection>
		);
	}

	const needsSection = showNeeds && (
		<NeedsYouSection
			title={filter === "needs" ? title : undefined}
			items={needs}
			directory={directory}
			text={text}
			onOpenConversation={openConversation}
			onOpenAssistant={openPromptChat}
		/>
	);

	const dialogs = (
		<>
			<NewRoomDialog
				open={dialog === "room"}
				onOpenChange={(open) => setDialog(open ? "room" : null)}
				source={pickerSource}
				onCreated={openConversation}
			/>
			<NewMessageDialog
				open={dialog === "message"}
				onOpenChange={(open) => setDialog(open ? "message" : null)}
				source={pickerSource}
				onPicked={openPicked}
			/>
		</>
	);

	if (filter === "rooms") {
		return (
			<CrewStage widget={widget}>
				<RoomsSection
					title={title}
					rooms={listed}
					direct={[]}
					context={context}
					status={status}
					onNewRoom={() => setDialog("room")}
					onNewMessage={() => setDialog("message")}
				/>
				{dialogs}
			</CrewStage>
		);
	}

	if (filter === "needs") {
		if (needs.length > 0)
			return <CrewStage widget={widget}>{needsSection}</CrewStage>;
		// Nothing waits: the strip steps aside, except while Customize shows where it sits.
		if (!editing) return null;
		return (
			<CrewStage widget={widget}>
				<CrewSection
					title={title}
					aside={
						<span className="text-[13px] font-normal text-muted-foreground">
							{t("home.needs.empty", "Nothing needs you right now.")}
						</span>
					}
				>
					{null}
				</CrewSection>
			</CrewStage>
		);
	}

	return (
		<HomeCrewSection
			widget={widget}
			title={title}
			actions={
				<Button
					type="button"
					variant="outline"
					size="sm"
					className="rounded-[9px]"
					onClick={() => setDialog("message")}
				>
					<MessageSquarePlus aria-hidden />
					{t("crew.newMessage", "New message")}
				</Button>
			}
		>
			{needsSection}
			<RoomsStatusLine status={status} empty={listed.length === 0} />
			{listed.length > 0 && (
				<div className="flex min-w-0 flex-col gap-1.5">
					{listed.map((conversation) => (
						<ConversationRow
							key={conversation.id}
							conversation={conversation}
							context={context}
						/>
					))}
				</div>
			)}
			{dialogs}
		</HomeCrewSection>
	);
}

/** The value a widget stores for a bot: a built-in's key travels between accounts, a bot's id doesn't. */
function homeBotValue(member: CrewMember): string {
	return member.system && !member.bot?.local ? member.system : member.key;
}

function BotSelect({
	data,
	id,
	value,
	onChange,
	className,
}: Readonly<{
	data: CrewData;
	id: string;
	value: string;
	onChange: (value: string) => void;
	className?: string;
}>) {
	const crew = useCrewMembers(data.bots);
	const selected = homeCrewMember(crew, value);
	const current = selected ? homeBotValue(selected) : value;
	return (
		<select
			id={id}
			value={current}
			onChange={(event) => onChange(event.target.value)}
			className={className}
		>
			{crew.map((member) => (
				<option key={member.key} value={homeBotValue(member)}>
					{member.name}
				</option>
			))}
		</select>
	);
}

/** A select of the reader's bots for widget settings; empty means the Assistant. */
export function HomeBotPicker(
	props: Readonly<{
		id: string;
		value: string;
		onChange: (value: string) => void;
		className?: string;
	}>,
) {
	return (
		<CrewDataScope>
			{(data) => <BotSelect data={data} {...props} />}
		</CrewDataScope>
	);
}

export function HomeBotChat(props: HomeContentProps) {
	return (
		<CrewDataScope>
			{(data) => <BotChat data={data} {...props} />}
		</CrewDataScope>
	);
}

export function HomeBotCrew(props: HomeContentProps) {
	return (
		<CrewDataScope>
			{(data) => <BotCrew data={data} {...props} />}
		</CrewDataScope>
	);
}

export function HomeBotMessages(props: HomeContentProps) {
	return (
		<CrewDataScope>
			{(data) => <BotMessages data={data} {...props} />}
		</CrewDataScope>
	);
}
