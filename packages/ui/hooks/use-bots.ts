"use client";

import { useTranslation } from "@flow-like/locales";
import {
	type QueryClient,
	useMutation,
	useQuery,
	useQueryClient,
} from "@tanstack/react-query";
import { useCallback } from "react";
import { useBackend } from "../state/backend-state";
import type {
	BotApprovalDecision,
	BotSystemKey,
	IBot,
	IBotState,
	IBotWebhook,
	ICreateBotRequest,
	ICreateBotTaskRequest,
	IUpdateBotRequest,
	IUpdateBotTaskRequest,
	IUpsertBotSecretRequest,
} from "../state/backend-state/bot-state";
import { inboxKeys } from "./use-inbox";

export const botKeys = {
	all: ["bots"] as const,
	list: () => [...botKeys.all, "list"] as const,
	detail: (botId: string) => [...botKeys.all, "detail", botId] as const,
	apps: (botId: string) => [...botKeys.all, "apps", botId] as const,
	secrets: (botId: string) => [...botKeys.all, "secrets", botId] as const,
	toolGrants: (botId: string) =>
		[...botKeys.all, "tool-grants", botId] as const,
	permissions: (botId: string) =>
		[...botKeys.all, "permissions", botId] as const,
	presence: () => [...botKeys.all, "presence"] as const,
	webhooks: (botId: string) => [...botKeys.all, "webhooks", botId] as const,
	tasks: (botId: string) => [...botKeys.all, "tasks", botId] as const,
	mailAddress: (botId: string) =>
		[...botKeys.all, "mail-address", botId] as const,
	mailThreads: (botId: string) =>
		[...botKeys.all, "mail-threads", botId] as const,
	mailThread: (botId: string, threadId: string) =>
		[...botKeys.all, "mail-thread", botId, threadId] as const,
};

const BOT_STALE_TIME = 30_000;

function requireBotState(state: IBotState | undefined): IBotState {
	if (!state) throw new Error("Bots need a signed-in Flow-Like account");
	return state;
}

/** A bot's sub is also cached as a directory entry; drop it so renamed or restyled bots redraw. */
function forgetBotLookups(queryClient: QueryClient, botId: string) {
	queryClient.invalidateQueries({ queryKey: ["lookupUserBatched", botId] });
}

function storeBot(queryClient: QueryClient, bot: IBot) {
	queryClient.setQueryData(botKeys.detail(bot.id), bot);
	queryClient.setQueryData<IBot[]>(botKeys.list(), (bots) => {
		if (!bots) return bots;
		const index = bots.findIndex((existing) => existing.id === bot.id);
		if (index === -1) return [...bots, bot];
		const next = bots.slice();
		next[index] = bot;
		return next;
	});
	forgetBotLookups(queryClient, bot.id);
}

/** The signed-in account's bots, built-in bots first. Disabled without a hub account. */
export function useBots() {
	const state = useBackend().botState;
	return useQuery({
		queryKey: botKeys.list(),
		queryFn: () => requireBotState(state).listBots(),
		enabled: Boolean(state),
		staleTime: BOT_STALE_TIME,
	});
}

export function useBot(botId?: string | null) {
	const state = useBackend().botState;
	const queryClient = useQueryClient();
	return useQuery({
		queryKey: botKeys.detail(botId ?? ""),
		queryFn: () => requireBotState(state).getBot(botId as string),
		enabled: Boolean(state && botId),
		staleTime: BOT_STALE_TIME,
		initialData: () =>
			queryClient
				.getQueryData<IBot[]>(botKeys.list())
				?.find((bot) => bot.id === botId),
	});
}

export function useCreateBot() {
	const state = useBackend().botState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: (request: ICreateBotRequest) =>
			requireBotState(state).createBot(request),
		onSuccess: (bot) => storeBot(queryClient, bot),
	});
}

/** A new custom bot copied from `botId`; only where the host offers `cloneBot`. */
export function useCloneBot() {
	const state = useBackend().botState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: async (botId: string) => {
			if (!state?.cloneBot)
				throw new Error(`Bot ${botId} can't be duplicated on this host`);
			return state.cloneBot(botId);
		},
		onSuccess: (bot) => storeBot(queryClient, bot),
	});
}

export function useUpdateBot() {
	const state = useBackend().botState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({
			botId,
			request,
		}: {
			botId: string;
			request: IUpdateBotRequest;
		}) => requireBotState(state).updateBot(botId, request),
		onSuccess: (bot) => storeBot(queryClient, bot),
	});
}

export function useDeleteBot() {
	const state = useBackend().botState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: (botId: string) => requireBotState(state).deleteBot(botId),
		onSuccess: (_, botId) => {
			queryClient.setQueryData<IBot[]>(botKeys.list(), (bots) =>
				bots?.filter((bot) => bot.id !== botId),
			);
			queryClient.removeQueries({ queryKey: botKeys.detail(botId) });
			forgetBotLookups(queryClient, botId);
		},
	});
}

export function useBotApps(botId?: string | null) {
	const state = useBackend().botState;
	return useQuery({
		queryKey: botKeys.apps(botId ?? ""),
		queryFn: () => requireBotState(state).listBotApps(botId as string),
		enabled: Boolean(state && botId),
	});
}

