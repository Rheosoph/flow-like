"use client";

import {
	type QueryClient,
	useMutation,
	useQuery,
	useQueryClient,
} from "@tanstack/react-query";
import { useMemo } from "react";
import { adaptiveRefetchInterval } from "../lib/adaptive-polling";
import type { DataClassification } from "../lib/data-classification";
import { useBackend } from "../state/backend-state";
import type {
	IBotTeamState,
	ICreateTeamRequest,
	IOrgView,
	ITeam,
	IUpdateTeamRequest,
	TeamRole,
} from "../state/backend-state/bot-team-state";
import type { IBotRoomLimits } from "../state/backend-state/inbox-state";
import { inboxKeys, useSetConversationImage } from "./use-inbox";

export const botTeamKeys = {
	all: ["bot-teams"] as const,
	org: () => [...botTeamKeys.all, "org"] as const,
	briefing: (botId: string) => [...botTeamKeys.all, "briefing", botId] as const,
	pinnable: (teamId: string) =>
		[...botTeamKeys.all, "pinnable", teamId] as const,
};

const ORG_STALE_TIME = 30_000;
const ORG_REFRESH = adaptiveRefetchInterval({ baseMs: 60_000 });

const EMPTY_ORG: IOrgView = {
	teams: [],
	bots: [],
	invitations: [],
	threads: [],
	reach: [],
	chats: [],
	classifications: [],
	apps: [],
};

function requireTeamState(state: IBotTeamState | undefined): IBotTeamState {
	if (!state) throw new Error("Bot teams need a hub account");
	return state;
}

/** After any team change: the org, every briefing, and the conversations team rooms live in. */
export function invalidateTeams(queryClient: QueryClient) {
	void queryClient.invalidateQueries({ queryKey: botTeamKeys.all });
	void queryClient.invalidateQueries({ queryKey: inboxKeys.all });
}

function storeTeamImage(
	queryClient: QueryClient,
	teamId: string,
	imageUrl: string | null,
) {
	queryClient.setQueryData<IOrgView>(botTeamKeys.org(), (org) =>
		org
			? {
					...org,
					teams: org.teams.map((team) =>
						team.id === teamId ? { ...team, image_url: imageUrl } : team,
					),
				}
			: org,
	);
}

/** Sets or removes a team's icon, which is its room's picture, and shows it in the org at once. */
export function useSetTeamIcon() {
	const setImage = useSetConversationImage();
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({
			roomId,
			image,
		}: {
			teamId: string;
			roomId: string;
			image: string | null;
		}) => setImage.mutateAsync({ conversationId: roomId, image }),
		onSuccess: (room, { teamId }) =>
			storeTeamImage(queryClient, teamId, room.image_url ?? null),
	});
}

/** Every org the reader belongs to. Empty without a hub account. */
export function useOrg() {
	const state = useBackend().botTeamState;
	const query = useQuery({
		queryKey: botTeamKeys.org(),
		queryFn: () => requireTeamState(state).getOrg(),
		enabled: Boolean(state),
		staleTime: ORG_STALE_TIME,
		refetchInterval: ORG_REFRESH,
		refetchOnWindowFocus: true,
	});
	return {
		...query,
		org: query.data ?? EMPTY_ORG,
		available: Boolean(state),
	};
}

/** The text a bot in a team reads about its team and org before every turn. */
export function useTeamBriefing(botId: string | null | undefined) {
	const state = useBackend().botTeamState;
	return useQuery({
		queryKey: botTeamKeys.briefing(botId ?? ""),
		queryFn: () => requireTeamState(state).getBriefing(botId as string),
		enabled: Boolean(state && botId),
		staleTime: ORG_STALE_TIME,
	});
}

/** The reader's apps for a team's pin dialog; loads only while `enabled`. */
export function usePinnableApps(teamId: string | null, enabled: boolean) {
	const state = useBackend().botTeamState;
	return useQuery({
		queryKey: botTeamKeys.pinnable(teamId ?? ""),
		queryFn: () => requireTeamState(state).listPinnableApps(teamId as string),
		enabled: Boolean(state && teamId && enabled),
		staleTime: ORG_STALE_TIME,
	});
}

/**
 * Which bots would stop answering in the team room if each of these people or bots joined.
 * Only admins preview people; the result is empty while nothing is asked for.
 */
export function useTeamPreview(
	teamId: string | null | undefined,
	userIds: readonly string[],
	botIds: readonly string[],
) {
	const state = useBackend().botTeamState;
	const users = useMemo(() => [...userIds].sort(), [userIds]);
	const bots = useMemo(() => [...botIds].sort(), [botIds]);
	return useQuery({
		queryKey: [...botTeamKeys.all, "preview", teamId ?? "", users, bots],
		queryFn: () =>
			requireTeamState(state).previewTeam(teamId as string, {
				user_ids: users,
				bot_ids: bots,
			}),
		enabled: Boolean(state && teamId && (users.length > 0 || bots.length > 0)),
		staleTime: ORG_STALE_TIME,
	});
}

