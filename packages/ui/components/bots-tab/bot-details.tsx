"use client";

import { useTranslation } from "@flow-like/locales";
import {
	BrainIcon,
	CloudUploadIcon,
	CopyIcon,
	CpuIcon,
	HardDriveIcon,
	Loader2Icon,
	LockIcon,
	type LucideIcon,
	PencilIcon,
	RadioIcon,
	ShieldQuestionIcon,
	XIcon,
} from "lucide-react";
import { type ReactNode, useState } from "react";
import { botPresence, useBotPresence } from "../../hooks/use-bot-presence";
import { useRunningBotTurn } from "../../hooks/use-bot-turn";
import { formatRelativeTime } from "../../lib/date";
import {
	FLOWPILOT_CAPABILITY,
	type IBot,
} from "../../state/backend-state/bot-state";
import type { GlobalChatBotView } from "../global-chat/use-global-chat-bot";
import { Button } from "../ui/button";
import { BotEncryption, offersEncryption } from "./bot-encryption";
import {
	BOT_LOCATION_ICONS,
	useBotCapabilityLabel,
	useBotLocationText,
	useBotPresenceText,
} from "./bot-labels";
import { BotEmailAddress, offersEmail } from "./bot-mail";
import { BotNotifications } from "./bot-notifications";
import {
	subjectCapabilities,
	subjectRemembers,
	subjectSystem,
} from "./bot-panel-tabs";
import { BotExchanges, BotPermissions } from "./bot-permissions";
import { BotTasks, offersTasks } from "./bot-tasks";
import { BotToolGrants } from "./bot-tool-grants";
import { BotWebhooks, offersWebhooks } from "./bot-webhooks";
import { BOT_CAPABILITY_ICONS } from "./editor/bot-capability-groups";
import { hasFixedLocation, hasFlowPilot } from "./editor/bot-draft";
import { BotTag } from "./editor/bot-editor-parts";
import {
	useBotEmbeddingModelOptions,
	useBotModelOptions,
} from "./editor/steps/use-bot-models";
import { useDuplicateBot } from "./editor/use-bot-editor-submit";

/** What the Details tab can start; each is offered only where it applies. */
export interface BotPanelActions {
	/** Opens the bot's settings. */
	onEdit?: (bot: IBot) => void;
	/** Gets the copy Duplicate made. */
	onDuplicated?: (copy: IBot) => void;
	/** Set while bots on this device can move to the signed-in account. */
	onMove?: (bot: IBot) => void;
	/** Opens a conversation in the chat dock. */
	onOpenConversation?: (conversationId: string) => void;
}

function DetailRow({
	icon: Icon,
	label,
	children,
	hint,
}: Readonly<{
	icon: LucideIcon;
	label: string;
	children: ReactNode;
	hint?: string;
}>) {
	return (
		<div className="flex items-start gap-3">
			<span className="grid size-8 shrink-0 place-items-center rounded-lg bg-muted text-muted-foreground">
				<Icon className="size-4" aria-hidden />
			</span>
			<div className="min-w-0 flex-1">
				<dt className="text-xs text-muted-foreground">{label}</dt>
				<dd className="text-sm font-medium wrap-break-word">{children}</dd>
				{hint && (
					<dd className="mt-0.5 text-xs leading-relaxed text-muted-foreground">
						{hint}
					</dd>
				)}
			</div>
		</div>
	);
}

function WhereItRuns({ bot }: Readonly<{ bot: IBot }>) {
	const { t } = useTranslation("bots");
	const locationText = useBotLocationText();
	const label = t("panel.details.runs", "Where it runs");
	if (bot.local) {
		return (
			<DetailRow
				icon={HardDriveIcon}
				label={label}
				hint={t(
					"local.badgeHint",
					"Lives only on this computer until you move it to an account.",
				)}
			>
				{t("local.badge", "On this device")}
			</DetailRow>
		);
	}
	const location = locationText(bot.location);
	return (
		<DetailRow
			icon={BOT_LOCATION_ICONS[bot.location]}
			label={label}
			hint={location.description}
		>
			{location.label}
		</DetailRow>
	);
}

