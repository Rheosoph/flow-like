"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowLeftIcon, MessageCircleIcon } from "lucide-react";
import { usePathname } from "next/navigation";
import { useCallback, useId, useState } from "react";
import { toast } from "sonner";
import { useCreateConversation } from "../../hooks/use-inbox";
import { useUserIdentity } from "../../hooks/use-user-lookup";
import { useClientRouter } from "../../lib/client-navigation";
import type { IBot } from "../../state/backend-state/bot-state";
import { useChatBotView } from "../global-chat/use-global-chat-bot";
import { Button } from "../ui/button";
import { Skeleton } from "../ui/skeleton";
import { IdentityProfileContent } from "../ui/user-identity";
import type { BotPanelActions } from "./bot-details";
import { BotOverviewPanel } from "./bot-overview-panel";
import {
	type BotPanelTab,
	DEFAULT_BOT_PANEL_TAB,
	botPanelTabs,
	shownBotPanelTab,
} from "./bot-panel-tabs";
import { isSelfPrincipal } from "./crew-model";
import {
	type DockSelection,
	chatBotIdOf,
	dockSelectionForBot,
	dockSelectionQuery,
} from "./dock-selection";

/** Moves the dock to another chat the way a link would; the dock follows the URL. */
function useShowInDock() {
	const router = useClientRouter();
	const pathname = usePathname();
	return useCallback(
		(selection: DockSelection) => {
			const query = dockSelectionQuery(selection, window.location.search);
			router.replace(`${pathname}${query ? `?${query}` : ""}`, {
				scroll: false,
			});
		},
		[pathname, router],
	);
}

const ROUND_BUTTON = "size-8 rounded-full text-muted-foreground";

function OwnBotInfo({
	bot,
	bots,
	actions,
	onClose,
}: Readonly<{
	bot: IBot;
	bots: readonly IBot[];
	actions: BotPanelActions;
	onClose: () => void;
}>) {
	const { t } = useTranslation("bots");
	const panelId = useId();
	const view = useChatBotView(chatBotIdOf(bot));
	const tabs = botPanelTabs(view);
	const [tab, setTab] = useState<BotPanelTab>(DEFAULT_BOT_PANEL_TAB);
	const showInDock = useShowInDock();
	const chatLabel = t("crew.memberInfo.openChat", "Chat with {{name}}", {
		name: view.name,
	});
	return (
		<BotOverviewPanel
			id={panelId}
			view={view}
			layout="sheet"
			tabs={{
				list: tabs,
				shown: shownBotPanelTab(tab, tabs) ?? DEFAULT_BOT_PANEL_TAB,
				onChange: setTab,
			}}
			actions={actions}
			trailing={
				<Button
					type="button"
					variant="ghost"
					size="icon"
					className={ROUND_BUTTON}
					onClick={() => showInDock(dockSelectionForBot(bot, bots))}
					aria-label={chatLabel}
					title={chatLabel}
				>
					<MessageCircleIcon aria-hidden />
				</Button>
			}
			onClose={onClose}
		/>
	);
}

function MessageButton({ principalId }: Readonly<{ principalId: string }>) {
	const { t } = useTranslation("bots");
	const create = useCreateConversation();
	const showInDock = useShowInDock();
	const open = () => {
		if (create.isPending) return;
		create.mutate(
			{ kind: "direct", members: [principalId] },
			{
				onSuccess: (conversation) =>
					showInDock({
						type: "conversation",
						conversationId: conversation.id,
					}),
				onError: (error) => {
					console.warn("Opening a direct conversation failed", error);
					toast.error(
						t(
							"crew.memberInfo.messageFailed",
							"Couldn't open the conversation. Try again.",
						),
					);
				},
			},
		);
	};
	return (
		<Button
			type="button"
			className="w-full rounded-xl"
			disabled={create.isPending}
			onClick={open}
		>
			<MessageCircleIcon aria-hidden />
			{t("crew.memberInfo.message", "Message")}
		</Button>
	);
}

function ProfileInfo({
	principalId,
	me,
	onClose,
}: Readonly<{ principalId: string; me?: string | null; onClose: () => void }>) {
	const { t } = useTranslation("bots");
	const identity = useUserIdentity(principalId);
	const back = t("panel.back", "Back to the chat");
	const person =
		identity.isResolved && !identity.bot && !isSelfPrincipal(principalId, me);
	return (
		<section
			aria-label={t("crew.memberInfo.label", "About {{name}}", {
				name: identity.label,
			})}
			className="flex h-full min-h-0 min-w-0 flex-col bg-background"
		>
			<div className="flex shrink-0 items-center px-2 pt-2">
				<Button
					type="button"
					variant="ghost"
					size="icon"
					className={ROUND_BUTTON}
					onClick={onClose}
					aria-label={back}
					title={back}
				>
					<ArrowLeftIcon aria-hidden />
				</Button>
			</div>
			<div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-3 pt-2 pb-6">
				<div className="overflow-hidden rounded-2xl border bg-card">
					{identity.isPending ? (
						<div className="flex items-center gap-3 p-4">
							<Skeleton className="size-12 rounded-full" />
							<div className="flex flex-1 flex-col gap-2">
								<Skeleton className="h-4 w-32" />
								<Skeleton className="h-3 w-20" />
							</div>
						</div>
					) : (
						<IdentityProfileContent identity={identity} />
					)}
				</div>
				{person && <MessageButton principalId={principalId} />}
			</div>
		</section>
	);
}

/**
 * Who someone in a conversation is, over the conversation: your own bot opens its overview with
 * every tab, as from its chat; anyone else shows the card their hover shows on a desktop.
 */
export function MemberInfoPanel({
	principalId,
	me,
	bots,
	actions,
	onClose,
}: Readonly<{
	principalId: string;
	me?: string | null;
	bots?: readonly IBot[];
	actions: BotPanelActions;
	onClose: () => void;
}>) {
	const own = bots?.find((bot) => bot.id === principalId);
	if (own && bots) {
		return (
			<OwnBotInfo bot={own} bots={bots} actions={actions} onClose={onClose} />
		);
	}
	return <ProfileInfo principalId={principalId} me={me} onClose={onClose} />;
}
