"use client";

import { useTranslation } from "@flow-like/locales";
import {
	type QueryClient,
	useMutation,
	useQueries,
	useQuery,
	useQueryClient,
} from "@tanstack/react-query";
import { useCallback, useMemo } from "react";
import { useBackend } from "../state/backend-state";
import type {
	BotApprovalDecision,
	BotSystemKey,
	IBot,
	IBotConnector,
	IBotRestrictions,
	IBotSkill,
	IBotState,
	IBotWebhook,
	ICreateBotConnectorRequest,
	ICreateBotRequest,
	ICreateBotSkillRequest,
	ICreateBotTaskRequest,
	IIssuedBotWebhook,
	IUpdateBotConnectorRequest,
	IUpdateBotRequest,
	IUpdateBotSkillRequest,
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
	skills: (botId: string) => [...botKeys.all, "skills", botId] as const,
	connectors: (botId: string) => [...botKeys.all, "connectors", botId] as const,
	connectedAccounts: () => [...botKeys.all, "connected-accounts"] as const,
	mailAddress: (botId: string) =>
		[...botKeys.all, "mail-address", botId] as const,
	mailThreads: (botId: string) =>
		[...botKeys.all, "mail-threads", botId] as const,
	mailThread: (botId: string, threadId: string) =>
		[...botKeys.all, "mail-thread", botId, threadId] as const,
	avatar: (botId: string, look: string) =>
		[...botKeys.all, "avatar", botId, look] as const,
};

const BOT_STALE_TIME = 30_000;

function requireBotState(state: IBotState | undefined): IBotState {
	if (!state) throw new Error("Bots need a signed-in Flow-Like account");
	return state;
}

function unsupported<T>(): Promise<T> {
	return Promise.reject(
		new Error("This backend does not support that bot action"),
	);
}

/** A bot's sub is also cached as a directory entry; drop it so renamed or restyled bots redraw. */
function forgetBotLookups(queryClient: QueryClient, botId: string) {
	queryClient.invalidateQueries({ queryKey: ["lookupUserBatched", botId] });
}

/** Refresh settings written by a bot tool outside the settings editor. */
export function invalidateBotConfiguration(
	queryClient: QueryClient,
	botId?: string | null,
) {
	void queryClient.invalidateQueries({ queryKey: botKeys.list() });
	void queryClient.invalidateQueries({
		queryKey: [...botKeys.all, "on-this-computer"],
	});
	const system =
		!botId || botId === "flux"
			? "flux"
			: botId === "general"
				? "general"
				: undefined;
	const ids = new Set([
		...(botId ? [botId] : []),
		...(queryClient.getQueryData<IBot[]>(botKeys.list()) ?? [])
			.filter((bot) => system && bot.system === system)
			.map((bot) => bot.id),
	]);
	for (const id of ids) {
		void queryClient.invalidateQueries({ queryKey: botKeys.detail(id) });
		forgetBotLookups(queryClient, id);
	}
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

async function syncBotAvatar(state: IBotState, bot: IBot): Promise<boolean> {
	if (!state.getAvatar || !state.putAvatar) return false;
	try {
		if ((await state.getAvatar(bot.id)).url) return true;
		const { botAvatarPng } = await import("../components/bots/avatar-image");
		await state.putAvatar(
			bot.id,
			bot.appearance,
			await botAvatarPng(bot.appearance),
		);
		return true;
	} catch (error) {
		console.warn(
			`[Bots] Could not upload the notification avatar of bot ${bot.id}:`,
			error,
		);
		throw error;
	}
}

/**
 * Uploads a picture of each account bot's current look the hub has none of, so its push
 * notifications show the bot. Only clients can draw a look, so the ones showing the bots do.
 */
export function useBotAvatarSync(bots: readonly IBot[] | undefined) {
	const state = useBackend().botState;
	const accountBots = useMemo(
		() => (bots ?? []).filter((bot) => !bot.local),
		[bots],
	);
	useQueries({
		queries: accountBots.map((bot) => ({
			queryKey: botKeys.avatar(bot.id, JSON.stringify(bot.appearance)),
			queryFn: () => syncBotAvatar(requireBotState(state), bot),
			enabled: Boolean(state?.getAvatar && state.putAvatar),
			staleTime: Number.POSITIVE_INFINITY,
			// A later look's upload deletes this one's picture, so a look shown again checks again.
			gcTime: 0,
			retry: false,
			refetchOnWindowFocus: false,
			refetchOnReconnect: false,
		})),
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

export function useSetBotAppRestrictions() {
	const state = useBackend().botState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({
			botId,
			appId,
			restrictions,
		}: {
			botId: string;
			appId: string;
			restrictions: IBotRestrictions;
		}) =>
			requireBotState(state).setBotAppRestrictions?.(
				botId,
				appId,
				restrictions,
			) ?? unsupported<IBotRestrictions>(),
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

/** Lets one bot ask another without asking the owner; every bot's grants are read again. */
export function useGrantBotPermission() {
	const state = useBackend().botState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({
			fromBotId,
			toBotId,
		}: {
			fromBotId: string;
			toBotId: string;
		}) => requireBotState(state).grantPermission(fromBotId, toBotId),
		onSettled: () =>
			queryClient.invalidateQueries({
				queryKey: [...botKeys.all, "permissions"],
			}),
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
			requireBotState(state).createTask?.(botId, request) ?? unsupported(),
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
			requireBotState(state).updateTask?.(botId, taskId, request) ??
			unsupported(),
		onSettled: (_, __, { botId }) =>
			queryClient.invalidateQueries({ queryKey: botKeys.tasks(botId) }),
	});
}

export function useDeleteBotTask() {
	const state = useBackend().botState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({ botId, taskId }: { botId: string; taskId: string }) =>
			requireBotState(state).deleteTask?.(botId, taskId) ?? unsupported(),
		onSettled: (_, __, { botId }) =>
			queryClient.invalidateQueries({ queryKey: botKeys.tasks(botId) }),
	});
}

/** The bot's skills with their status; empty where skills are not kept. */
export function useBotSkills(botId?: string | null) {
	const state = useBackend().botState;
	return useQuery({
		queryKey: botKeys.skills(botId ?? ""),
		queryFn: async () =>
			(await requireBotState(state).listSkills?.(botId as string)) ?? [],
		enabled: Boolean(state?.listSkills && botId),
	});
}

export type BotSkillAction =
	| { type: "create"; request: ICreateBotSkillRequest }
	| { type: "update"; skillId: string; request: IUpdateBotSkillRequest }
	| { type: "approve"; skillId: string }
	| { type: "disable"; skillId: string }
	| { type: "delete"; skillId: string }
	| { type: "import"; zip: Uint8Array }
	| { type: "package"; packageId: string; version?: string };

/** Every change to a bot's skills; the list is refetched afterwards. */
export function useBotSkillAction() {
	const state = useBackend().botState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: async ({
			botId,
			action,
		}: {
			botId: string;
			action: BotSkillAction;
		}): Promise<IBotSkill | IBotSkill[] | undefined> => {
			const bots = requireBotState(state);
			switch (action.type) {
				case "create":
					return (
						bots.createSkill?.(botId, action.request) ??
						unsupported<IBotSkill>()
					);
				case "update":
					return (
						bots.updateSkill?.(botId, action.skillId, action.request) ??
						unsupported<IBotSkill>()
					);
				case "approve":
					return (
						bots.approveSkill?.(botId, action.skillId) ??
						unsupported<IBotSkill>()
					);
				case "disable":
					return (
						bots.disableSkill?.(botId, action.skillId) ??
						unsupported<IBotSkill>()
					);
				case "import":
					return (
						bots.importSkill?.(botId, action.zip) ?? unsupported<IBotSkill>()
					);
				case "package":
					return (
						bots.installSkillPackage?.(
							botId,
							action.packageId,
							action.version,
						) ?? unsupported<IBotSkill[]>()
					);
				case "delete":
					await (bots.deleteSkill?.(botId, action.skillId) ?? unsupported());
					return undefined;
			}
		},
		onSettled: (_, __, { botId }) =>
			queryClient.invalidateQueries({ queryKey: botKeys.skills(botId) }),
	});
}