/** Whether a desktop or device bot's host answers, and when it last did. */
function BotStatus({ bot }: Readonly<{ bot: IBot }>) {
	const { t } = useTranslation("bots");
	const presenceText = useBotPresenceText();
	const hosted = !bot.local && !bot.system && bot.location !== "server";
	const presence = botPresence(bot, useBotPresence(hosted).data);
	if (!hosted || !presence) return null;
	const lastSeen = presence.last_seen_at
		? formatRelativeTime(presence.last_seen_at, "long", "")
		: "";
	return (
		<DetailRow
			icon={RadioIcon}
			label={t("panel.details.status", "Status")}
			hint={
				lastSeen
					? t("presence.lastSeen", "Last seen {{when}}", { when: lastSeen })
					: undefined
			}
		>
			{presenceText(presence.state)}
		</DetailRow>
	);
}

/** The turn the bot works on without a chat window, with the way to stop it. */
function BotWorking({ bot }: Readonly<{ bot: IBot }>) {
	const { t } = useTranslation("bots");
	const { turn, waiting, cancel, cancelling } = useRunningBotTurn(bot.id);
	if (!turn && waiting) {
		return (
			<div
				data-slot="bot-waiting"
				className="flex items-center gap-2.5 rounded-lg bg-muted/60 py-1.5 pr-1.5 pl-3 text-sm"
			>
				<ShieldQuestionIcon
					className="size-4 shrink-0 text-primary"
					aria-hidden
				/>
				<output className="min-w-0 flex-1 font-medium">
					{t("panel.waitingForYou", "Waiting for you")}
				</output>
			</div>
		);
	}
	if (!turn) return null;
	return (
		<div
			data-slot="bot-working"
			className="flex items-center gap-2.5 rounded-lg bg-muted/60 py-1.5 pr-1.5 pl-3 text-sm"
		>
			<Loader2Icon
				className="size-4 shrink-0 animate-spin text-muted-foreground"
				aria-hidden
			/>
			<output className="min-w-0 flex-1 font-medium">
				{t("panel.working", "Working…")}
			</output>
			<Button
				type="button"
				variant="ghost"
				size="sm"
				className="h-7 px-2.5 text-xs"
				disabled={cancelling}
				onClick={() => void cancel()}
			>
				<XIcon aria-hidden />
				{t("panel.cancelTurn", "Cancel")}
			</Button>
		</div>
	);
}

function ModelName({ id }: Readonly<{ id: string }>) {
	const { t } = useTranslation("bots");
	const { options, isLoading } = useBotModelOptions();
	const option = options.find((item) => item.id === id);
	if (option) return option.label;
	return isLoading
		? "…"
		: t("editor.brain.unknownModel", "Unknown model ({{id}})", { id });
}

function MemoryModelName({ id }: Readonly<{ id: string }>) {
	const { t } = useTranslation("bots");
	const { options, isLoading } = useBotEmbeddingModelOptions();
	const option = options.find((item) => item.id === id);
	if (option) return option.label;
	return isLoading
		? "…"
		: t("editor.brain.unknownModel", "Unknown model ({{id}})", { id });
}

function BotModels({ view }: Readonly<{ view: GlobalChatBotView }>) {
	const { t } = useTranslation("bots");
	const model = view.bot?.model;
	const embeddingModel = view.bot?.embedding_model;
	const fallbacks = model?.fallbacks.length ?? 0;
	return (
		<>
			<DetailRow
				icon={CpuIcon}
				label={t("editor.brain.primary", "Model")}
				hint={
					fallbacks > 0
						? t("panel.details.fallbacks", "Then {{count}} fallbacks", {
								count: fallbacks,
								defaultValue_one: "Then {{count}} fallback",
							})
						: undefined
				}
			>
				{model?.primary ? (
					<ModelName id={model.primary} />
				) : (
					t("editor.brain.automatic", "Automatic")
				)}
			</DetailRow>
			{subjectRemembers(view) && (
				<DetailRow
					icon={BrainIcon}
					label={t("editor.brain.memoryModel", "Memory model")}
				>
					{embeddingModel ? (
						<MemoryModelName id={embeddingModel} />
					) : (
						t("editor.brain.memoryHost", "Let the host choose")
					)}
				</DetailRow>
			)}
		</>
	);
}

