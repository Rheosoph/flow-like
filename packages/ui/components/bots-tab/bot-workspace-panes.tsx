"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ArrowUpRightIcon,
	BanIcon,
	CircleAlertIcon,
	CloudOffIcon,
	Loader2Icon,
} from "lucide-react";
import Link from "next/link";
import {
	type ReactNode,
	Suspense,
	useCallback,
	useMemo,
	useState,
} from "react";
import { useAppPermissions } from "../../hooks/use-app-permissions";
import { useInvoke } from "../../hooks/use-invoke";
import { getErrorMessage } from "../../lib/error-message";
import { RolePermissions } from "../../lib/permission/role-permission";
import { asArray } from "../../lib/response-shape";
import { useBackend } from "../../state/backend-state";
import type { IBot } from "../../state/backend-state/bot-state";
import {
	botHasInboxHistory,
	jumpBotChat,
} from "../../state/global-chat/bot-inbox";
import { ASSISTANT_BOT_ID } from "../../state/global-chat/global-chat-bots";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
import { MemoryList } from "../global-chat/memory-list";
import { Button } from "../ui/button";
import { BotFactsList } from "./bot-facts-list";
import { BotHistoryPanel } from "./bot-history-panel";
import { useBotWorkspace, workspaceFailure } from "./use-bot-workspace";
import { WorkspaceDataStudio, WorkspaceStorage } from "./workspace-views";

interface PaneProps {
	bot: IBot;
	/** The bot's display name, for the pane's messages. */
	name: string;
}

/** The workspace app's own pages, where its files and tables have the whole window. */
export function workspaceFullViewRoute(
	appId: string,
	view: "storage" | "explore",
): string {
	return `/library/config/${view}?${new URLSearchParams({ id: appId })}`;
}

function FullViewLink({ href }: Readonly<{ href: string }>) {
	const { t } = useTranslation("bots");
	return (
		<div className="flex shrink-0 justify-end px-3 pb-1">
			<Button
				asChild
				variant="ghost"
				size="sm"
				className="h-7 gap-1 px-2 text-xs text-muted-foreground"
			>
				<Link href={href}>
					{t("panel.openFull", "Open full view")}
					<ArrowUpRightIcon className="size-3.5" aria-hidden />
				</Link>
			</Button>
		</div>
	);
}

function PaneNotice({
	icon,
	title,
	detail,
	action,
}: Readonly<{
	icon: ReactNode;
	title: string;
	detail?: string;
	action?: ReactNode;
}>) {
	return (
		<div className="flex flex-1 flex-col items-center justify-center gap-3 px-6 py-10 text-center">
			<span className="flex size-11 items-center justify-center rounded-2xl bg-muted text-muted-foreground [&_svg]:size-5">
				{icon}
			</span>
			<p className="max-w-72 text-sm font-medium">{title}</p>
			{detail && (
				<p className="max-w-72 text-xs wrap-break-word text-muted-foreground">
					{detail}
				</p>
			)}
			{action}
		</div>
	);
}

function PaneLoading({ label }: Readonly<{ label: string }>) {
	return (
		<output className="flex flex-1 flex-col items-center justify-center gap-3 px-6 py-10 text-center text-sm text-muted-foreground">
			<Loader2Icon className="size-5 animate-spin" aria-hidden />
			{label}
		</output>
	);
}

function RetryButton({ onRetry }: Readonly<{ onRetry: () => void }>) {
	const { t } = useTranslation("bots");
	return (
		<Button type="button" variant="outline" size="sm" onClick={onRetry}>
			{t("crew.retry", "Try again")}
		</Button>
	);
}

/** Files and Data open in the bot's workspace app, set up the first time either is shown. */
function WorkspaceGate({
	bot,
	name,
	children,
}: Readonly<PaneProps & { children: (appId: string) => ReactNode }>) {
	const { t } = useTranslation("bots");
	const { supported, query } = useBotWorkspace(bot.id);
	const unavailable = (
		<PaneNotice
			icon={<CloudOffIcon aria-hidden />}
			title={t(
				"workspace.unsupported",
				"Files and data for {{name}} aren't available here yet.",
				{ name },
			)}
		/>
	);
	if (!supported) return unavailable;
	if (query.isPending || (query.isError && query.isFetching)) {
		return (
			<PaneLoading
				label={t("workspace.preparing", "Setting up {{name}}'s workspace…", {
					name,
				})}
			/>
		);
	}
	if (query.isError) {
		const failure = workspaceFailure(query.error);
		if (failure.kind === "unsupported") return unavailable;
		if (failure.kind === "refused") {
			return (
				<PaneNotice
					icon={<BanIcon aria-hidden />}
					title={t(
						"workspace.refused",
						"{{name}} can't keep its own files and data yet.",
						{ name },
					)}
					detail={failure.message}
				/>
			);
		}
		return (
			<PaneNotice
				icon={<CircleAlertIcon aria-hidden />}
				title={t("workspace.failed", "Couldn't set up {{name}}'s workspace.", {
					name,
				})}
				detail={failure.message}
				action={<RetryButton onRetry={() => void query.refetch()} />}
			/>
		);
	}
	return children(query.data.app_id);
}

