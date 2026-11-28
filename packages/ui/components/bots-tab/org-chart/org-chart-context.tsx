"use client";

import { useTranslation } from "@flow-like/locales";
import {
	type ReactNode,
	createContext,
	useCallback,
	useContext,
	useMemo,
	useState,
} from "react";
import { toast } from "sonner";
import { useTeamActions } from "../../../hooks/use-bot-teams";
import { useBotLabels } from "../../../hooks/use-bots";
import { orgBot } from "../../../lib/bot-teams/org-model";
import type { IBot } from "../../../state/backend-state/bot-state";
import type {
	IOrgBot,
	IOrgView,
	ITeam,
} from "../../../state/backend-state/bot-team-state";
import { type AppFilter, appLinks } from "./org-chart-apps";
import {
	type Highlight,
	dropAction,
	highlightFor,
	toOrgBot,
} from "./org-chart-model";

export type Selection =
	| { kind: "bot"; id: string }
	| { kind: "team"; id: string }
	| { kind: "app"; id: string }
	| null;

export type DialogState =
	| { kind: "newTeam"; parentId: string | null }
	| { kind: "addPeople"; teamId: string }
	| { kind: "addBot"; teamId: string }
	| { kind: "pinApp"; teamId: string }
	| null;

export interface OrgChartContextValue {
	org: IOrgView;
	me: string | null;
	ownBots: readonly IBot[];
	botOf: (id: string) => IOrgBot | undefined;
	nameOf: (bot: Pick<IOrgBot, "system" | "name">) => string;
	selection: Selection;
	select: (selection: Selection) => void;
	selectedBotId: string | null;
	highlight: Highlight;
	showAsk: boolean;
	/** Mail addresses, webhooks and sent mail beside the teams. */
	showIo: boolean;
	/** Each team's apps under it. */
	showApps: boolean;
	appFilter: AppFilter;
	dragBotId: string | null;
	setDragBotId: (id: string | null) => void;
	dialog: DialogState;
	setDialog: (dialog: DialogState) => void;
	actions: ReturnType<typeof useTeamActions>;
	placeBot: (team: ITeam, bot: IOrgBot) => void;
	onOpenChat: (botId: string) => void;
	onOpenConversation: (conversationId: string) => void;
	onOpenApp: (appId: string, where: AppDestination) => void;
}

/** `app` opens the app; `store` its store page, where people join or ask to. */
export type AppDestination = "app" | "store";

const OrgChartContext = createContext<OrgChartContextValue | null>(null);

export function useOrgChart(): OrgChartContextValue {
	const value = useContext(OrgChartContext);
	if (!value) throw new Error("useOrgChart needs an OrgChartProvider");
	return value;
}

export function OrgChartProvider({
	org,
	me,
	bots,
	showAsk,
	showIo,
	showApps,
	appFilter,
	onOpenChat,
	onOpenConversation,
	onOpenApp,
	children,
}: Readonly<{
	org: IOrgView;
	me: string | null;
	bots: readonly IBot[];
	showAsk: boolean;
	showIo: boolean;
	showApps: boolean;
	appFilter: AppFilter;
	onOpenChat: (botId: string) => void;
	onOpenConversation: (conversationId: string) => void;
	onOpenApp: (appId: string, where: AppDestination) => void;
	children: ReactNode;
}>) {
	const { t } = useTranslation("bots");
	const labels = useBotLabels();
	const actions = useTeamActions();
	const [selection, setSelection] = useState<Selection>(null);
	const [dragBotId, setDragBotId] = useState<string | null>(null);
	const [dialog, setDialog] = useState<DialogState>(null);

	const ownBots = useMemo(
		() => bots.filter((bot) => bot.owner_id === me),
		[bots, me],
	);
	const botOf = useCallback(
		(id: string): IOrgBot | undefined =>
			orgBot(org, id) ??
			(() => {
				const own = bots.find((bot) => bot.id === id);
				return own ? toOrgBot(own) : undefined;
			})(),
		[org, bots],
	);
	const nameOf = useCallback(
		(bot: Pick<IOrgBot, "system" | "name">) =>
			labels({ system: bot.system ?? null, name: bot.name }).name ||
			t("teams.org.unnamedBot", "Bot"),
		[labels, t],
	);
	const selectedBotId = selection?.kind === "bot" ? selection.id : null;
	const selectedAppId = selection?.kind === "app" ? selection.id : null;
	const highlight = useMemo(
		() =>
			selectedAppId
				? { ...appLinks(org, selectedAppId), appIds: new Set([selectedAppId]) }
				: highlightFor(org, selectedBotId),
		[org, selectedBotId, selectedAppId],
	);

	const placeBot = useCallback(
		(team: ITeam, bot: IOrgBot) => {
			const action = dropAction(team, bot, me);
			if (!action) return;
			actions.addBot.mutate(
				{ teamId: team.id, botId: bot.id },
				{
					onSuccess: () =>
						toast.success(
							action === "add"
								? t("teams.org.toast.added", "{{bot}} joined {{team}}", {
										bot: nameOf(bot),
										team: team.name,
									})
								: t(
										"teams.org.toast.invited",
										"Invitation sent. {{bot}} joins {{team}} once its owner accepts",
										{ bot: nameOf(bot), team: team.name },
									),
						),
					onError: (error) => toast.error(error.message),
				},
			);
		},
		[actions.addBot, me, nameOf, t],
	);

	const value = useMemo<OrgChartContextValue>(
		() => ({
			org,
			me,
			ownBots,
			botOf,
			nameOf,
			selection,
			select: setSelection,
			selectedBotId,
			highlight,
			showAsk,
			showIo,
			showApps,
			appFilter,
			dragBotId,
			setDragBotId,
			dialog,
			setDialog,
			actions,
			placeBot,
			onOpenChat,
			onOpenConversation,
			onOpenApp,
		}),
		[
			org,
			me,
			ownBots,
			botOf,
			nameOf,
			selection,
			selectedBotId,
			highlight,
			showAsk,
			showIo,
			showApps,
			appFilter,
			dragBotId,
			dialog,
			actions,
			placeBot,
			onOpenChat,
			onOpenConversation,
			onOpenApp,
		],
	);

	return (
		<OrgChartContext.Provider value={value}>
			{children}
		</OrgChartContext.Provider>
	);
}
