"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback } from "react";
import { formatRelativeTime } from "../../lib/date";
import { type BotHostState, botHostState } from "../../lib/device-bot-host";

export interface BotHostStatus {
	state: BotHostState;
	label: string;
}

/** Whether a device's agent is hosting bots, in words: ready, last seen, or not yet seen. */
export function useBotHostStatus() {
	const { t } = useTranslation("bots");
	return useCallback(
		(seenAt: number | null | undefined, now?: number): BotHostStatus => {
			const state = botHostState(seenAt, now);
			if (state === "ready")
				return { state, label: t("deviceHost.ready", "Ready for bots") };
			if (state === "offline")
				return {
					state,
					label: t(
						"deviceHost.offline",
						"Bot host offline — last seen {{when}}",
						{
							when: formatRelativeTime(
								seenAt,
								"long",
								"",
								now === undefined ? {} : { now },
							),
						},
					),
				};
			return {
				state,
				label: t("deviceHost.never", "Update or start the agent to host bots"),
			};
		},
		[t],
	);
}