function WorkspaceFiles({
	appId,
	name,
}: Readonly<{ appId: string; name: string }>) {
	const { t } = useTranslation("bots");
	const storage = useBackend().storageState;
	const permissions = useAppPermissions(appId);
	const [prefix, setPrefix] = useState("");
	const fileToUrl = useCallback(
		async (file: string) => {
			const [result] = asArray(
				await storage.downloadStorageItems(appId, [file]),
			);
			return result?.url ?? "";
		},
		[appId, storage],
	);
	return (
		<WorkspaceStorage
			key={`${appId}-${prefix}`}
			appId={appId}
			prefix={prefix}
			updatePrefix={setPrefix}
			fileToUrl={fileToUrl}
			title={t("workspace.filesTitle", "{{name}}'s files", { name })}
			enableCraftEditors
			canWrite={
				!permissions.isLoading && permissions.can(RolePermissions.WriteFiles)
			}
		/>
	);
}

export function BotFilesPane({ bot, name }: Readonly<PaneProps>) {
	const { t } = useTranslation("bots");
	return (
		<WorkspaceGate bot={bot} name={name}>
			{(appId) => (
				<>
					<FullViewLink href={workspaceFullViewRoute(appId, "storage")} />
					<Suspense
						fallback={
							<PaneLoading label={t("crew.dock.loading", "Loading…")} />
						}
					>
						<WorkspaceFiles appId={appId} name={name} />
					</Suspense>
				</>
			)}
		</WorkspaceGate>
	);
}

export function BotDataPane({ bot, name }: Readonly<PaneProps>) {
	const { t } = useTranslation("bots");
	return (
		<WorkspaceGate bot={bot} name={name}>
			{(appId) => (
				<>
					<FullViewLink href={workspaceFullViewRoute(appId, "explore")} />
					<Suspense
						fallback={
							<PaneLoading label={t("crew.dock.loading", "Loading…")} />
						}
					>
						<div className="flex min-h-0 flex-1 flex-col overflow-auto">
							<WorkspaceDataStudio appId={appId} />
						</div>
					</Suspense>
				</>
			)}
		</WorkspaceGate>
	);
}

/**
 * The memories of `botId` (`null` for Flux's, kept per profile). Memory is active while the bot or
 * the chat names an embedding model.
 */
function BotFactsPane({
	botId,
	name,
}: Readonly<{ botId: string; name: string }>) {
	const { t } = useTranslation("bots");
	const { query } = useBotWorkspace(botId);
	const appId = query.data?.app_id;
	return (
		<div className="flex min-h-0 flex-1 flex-col">
			<BotHistoryPanel botId={botId} name={name} />
			{appId ? (
				<BotFactsList
					botId={botId}
					appId={appId}
					name={name}
					onJump={(seq) => {
						if (botHasInboxHistory(botId)) void jumpBotChat(botId, seq);
					}}
				/>
			) : query.isError ? (
				<PaneNotice
					icon={<CircleAlertIcon aria-hidden />}
					title={t(
						"workspace.memoryFailed",
						"Couldn't load {{name}}'s memory.",
						{
							name,
						},
					)}
					detail={getErrorMessage(query.error)}
					action={<RetryButton onRetry={() => void query.refetch()} />}
				/>
			) : (
				<PaneLoading label={t("crew.dock.loading", "Loading…")} />
			)}
		</div>
	);
}

export function BotMemoryPane({
	botId,
	embeddingModel,
	name,
}: Readonly<{
	botId: string | null;
	embeddingModel?: string | null;
	name: string;
}>) {
	const { t } = useTranslation("bots");
	const backend = useBackend();
	const chatEmbeddingModel = useGlobalChatStore((s) => s.embeddingModelId);
	const active = Boolean(embeddingModel || chatEmbeddingModel);
	const profile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
		true,
	);
	const profileId = profile.data?.hub_profile.id;
	const owner = useMemo(
		() => (profileId ? { profileId, botId } : null),
		[profileId, botId],
	);
	if (botId && botId !== ASSISTANT_BOT_ID) {
		return <BotFactsPane botId={botId} name={name} />;
	}
	if (owner) {
		return (
			<MemoryList
				owner={owner}
				active={active}
				emptyHint={t(
					"workspace.memoryEmpty",
					"{{name}} saves facts, preferences and decisions here as you chat.",
					{ name },
				)}
				className="flex-1 px-4 pt-3 pb-4"
				listClassName="flex-1"
			/>
		);
	}
	if (profile.isError) {
		return (
			<PaneNotice
				icon={<CircleAlertIcon aria-hidden />}
				title={t("workspace.memoryFailed", "Couldn't load {{name}}'s memory.", {
					name,
				})}
				detail={getErrorMessage(profile.error)}
				action={<RetryButton onRetry={() => void profile.refetch()} />}
			/>
		);
	}
	return <PaneLoading label={t("crew.dock.loading", "Loading…")} />;
}