export function useSetBotApp() {
	const state = useBackend().botState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({
			botId,
			appId,
			attached,
		}: {
			botId: string;
			appId: string;
			attached: boolean;
		}) =>
			attached
				? requireBotState(state).attachBotApp(botId, appId)
				: requireBotState(state).detachBotApp(botId, appId),
		onSettled: (_, __, { botId }) =>
			queryClient.invalidateQueries({ queryKey: botKeys.apps(botId) }),
	});
}

export function useBotSecrets(botId?: string | null) {
	const state = useBackend().botState;
	return useQuery({
		queryKey: botKeys.secrets(botId ?? ""),
		queryFn: () => requireBotState(state).listBotSecrets(botId as string),
		enabled: Boolean(state && botId),
	});
}

export function useUpsertBotSecret() {
	const state = useBackend().botState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({
			botId,
			name,
			request,
		}: {
			botId: string;
			name: string;
			request: IUpsertBotSecretRequest;
		}) => requireBotState(state).upsertBotSecret(botId, name, request),
		gcTime: 0,
		onSettled: (_, __, { botId }) =>
			queryClient.invalidateQueries({ queryKey: botKeys.secrets(botId) }),
	});
}

export function useDeleteBotSecret() {
	const state = useBackend().botState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({ botId, name }: { botId: string; name: string }) =>
			requireBotState(state).deleteBotSecret(botId, name),
		onSettled: (_, __, { botId }) =>
			queryClient.invalidateQueries({ queryKey: botKeys.secrets(botId) }),
	});
}

/** Whom the bot may ask and who may ask it; the standing grants, not the one-off answers. */
export function useBotPermissions(botId?: string | null) {
	const state = useBackend().botState;
	return useQuery({
		queryKey: botKeys.permissions(botId ?? ""),
		queryFn: () => requireBotState(state).listPermissions(botId as string),
		enabled: Boolean(state && botId),
	});
}

/** Takes back a grant; every bot's grants are read again since the other side shows it too. */
export function useRevokeBotPermission() {
	const state = useBackend().botState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({
			fromBotId,
			toBotId,
		}: {
			fromBotId: string;
			toBotId: string;
		}) => requireBotState(state).revokePermission(fromBotId, toBotId),
		onSettled: () =>
			queryClient.invalidateQueries({
				queryKey: [...botKeys.all, "permissions"],
			}),
	});
}

/** What the bot may do without asking: the owner's "Always allow" answers. Absent where grants are not kept. */
export function useBotToolGrants(botId?: string | null) {
	const state = useBackend().botState;
	return useQuery({
		queryKey: botKeys.toolGrants(botId ?? ""),
		queryFn: () => state?.listToolGrants?.(botId as string) ?? [],
		enabled: Boolean(state?.listToolGrants && botId),
	});
}

export function useRevokeBotToolGrant() {
	const state = useBackend().botState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({
			botId,
			tool,
			scope,
		}: {
			botId: string;
			tool: string;
			scope: string;
		}) => {
			const revoke = requireBotState(state).deleteToolGrant;
			if (!revoke) throw new Error("This backend keeps no tool grants");
			return revoke.call(state, botId, tool, scope);
		},
		onSettled: (_, __, { botId }) =>
			queryClient.invalidateQueries({ queryKey: botKeys.toolGrants(botId) }),
	});
}

/** Answers an approval card in a bot's chat; the chat reads the answer back from the inbox. */
export function useAnswerBotApproval() {
	const state = useBackend().botState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({
			botId,
			turnId,
			approvalId,
			decision,
		}: {
			botId: string;
			turnId: string;
			approvalId: string;
			decision: BotApprovalDecision;
		}) => {
			const answer = requireBotState(state).answerApproval;
			if (!answer) throw new Error("This backend cannot answer approvals");
			return answer.call(state, botId, turnId, approvalId, decision);
		},
		onSettled: (_, __, { botId, decision }) => {
			queryClient.invalidateQueries({ queryKey: inboxKeys.all });
			if (decision === "always") {
				queryClient.invalidateQueries({ queryKey: botKeys.toolGrants(botId) });
			}
		},
	});
}

/** The name and description to show for a bot, with built-in defaults in the reader's language. */
export function useBotLabels() {
	const { t } = useTranslation("bots");
	return useCallback(
		(bot: {
			system: BotSystemKey | null;
			name?: string | null;
			description?: string | null;
		}) => ({
			name:
				bot.name?.trim() ||
				(bot.system === "flux"
					? t("system.flux.name")
					: bot.system === "general"
						? t("system.general.name")
						: ""),
			description:
				bot.description?.trim() ||
				(bot.system === "flux"
					? t("system.flux.description")
					: bot.system === "general"
						? t("system.general.description")
						: ""),
		}),
		[t],
	);
}

/** The scheduled tasks of a bot. */
export function useBotTasks(botId?: string | null) {
	const state = useBackend().botState;
	return useQuery({
		queryKey: botKeys.tasks(botId ?? ""),
		queryFn: () => requireBotState(state).listTasks?.(botId as string) ?? [],
		enabled: Boolean(state?.listTasks && botId),
	});
}

