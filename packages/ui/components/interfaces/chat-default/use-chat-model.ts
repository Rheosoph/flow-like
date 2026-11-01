"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useInvoke } from "../../../hooks/use-invoke";
import { IEventExecutionMode } from "../../../lib/schema/flow/event";
import type { IEventPayloadChat } from "../../../lib/schema/flow/event-payload-chat";
import { useBackend } from "../../../state/backend-state";
import {
	chatModelOptions,
	readChatModel,
	storeChatModel,
} from "./chat-model-selection";
import type { ChatBoxModel } from "./model-picker";

/** The user's model choice belongs to the app, so every chat event shares it. */
export function useChatModel(
	appId: string,
	config?: Partial<IEventPayloadChat>,
	executionMode?: IEventExecutionMode,
) {
	const { t, i18n } = useTranslation("chat");
	const backend = useBackend();
	const enabled = config?.allow_model_selection !== false;
	const profile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
		enabled,
	);
	const profileBits = useInvoke(
		backend.bitState.getProfileBits,
		backend.bitState,
		[],
		enabled && !!profile.data,
		[profile.data?.hub_profile.id],
	);
	const capabilities = backend.capabilities();
	const remote = executionMode === IEventExecutionMode.Remote;
	const canHostLlamaCPP = !remote && capabilities.canHostLlamaCPP;
	const canHostMLX = !remote && capabilities.canHostMLX;
	const canUseNativeAgentProviders =
		!remote && capabilities.canUseNativeAgentProviders;
	const deviceModels = !remote && capabilities.deviceModels;
	const requireVlm = config?.require_vlm === true;
	const models = useMemo(
		() =>
			chatModelOptions(
				profileBits.data,
				profile.data?.hub_profile.bits,
				{
					canHostLlamaCPP,
					canHostMLX,
					canUseNativeAgentProviders,
					deviceModels,
				},
				requireVlm,
			),
		[
			profileBits.data,
			profile.data?.hub_profile.bits,
			canHostLlamaCPP,
			canHostMLX,
			canUseNativeAgentProviders,
			deviceModels,
			requireVlm,
		],
	);
	const [picked, setPicked] = useState<{
		appId: string;
		modelId: string | undefined;
	}>();
	const revision = useRef(0);

	useEffect(() => {
		const pendingRevision = ++revision.current;
		void readChatModel(appId).then((modelId) => {
			if (revision.current === pendingRevision) setPicked({ appId, modelId });
		});
		return () => {
			revision.current++;
		};
	}, [appId]);

	const selectedModel =
		enabled &&
		picked?.appId === appId &&
		models.some((model) => model.id === picked.modelId)
			? picked.modelId
			: undefined;
	const onChange = useCallback(
		(modelId: string) => {
			if (
				!enabled ||
				(modelId && !models.some((model) => model.id === modelId))
			)
				return;
			revision.current++;
			setPicked({ appId, modelId: modelId || undefined });
			void storeChatModel(appId, modelId || undefined);
		},
		[appId, enabled, models],
	);
	const language = i18n.language;
	const options = useMemo(
		() => [
			{ id: "", label: t("automaticModel", "Automatic") },
			...models.map((model) => ({
				id: model.id,
				label: model.meta?.[language]?.name ?? model.meta?.en?.name ?? model.id,
			})),
		],
		[models, t, language],
	);
	const picker = useMemo<ChatBoxModel | undefined>(
		() =>
			enabled ? { value: selectedModel ?? "", options, onChange } : undefined,
		[enabled, selectedModel, options, onChange],
	);

	return { selectedModel, picker };
}