/** The MCP servers and accounts attached to a bot. */
export function useBotConnectors(botId?: string | null) {
	const state = useBackend().botState;
	return useQuery({
		queryKey: botKeys.connectors(botId ?? ""),
		queryFn: async () =>
			(await requireBotState(state).listConnectors?.(botId as string)) ?? [],
		enabled: Boolean(state?.listConnectors && botId),
	});
}

/** The accounts the person connected, for the connector picker. */
export function useConnectedAccounts(enabled = true) {
	const state = useBackend().botState;
	return useQuery({
		queryKey: botKeys.connectedAccounts(),
		queryFn: async () =>
			(await requireBotState(state).listConnectedAccounts?.()) ?? [],
		enabled: enabled && Boolean(state?.listConnectedAccounts),
	});
}

export type BotConnectorAction =
	| { type: "create"; request: ICreateBotConnectorRequest }
	| { type: "update"; connectorId: string; request: IUpdateBotConnectorRequest }
	| { type: "refresh"; connectorId: string }
	| { type: "delete"; connectorId: string };

/** Every change to a bot's connectors; the list is refetched afterwards. */
export function useBotConnectorAction() {
	const state = useBackend().botState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: async ({
			botId,
			action,
		}: {
			botId: string;
			action: BotConnectorAction;
		}): Promise<IBotConnector | undefined> => {
			const bots = requireBotState(state);
			switch (action.type) {
				case "create":
					return (
						bots.createConnector?.(botId, action.request) ??
						unsupported<IBotConnector>()
					);
				case "update":
					return (
						bots.updateConnector?.(botId, action.connectorId, action.request) ??
						unsupported<IBotConnector>()
					);
				case "refresh":
					return (
						bots.refreshConnector?.(botId, action.connectorId) ??
						unsupported<IBotConnector>()
					);
				case "delete":
					await (bots.deleteConnector?.(botId, action.connectorId) ??
						unsupported());
					return undefined;
			}
		},
		onSettled: (_, __, { botId }) =>
			queryClient.invalidateQueries({ queryKey: botKeys.connectors(botId) }),
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
			if (hookId)
				return (
					bots.rotateWebhook?.(botId, hookId) ??
					unsupported<IIssuedBotWebhook>()
				);
			return (
				bots.createWebhook?.(botId, request ?? { name: "" }) ??
				unsupported<IIssuedBotWebhook>()
			);
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
			requireBotState(state).deleteWebhook?.(botId, hookId) ?? unsupported(),
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
			requireBotState(state).claimMailAddress?.(botId, alias) ?? unsupported(),
		onSettled: (_, __, { botId }) =>
			queryClient.invalidateQueries({ queryKey: botKeys.mailAddress(botId) }),
	});
}

export function useReleaseBotMailAddress() {
	const state = useBackend().botState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({ botId }: { botId: string }) =>
			requireBotState(state).releaseMailAddress?.(botId) ?? unsupported(),
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