export function useCreateBotTask() {
	const state = useBackend().botState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({
			botId,
			request,
		}: { botId: string; request: ICreateBotTaskRequest }) =>
			requireBotState(state).createTask?.(botId, request),
		onSettled: (_, __, { botId }) =>
			queryClient.invalidateQueries({ queryKey: botKeys.tasks(botId) }),
	});
}

export function useUpdateBotTask() {
	const state = useBackend().botState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({
			botId,
			taskId,
			request,
		}: { botId: string; taskId: string; request: IUpdateBotTaskRequest }) =>
			requireBotState(state).updateTask?.(botId, taskId, request),
		onSettled: (_, __, { botId }) =>
			queryClient.invalidateQueries({ queryKey: botKeys.tasks(botId) }),
	});
}

export function useDeleteBotTask() {
	const state = useBackend().botState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({ botId, taskId }: { botId: string; taskId: string }) =>
			requireBotState(state).deleteTask?.(botId, taskId),
		onSettled: (_, __, { botId }) =>
			queryClient.invalidateQueries({ queryKey: botKeys.tasks(botId) }),
	});
}

/** The public URLs that wake an account bot. */
export function useBotWebhooks(botId?: string | null) {
	const state = useBackend().botState;
	return useQuery({
		queryKey: botKeys.webhooks(botId ?? ""),
		queryFn: () => requireBotState(state).listWebhooks?.(botId as string) ?? [],
		enabled: Boolean(state?.listWebhooks && botId),
	});
}

/** Creates or rotates a webhook; the result holds the only copy of its secret, so it is not cached. */
export function useIssueBotWebhook() {
	const state = useBackend().botState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({
			botId,
			hookId,
			request,
		}: {
			botId: string;
			hookId?: string;
			request?: { name: string; instruction?: string };
		}) => {
			const bots = requireBotState(state);
			if (hookId) return bots.rotateWebhook?.(botId, hookId);
			return bots.createWebhook?.(botId, request ?? { name: "" });
		},
		gcTime: 0,
		onSettled: (_, __, { botId }) =>
			queryClient.invalidateQueries({ queryKey: botKeys.webhooks(botId) }),
	});
}

export function useDeleteBotWebhook() {
	const state = useBackend().botState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({ botId, hookId }: { botId: string; hookId: string }) =>
			requireBotState(state).deleteWebhook?.(botId, hookId),
		onSettled: (_, __, { botId }) =>
			queryClient.invalidateQueries({ queryKey: botKeys.webhooks(botId) }),
	});
}

/** The full URL a webhook is called at. */
export function useBotWebhookUrl() {
	const state = useBackend().botState;
	return useCallback(
		(botId: string, webhook: IBotWebhook) =>
			requireBotState(state).webhookUrl?.(botId, webhook) ??
			Promise.resolve(webhook.path),
		[state],
	);
}

/** The bot's own mail address, if it claimed one. */
export function useBotMailAddress(botId?: string | null) {
	const state = useBackend().botState;
	return useQuery({
		queryKey: botKeys.mailAddress(botId ?? ""),
		queryFn: () => requireBotState(state).getMailAddress?.(botId as string),
		enabled: Boolean(state?.getMailAddress && botId),
	});
}

export function useCheckBotMailAddress() {
	const state = useBackend().botState;
	return useCallback(
		(botId: string, alias: string) =>
			requireBotState(state).checkMailAddress?.(botId, alias) ??
			Promise.resolve({ available: false }),
		[state],
	);
}

export function useClaimBotMailAddress() {
	const state = useBackend().botState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({ botId, alias }: { botId: string; alias: string }) =>
			requireBotState(state).claimMailAddress?.(botId, alias),
		onSettled: (_, __, { botId }) =>
			queryClient.invalidateQueries({ queryKey: botKeys.mailAddress(botId) }),
	});
}

export function useReleaseBotMailAddress() {
	const state = useBackend().botState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({ botId }: { botId: string }) =>
			requireBotState(state).releaseMailAddress?.(botId),
		onSettled: (_, __, { botId }) =>
			queryClient.invalidateQueries({ queryKey: botKeys.mailAddress(botId) }),
	});
}

/** The bot's mail threads, newest activity first. */
export function useBotMailThreads(botId?: string | null) {
	const state = useBackend().botState;
	return useQuery({
		queryKey: botKeys.mailThreads(botId ?? ""),
		queryFn: () =>
			requireBotState(state).listMailThreads?.(botId as string) ?? [],
		enabled: Boolean(state?.listMailThreads && botId),
		refetchInterval: 30_000,
	});
}

export function useBotMailThread(
	botId?: string | null,
	threadId?: string | null,
) {
	const state = useBackend().botState;
	return useQuery({
		queryKey: botKeys.mailThread(botId ?? "", threadId ?? ""),
		queryFn: () =>
			requireBotState(state).getMailThread?.(
				botId as string,
				threadId as string,
			),
		enabled: Boolean(state?.getMailThread && botId && threadId),
	});
}
