"use client";

import { useTranslation } from "@flow-like/locales";
import { PlusIcon, Settings2Icon } from "lucide-react";
import { useMemo } from "react";
import { useBotLabels } from "../../hooks/use-bots";
import {
	type BotLocation,
	type BotSystemKey,
	type IBot,
	type IBotPresence,
	SYSTEM_BOT_APPEARANCES,
} from "../../state/backend-state/bot-state";
import { BotAvatar } from "../bots/bot-avatar";
import type { BotAppearance } from "../bots/types";
import { useChatBotReaction } from "../global-chat/chat-bot-reaction";
import { Skeleton } from "../ui/skeleton";
import {
	BotLocationBadge,
	BuiltInBadge,
	EncryptedBadge,
	LocalBadge,
	useBotPresenceText,
	useBotStatusText,
} from "./bot-labels";
import { StatusDot, crewCardClass } from "./crew-section";
import { type DockSelection, dockSelectionForBot } from "./dock-selection";

export interface CrewMember {
	key: string;
	selection: DockSelection;
	/** Absent for the built-in placeholders shown without an account. */
	bot?: IBot;
	system: BotSystemKey | null;
	name: string;
	description: string;
	location?: BotLocation;
	encrypted: boolean;
	/** Lives only on this computer. */
	local: boolean;
	appearance?: Partial<BotAppearance>;
}

function CrewStatus({
	member,
	presence,
}: Readonly<{ member: CrewMember; presence?: IBotPresence | null }>) {
	const status = useBotStatusText()(member.system, member.location);
	const presenceText = useBotPresenceText();
	if (presence && !member.system) {
		const hosted = member.location !== "server";
		return (
			<>
				<StatusDot muted={presence.state !== "online"} />
				{hosted ? presenceText(presence.state) : status.label}
			</>
		);
	}
	return (
		<>
			<StatusDot muted={!status.ready} />
			{status.label}
		</>
	);
}

export function CrewBotCard({
	member,
	presence,
	selected,
	onSelect,
	onSettings,
}: Readonly<{
	member: CrewMember;
	/** Where the bot stands when it runs in the account; none for bots on this computer. */
	presence?: IBotPresence | null;
	selected: boolean;
	onSelect: (member: CrewMember) => void;
	onSettings?: (member: CrewMember) => void;
}>) {
	const { t } = useTranslation("bots");
	const reaction = useChatBotReaction(
		selected
			? member.system === "flux"
				? null
				: (member.bot?.id ?? member.system ?? member.key)
			: undefined,
	);
	return (
		<div
			className={crewCardClass(
				selected,
				"group relative flex min-w-0 @max-[520px]/crew:rounded-xl",
			)}
		>
			<button
				type="button"
				aria-current={selected ? "true" : undefined}
				onClick={() => onSelect(member)}
				className="flex w-full min-w-0 flex-col items-center gap-2 rounded-2xl px-2.5 pt-3 pb-3.5 text-center outline-none focus-visible:ring-2 focus-visible:ring-ring @max-[520px]/crew:flex-row @max-[520px]/crew:gap-3 @max-[520px]/crew:rounded-xl @max-[520px]/crew:px-3 @max-[520px]/crew:py-2.5 @max-[520px]/crew:pr-10 @max-[520px]/crew:text-left"
			>
				<span className="relative flex h-30.5 w-full items-end justify-center @max-[520px]/crew:size-11 @max-[520px]/crew:shrink-0">
					<span
						aria-hidden
						className="absolute bottom-0.5 left-1/2 h-3.5 w-20 -translate-x-1/2 rounded-full bg-muted blur-sm @max-[520px]/crew:hidden"
					/>
					<BotAvatar
						appearance={member.appearance}
						expression={reaction.expression}
						expressionOverridesState={reaction.expressionOverridesState}
						agentState={reaction.agentState}
						size={104}
						decorative
						className="relative @max-[520px]/crew:[--bot-size:44px]!"
					/>
					{member.encrypted && (
						<EncryptedBadge className="absolute top-0 right-0.5 @max-[520px]/crew:-right-1 @max-[520px]/crew:-top-1" />
					)}
				</span>
				<span className="flex w-full min-w-0 flex-col items-center gap-1 @max-[520px]/crew:flex-1 @max-[520px]/crew:items-start @max-[520px]/crew:gap-0.5">
					<span className="max-w-full truncate text-[15px] font-semibold">
						{member.name}
					</span>
					<span className="inline-flex max-w-full items-center gap-1.5 text-xs font-medium text-muted-foreground @max-[520px]/crew:hidden">
						<CrewStatus member={member} presence={presence} />
					</span>
					<span className="line-clamp-2 min-h-8.5 text-xs leading-snug text-muted-foreground @max-[520px]/crew:line-clamp-1 @max-[520px]/crew:min-h-0">
						{member.description}
					</span>
				</span>
				<span className="flex flex-wrap justify-center gap-1 @max-[520px]/crew:hidden">
					{member.local ? (
						<LocalBadge />
					) : (
						member.location && <BotLocationBadge location={member.location} />
					)}
					{member.system && <BuiltInBadge />}
				</span>
			</button>
			{onSettings && (
				<button
					type="button"
					onClick={() => onSettings(member)}
					aria-label={t("crew.botSettings", "Settings for {{name}}", {
						name: member.name,
					})}
					title={t("crew.botSettings", "Settings for {{name}}", {
						name: member.name,
					})}
					className="absolute top-2 left-2 inline-flex size-7 items-center justify-center rounded-lg text-muted-foreground opacity-70 outline-none transition-opacity hover:bg-muted hover:text-foreground hover:opacity-100 focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-ring group-hover:opacity-100 @max-[520px]/crew:top-1/2 @max-[520px]/crew:left-auto @max-[520px]/crew:right-2 @max-[520px]/crew:-translate-y-1/2"
				>
					<Settings2Icon className="size-4" aria-hidden />
				</button>
			)}
		</div>
	);
}

