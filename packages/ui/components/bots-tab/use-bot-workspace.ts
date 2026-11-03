"use client";

import { useQuery } from "@tanstack/react-query";
import { botKeys } from "../../hooks/use-bots";
import {
	ApiResponseError,
	apiErrorMessage,
	isMissingRouteError,
} from "../../lib/api-error";
import { getErrorMessage } from "../../lib/error-message";
import { isRecord } from "../../lib/response-shape";
import { useBackend } from "../../state/backend-state";
import type { IBotWorkspace } from "../../state/backend-state/bot-state";

/** Under `botKeys.all`, so a move or another bot-wide refresh asks again. */
export function botWorkspaceKey(botId: string) {
	return [...botKeys.all, "workspace", botId] as const;
}

export type WorkspaceFailure =
	| { kind: "unsupported" }
	| { kind: "refused"; message: string }
	| { kind: "failed"; message: string };

/** A hub without the route can't keep workspaces yet; a 409 is the hub saying no for this bot. */
export function workspaceFailure(error: unknown): WorkspaceFailure {
	if (isMissingRouteError(error)) return { kind: "unsupported" };
	if (error instanceof ApiResponseError && error.status === 409) {
		return { kind: "refused", message: error.serverMessage };
	}
	return {
		kind: "failed",
		message: apiErrorMessage(error, getErrorMessage(error)),
	};
}

function expectWorkspace(value: unknown, botId: string): IBotWorkspace {
	if (!isRecord(value) || typeof value.app_id !== "string" || !value.app_id) {
		throw new Error(`The workspace of bot ${botId} came back without an app`);
	}
	return { app_id: value.app_id };
}

/**
 * The bot's workspace app, set up the first time a pane needs it. Asked once per bot and
 * session; the host answers a repeat with the same app.
 */
export function useBotWorkspace(botId: string) {
	const state = useBackend().botState;
	const supported = typeof state?.getWorkspace === "function";
	const query = useQuery({
		queryKey: botWorkspaceKey(botId),
		queryFn: async () => {
			if (!state?.getWorkspace) {
				throw new Error(`This app can't open the workspace of bot ${botId}`);
			}
			return expectWorkspace(await state.getWorkspace(botId), botId);
		},
		enabled: supported,
		staleTime: Number.POSITIVE_INFINITY,
		gcTime: Number.POSITIVE_INFINITY,
		retry: false,
	});
	return { supported, query };
}
