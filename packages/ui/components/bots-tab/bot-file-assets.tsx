"use client";

import { useQueryClient } from "@tanstack/react-query";
import { type ReactNode, useMemo } from "react";
import { useBackend } from "../../state/backend-state";
import { EditorAssetResolverContext } from "../editor/asset-resolver-context";
import { botFilePath, isBotFileReference } from "./bot-file-path";
import { botWorkspaceKey } from "./use-bot-workspace";

/** Workspace lookup stays lazy, so ordinary chat text never creates a workspace. */
function BotWorkspaceAssets({
	botId,
	children,
}: Readonly<{ botId: string; children: ReactNode }>) {
	const { botState, storageState } = useBackend();
	const queryClient = useQueryClient();
	const resolver = useMemo(
		() => ({
			accepts: isBotFileReference,
			resolve: async (url: string) => {
				const path = botFilePath(url);
				const getWorkspace = botState?.getWorkspace?.bind(botState);
				if (!getWorkspace) {
					throw new Error("This app cannot open bot files.");
				}
				const workspace = await queryClient.fetchQuery({
					queryKey: botWorkspaceKey(botId),
					queryFn: () => getWorkspace(botId),
					staleTime: Number.POSITIVE_INFINITY,
					gcTime: Number.POSITIVE_INFINITY,
				});
				if (!workspace?.app_id) {
					throw new Error("The bot's workspace is unavailable.");
				}
				const items = await storageState.downloadStorageItems(
					workspace.app_id,
					[path],
				);
				if (!items[0]?.url) throw new Error(`File is unavailable: ${path}`);
				return items[0].url;
			},
		}),
		[botId, botState, storageState, queryClient],
	);
	return (
		<EditorAssetResolverContext.Provider value={resolver}>
			{children}
		</EditorAssetResolverContext.Provider>
	);
}

/** A room message can select its sender's files; other messages inherit the chat's resolver. */
export function BotFileAssets({
	botId,
	children,
}: Readonly<{ botId?: string | null; children: ReactNode }>) {
	return botId ? (
		<BotWorkspaceAssets botId={botId}>{children}</BotWorkspaceAssets>
	) : (
		children
	);
}
