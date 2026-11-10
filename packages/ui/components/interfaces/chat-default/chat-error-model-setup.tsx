"use client";

import { useTranslation } from "@flow-like/locales";
import CheckIcon from "lucide-react/dist/esm/icons/circle-check.js";
import LoaderIcon from "lucide-react/dist/esm/icons/loader-circle.js";
import { useCallback, useMemo, useState } from "react";
import { useAddModelToProfile } from "../../../hooks/use-add-model-to-profile";
import {
	type LocalModelHostCapabilities,
	isCloudRunnableLlmModel,
	isHostableLlmModel,
} from "../../../lib/bit/local-model-filter";
import type { IBit } from "../../../lib/schema/bit/bit";
import { useBackend } from "../../../state/backend-state";
import { BotModelPicker } from "../../bots-tab/editor/steps/model-picker";
import {
	LLM_CATALOG,
	type ModelCatalogSpec,
	useBotBitOptions,
} from "../../bots-tab/editor/steps/use-bot-models";

interface ModelPick {
	id: string;
	state: "adding" | "added";
}

type SetupState = "needed" | "adding" | "ready";

/** What this app can run itself, for an answer that does not come from the cloud. */
function useHostCapabilities(): LocalModelHostCapabilities {
	const capabilities = useBackend().capabilities?.();
	const canHostLlamaCPP = capabilities?.canHostLlamaCPP === true;
	const canHostMLX = capabilities?.canHostMLX === true;
	const canUseNativeAgentProviders =
		capabilities?.canUseNativeAgentProviders === true;
	const deviceModels = capabilities?.deviceModels === true;
	return useMemo(() => {
		return {
			canHostLlamaCPP,
			canHostMLX,
			canUseNativeAgentProviders,
			deviceModels,
		};
	}, [canHostLlamaCPP, canHostMLX, canUseNativeAgentProviders, deviceModels]);
}

function setupState(pick: ModelPick | null, usable: number): SetupState {
	if (pick?.state === "adding") return "adding";
	return pick || usable > 0 ? "ready" : "needed";
}

/**
 * The way out of a turn nothing in the profile could answer: the model picker, in the card. A pick
 * is added to the profile; the person then sends the message again. The picker stays available
 * after a model fits, since only the answering host knows what it can run.
 */
export function ChatErrorModelSetup({
	hostedOnly,
}: Readonly<{
	/** The answer runs in the cloud, so models on the person's devices are left out. */
	hostedOnly: boolean;
}>) {
	const { t } = useTranslation("chat");
	const host = useHostCapabilities();
	const { gate } = useAddModelToProfile();
	const [pick, setPick] = useState<ModelPick | null>(null);

	const usable = useCallback(
		(bit: IBit) => {
			const fits = hostedOnly
				? isCloudRunnableLlmModel(bit)
				: isHostableLlmModel(bit, host);
			return fits && !gate(bit).isRestricted;
		},
		[hostedOnly, host, gate],
	);
	const catalog = useMemo<ModelCatalogSpec>(() => {
		return {
			types: LLM_CATALOG.types,
			accepts: hostedOnly ? isCloudRunnableLlmModel : undefined,
		};
	}, [hostedOnly]);
	const { options } = useBotBitOptions(LLM_CATALOG.types, usable);
	const inProfile = useMemo(() => {
		const ids = new Set<string>();
		for (const option of options) ids.add(option.id);
		return ids;
	}, [options]);

	const choose = (id: string | null) => {
		const state = id && inProfile.has(id) ? "added" : "adding";
		setPick(id ? { id, state } : null);
	};
	const settle = useCallback((id: string, added: boolean) => {
		setPick((current) => {
			if (current?.id !== id) return current;
			return added ? { id, state: "added" } : null;
		});
	}, []);

	const state = setupState(pick, options.length);
	const placeholder =
		options.length > 0
			? t("modelSetupChooseAnother", "Choose another model")
			: t("modelSetupChoose", "Choose a model");

	return (
		<div
			className="flex flex-col gap-1.5 pt-1.5"
			data-fl-chat-error-model-setup={state}
		>
			<div className="max-w-[26rem]">
				<BotModelPicker
					label={t("modelSetupLabel", "Language model")}
					value={pick?.id ?? null}
					options={options}
					catalog={catalog}
					placeholder={placeholder}
					onChange={choose}
					onAddSettled={settle}
				/>
			</div>
			{state === "adding" && (
				<p className="flex items-center gap-1.5 text-xs text-muted-foreground">
					<LoaderIcon className="size-3.5 shrink-0 animate-spin" aria-hidden />
					{t("modelSetupAdding", "Adding it to your models…")}
				</p>
			)}
			{state === "ready" && (
				<p className="flex items-start gap-1.5 text-xs text-muted-foreground">
					<CheckIcon
						className="mt-px size-3.5 shrink-0 text-good"
						aria-hidden
					/>
					{t(
						"modelSetupReady",
						"Your profile has a language model now. Send your message again.",
					)}
				</p>
			)}
		</div>
	);
}
