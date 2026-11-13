"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback, useMemo } from "react";
import { ApiResponseError, apiErrorMessage } from "../../../lib/api-error";
import type {
	BotCapability,
	BotLocation,
} from "../../../state/backend-state/bot-state";
import { APP_REFUSES_BOTS, refusesBots } from "./app-options";
import type { BotDraftProblem, BotPresetTexts } from "./bot-draft";
import { BOT_SERVER_LIMIT_REACHED, serverLimitFact } from "./bot-limits";
import type { BotPreset } from "./bot-presets";

export interface BotCopy {
	label: string;
	description: string;
}

export function useBotCapabilityCopy() {
	const { t } = useTranslation("bots");
	return useCallback(
		(capability: BotCapability): BotCopy => ({
			label: t(`capabilities.${capability}.label`),
			description: t(`capabilities.${capability}.description`),
		}),
		[t],
	);
}

export function useBotLocationCopy() {
	const { t } = useTranslation("bots");
	return useCallback(
		(location: BotLocation): BotCopy => ({
			label: t(`locations.${location}.label`),
			description: t(`locations.${location}.description`),
		}),
		[t],
	);
}

export function useBotPresetTexts() {
	const { t } = useTranslation("bots");
	return useCallback(
		(preset: BotPreset): BotPresetTexts => ({
			name: t(preset.nameKey),
			description: t(preset.descriptionKey),
			instructions: t(preset.instructionsKey),
		}),
		[t],
	);
}

type BotProblemMessages = {
	[Code in BotDraftProblem["code"]]: (
		problem: Extract<BotDraftProblem, { code: Code }>,
	) => string;
};

export function useBotProblemMessage() {
	const { t } = useTranslation("bots");
	return useMemo(() => {
		const messages: BotProblemMessages = {
			name_required: () => t("editor.problems.nameRequired"),
			name_too_long: ({ max }) => t("editor.problems.nameTooLong", { max }),
			description_too_long: ({ max }) =>
				t("editor.problems.descriptionTooLong", { max }),
			instructions_too_long: ({ max }) =>
				t("editor.problems.instructionsTooLong", { max }),
			look_locked: ({ items }) =>
				t("editor.problems.lookLocked", { items: items.join(", ") }),
			look_invalid: ({ items }) =>
				t("editor.problems.lookInvalid", { items: items.join(", ") }),
			profile_required: () => t("editor.problems.profileRequired"),
			device_required: () => t("editor.problems.deviceRequired"),
			docker_requires_device: () =>
				t(
					"editor.problems.dockerRequiresDevice",
					"Choose a managed device for Docker hosting, or explicitly switch back to running directly on the host.",
				),
		};
		return (problem: BotDraftProblem): string =>
			(messages[problem.code] as (problem: BotDraftProblem) => string)(problem);
	}, [t]);
}

/** The plan's allowance of bots on the server in the hub's words, or ours when it said none. */
export function useServerLimitFact() {
	const { t } = useTranslation("bots");
	return useCallback(
		(serverMessage: string) =>
			serverLimitFact(serverMessage) ||
			t(
				"editor.errors.serverLimitFact",
				"Your plan's bots on the server are used up",
			),
		[t],
	);
}

/** Known refusals in plain words; anything else shows what the server said, or the fallback. */
export function useBotErrorMessage() {
	const { t } = useTranslation("bots");
	const limitFact = useServerLimitFact();
	return useMemo(() => {
		const messages: Readonly<
			Record<string, (serverMessage: string) => string>
		> = {
			BOT_CAPABILITY_LOCATION: () => t("editor.errors.capabilityLocation"),
			BOT_LIMIT_REACHED: () => t("editor.errors.limitReached"),
			[BOT_SERVER_LIMIT_REACHED]: (serverMessage) =>
				t(
					"editor.errors.serverLimitReached",
					"{{fact}}. Run it on this desktop or a device, delete one, or upgrade your plan.",
					{ fact: limitFact(serverMessage) },
				),
			BOT_SYSTEM_UNDELETABLE: () => t("editor.errors.systemUndeletable"),
			BOT_CAPABILITIES_FIXED: () => t("editor.errors.fluxCapabilities"),
			BOT_SECRET_LIMIT_REACHED: () => t("editor.errors.secretLimit"),
			BOT_APP_LIMIT_REACHED: () => t("editor.errors.appLimit"),
			[APP_REFUSES_BOTS]: (serverMessage) =>
				serverMessage || t("editor.errors.appRefusesBots"),
		};
		return (error: unknown, fallback: string): string => {
			const refusal = error instanceof ApiResponseError ? error : undefined;
			const code =
				refusal?.code ?? (refusesBots(error) ? APP_REFUSES_BOTS : undefined);
			return code && Object.hasOwn(messages, code)
				? messages[code](refusal?.serverMessage ?? "")
				: apiErrorMessage(error, fallback);
		};
	}, [limitFact, t]);
}