/** Its abilities as chips; a FlowPilot bot has the one that is always on. */
function BotAbilities({ view }: Readonly<{ view: GlobalChatBotView }>) {
	const { t } = useTranslation("bots");
	const label = useBotCapabilityLabel();
	const capabilities = subjectCapabilities(view);
	const flowPilot = hasFlowPilot({
		system: subjectSystem(view),
		capabilities,
	});
	const shown = flowPilot ? [FLOWPILOT_CAPABILITY] : capabilities;
	return (
		<section className="flex flex-col gap-2">
			<h3 className="text-xs text-muted-foreground">
				{t("panel.details.abilities", "Abilities")}
			</h3>
			{shown.length === 0 ? (
				<p className="text-sm text-muted-foreground">
					{t("editor.preview.noCapabilities", "No abilities yet")}
				</p>
			) : (
				<ul className="flex flex-wrap gap-1.5">
					{shown.map((capability) => {
						const Icon = BOT_CAPABILITY_ICONS[capability];
						return (
							<li key={capability}>
								<BotTag className="text-foreground">
									<Icon className="size-3" aria-hidden />
									{label(capability)}
								</BotTag>
							</li>
						);
					})}
					{flowPilot && (
						<li>
							<BotTag>
								<LockIcon className="size-3" aria-hidden />
								{t("editor.abilities.flowpilot.badge", "Always on")}
							</BotTag>
						</li>
					)}
				</ul>
			)}
		</section>
	);
}

function BotActions({
	bot,
	actions,
}: Readonly<{ bot: IBot; actions: BotPanelActions }>) {
	const { t } = useTranslation("bots");
	const duplicateBot = useDuplicateBot();
	const [duplicating, setDuplicating] = useState(false);
	const duplicate = async () => {
		if (!duplicateBot || duplicating) return;
		setDuplicating(true);
		const copy = await duplicateBot(bot);
		setDuplicating(false);
		if (copy) actions.onDuplicated?.(copy);
	};
	const { onEdit, onMove } = actions;
	return (
		<div className="flex flex-wrap gap-2">
			{onEdit && (
				<Button
					type="button"
					variant="outline"
					size="sm"
					onClick={() => onEdit(bot)}
				>
					<PencilIcon aria-hidden />
					{t("panel.details.edit", "Edit")}
				</Button>
			)}
			{duplicateBot && (
				<Button
					type="button"
					variant="outline"
					size="sm"
					disabled={duplicating}
					onClick={() => void duplicate()}
				>
					{duplicating ? (
						<Loader2Icon className="animate-spin" aria-hidden />
					) : (
						<CopyIcon aria-hidden />
					)}
					{duplicating
						? t("editor.actions.duplicating", "Duplicating…")
						: t("editor.actions.duplicate", "Duplicate")}
				</Button>
			)}
			{bot.local && onMove && (
				<Button
					type="button"
					variant="outline"
					size="sm"
					onClick={() => onMove(bot)}
				>
					<CloudUploadIcon aria-hidden />
					{t("move.intro.action", "Move to your account")}
				</Button>
			)}
		</div>
	);
}

/** Who the bot is, where it runs, what it can do and the models it thinks and remembers with. */
export function BotDetails({
	view,
	actions,
}: Readonly<{ view: GlobalChatBotView; actions: BotPanelActions }>) {
	const { bot } = view;
	return (
		<div className="flex min-h-0 flex-1 flex-col gap-5 overflow-y-auto overscroll-contain px-5 pt-1 pb-6">
			{view.description && (
				<p className="text-sm leading-relaxed whitespace-pre-line text-muted-foreground">
					{view.description}
				</p>
			)}
			{bot && <BotWorking bot={bot} />}
			<dl className="flex flex-col gap-3.5">
				{bot && !hasFixedLocation(bot.system) && <WhereItRuns bot={bot} />}
				{bot && !hasFixedLocation(bot.system) && <BotStatus bot={bot} />}
				<BotModels view={view} />
			</dl>
			<BotAbilities view={view} />
			{bot && bot.system !== "flux" && (
				<BotToolGrants bot={bot} name={view.name} />
			)}
			{bot && bot.system !== "flux" && (
				<BotPermissions bot={bot} name={view.name} />
			)}
			{bot && bot.system !== "flux" && actions.onOpenConversation && (
				<BotExchanges bot={bot} onOpen={actions.onOpenConversation} />
			)}
			{bot && bot.system !== "flux" && <BotNotifications bot={bot} />}
			{bot && offersEncryption(bot) && <BotEncryption bot={bot} />}
			{bot && offersTasks(bot) && <BotTasks bot={bot} />}
			{bot && offersEmail(bot) && <BotEmailAddress botId={bot.id} />}
			{bot && offersWebhooks(bot) && <BotWebhooks bot={bot} />}
			{bot && <BotActions bot={bot} actions={actions} />}
		</div>
	);
}