function useTeamMutation<TArgs, TResult>(
	run: (state: IBotTeamState, args: TArgs) => Promise<TResult>,
) {
	const state = useBackend().botTeamState;
	const queryClient = useQueryClient();
	return useMutation<TResult, Error, TArgs>({
		mutationFn: (args: TArgs) => run(requireTeamState(state), args),
		onSuccess: (result) => {
			if (result && typeof result === "object" && "id" in result) {
				const team = result as unknown as ITeam;
				queryClient.setQueryData<IOrgView>(botTeamKeys.org(), (org) =>
					org
						? {
								...org,
								teams: org.teams.some((existing) => existing.id === team.id)
									? org.teams.map((existing) =>
											existing.id === team.id ? team : existing,
										)
									: [...org.teams, team],
							}
						: org,
				);
			}
			invalidateTeams(queryClient);
		},
	});
}

/** Every write the org view and the team settings make. */
export function useTeamActions() {
	return {
		createTeam: useTeamMutation((state, request: ICreateTeamRequest) =>
			state.createTeam(request),
		),
		updateTeam: useTeamMutation(
			(state, args: { teamId: string; request: IUpdateTeamRequest }) =>
				state.updateTeam(args.teamId, args.request),
		),
		deleteTeam: useTeamMutation((state, teamId: string) =>
			state.deleteTeam(teamId),
		),
		invitePeople: useTeamMutation(
			(state, args: { teamId: string; userIds: string[]; role?: TeamRole }) =>
				state.invitePeople(args.teamId, args.userIds, args.role),
		),
		removePerson: useTeamMutation(
			(state, args: { teamId: string; userId: string }) =>
				state.removePerson(args.teamId, args.userId),
		),
		setPersonRole: useTeamMutation(
			(state, args: { teamId: string; userId: string; role: TeamRole }) =>
				state.setPersonRole(args.teamId, args.userId, args.role),
		),
		acceptTeamInvite: useTeamMutation((state, teamId: string) =>
			state.acceptTeamInvite(teamId),
		),
		declineTeamInvite: useTeamMutation((state, teamId: string) =>
			state.declineTeamInvite(teamId),
		),
		addBot: useTeamMutation(
			(
				state,
				args: { teamId: string; botId: string; limits?: IBotRoomLimits | null },
			) => state.addBot(args.teamId, args.botId, args.limits),
		),
		removeBot: useTeamMutation(
			(state, args: { teamId: string; botId: string }) =>
				state.removeBot(args.teamId, args.botId),
		),
		acceptBotInvite: useTeamMutation(
			(
				state,
				args: { teamId: string; botId: string; limits?: IBotRoomLimits | null },
			) => state.acceptBotInvite(args.teamId, args.botId, args.limits),
		),
		declineBotInvite: useTeamMutation(
			(state, args: { teamId: string; botId: string }) =>
				state.declineBotInvite(args.teamId, args.botId),
		),
		setDirectAgreement: useTeamMutation(
			(state, args: { teamId: string; botId: string; agreed: boolean }) =>
				state.setDirectAgreement(args.teamId, args.botId, args.agreed),
		),
		acceptLead: useTeamMutation(
			(state, args: { teamId: string; botId: string }) =>
				state.acceptLead(args.teamId, args.botId),
		),
		declineLead: useTeamMutation(
			(state, args: { teamId: string; botId: string }) =>
				state.declineLead(args.teamId, args.botId),
		),
		resignLead: useTeamMutation(
			(state, args: { teamId: string; botId: string }) =>
				state.resignLead(args.teamId, args.botId),
		),
		openThread: useTeamMutation((state, teamId: string) =>
			state.openThread(teamId),
		),
		requestChat: useTeamMutation(
			(state, args: { botId: string; note?: string | null }) =>
				state.requestChat(args.botId, args.note),
		),
		acceptChat: useTeamMutation(
			(
				state,
				args: { botId: string; userId: string; limits?: IBotRoomLimits | null },
			) => state.acceptChat(args.botId, args.userId, args.limits),
		),
		declineChat: useTeamMutation(
			(state, args: { botId: string; userId: string }) =>
				state.declineChat(args.botId, args.userId),
		),
		endChat: useTeamMutation((state, args: { botId: string; userId: string }) =>
			state.endChat(args.botId, args.userId),
		),
		setClassification: useTeamMutation(
			(
				state,
				args: {
					orgId: string;
					userId: string;
					classification: DataClassification | null;
				},
			) =>
				state.setClassification(args.orgId, args.userId, args.classification),
		),
		pinApp: useTeamMutation((state, args: { teamId: string; appId: string }) =>
			state.pinApp(args.teamId, args.appId),
		),
		approveAppPin: useTeamMutation(
			(state, args: { teamId: string; appId: string }) =>
				state.approveAppPin(args.teamId, args.appId),
		),
		declineAppPin: useTeamMutation(
			(state, args: { teamId: string; appId: string }) =>
				state.declineAppPin(args.teamId, args.appId),
		),
		unpinApp: useTeamMutation(
			(state, args: { teamId: string; appId: string }) =>
				state.unpinApp(args.teamId, args.appId),
		),
	};
}