export function NewBotTile({
	disabled,
	onClick,
}: Readonly<{ disabled: boolean; onClick: () => void }>) {
	const { t } = useTranslation("bots");
	return (
		<button
			type="button"
			disabled={disabled}
			onClick={onClick}
			className="flex min-h-59 flex-col items-center justify-center gap-2.5 rounded-2xl border-[1.5px] border-dashed border-border px-2.5 py-3.5 text-center text-muted-foreground outline-none transition-colors hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60 @max-[520px]/crew:min-h-11 @max-[520px]/crew:flex-row @max-[520px]/crew:rounded-xl @max-[520px]/crew:py-2"
		>
			<span className="flex size-11 items-center justify-center rounded-full bg-muted text-foreground @max-[520px]/crew:size-7">
				<PlusIcon className="size-5" aria-hidden />
			</span>
			<span className="text-sm font-semibold text-foreground">
				{t("crew.newBot", "New bot")}
			</span>
			<span className="text-xs leading-snug @max-[520px]/crew:hidden">
				{t(
					"crew.newBotHint",
					"Describe what it should do, or start from a template",
				)}
			</span>
		</button>
	);
}

export function CrewGridSkeleton() {
	return (
		<>
			<Skeleton className="min-h-59 rounded-2xl @max-[520px]/crew:min-h-17 @max-[520px]/crew:rounded-xl" />
			<Skeleton className="min-h-59 rounded-2xl @max-[520px]/crew:min-h-17 @max-[520px]/crew:rounded-xl" />
			<Skeleton className="min-h-59 rounded-2xl @max-[520px]/crew:min-h-17 @max-[520px]/crew:rounded-xl" />
		</>
	);
}

export const CREW_GRID_CLASS =
	"grid grid-cols-[repeat(auto-fill,minmax(min(140px,100%),1fr))] gap-3 @max-[520px]/crew:grid-cols-1 @max-[520px]/crew:gap-1.5";

const BUILT_IN_KEYS: readonly BotSystemKey[] = ["general", "flux"];

/**
 * Cards for the caller's bots. Built-in bots the list doesn't hold (no list yet, no account,
 * or only bots on this device) still open their chat, ahead of the rest.
 */
export function useCrewMembers(bots?: readonly IBot[]): CrewMember[] {
	const botLabels = useBotLabels();
	return useMemo(() => {
		const placeholders = BUILT_IN_KEYS.filter(
			(system) => !bots?.some((bot) => bot.system === system),
		).map((system): CrewMember => {
			const { name, description } = botLabels({ system });
			return {
				key: system,
				selection: { type: "system", system },
				system,
				name,
				description,
				encrypted: false,
				local: false,
				appearance: SYSTEM_BOT_APPEARANCES[system],
			};
		});
		const members = (bots ?? []).map((bot): CrewMember => {
			const { name, description } = botLabels(bot);
			return {
				key: bot.id,
				selection: dockSelectionForBot(bot, bots),
				bot,
				system: bot.system,
				name,
				description,
				location: bot.system ? undefined : bot.location,
				encrypted: bot.capabilities.includes("encryption"),
				local: bot.local === true,
				appearance: bot.appearance,
			};
		});
		return [...placeholders, ...members];
	}, [bots, botLabels]);
}
