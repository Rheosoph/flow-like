"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback, useMemo } from "react";
import { type IBit, IBitTypes } from "../../../../lib/schema/bit/bit";
import type { BotLocation } from "../../../../state/backend-state/bot-state";
import { isBotDecisionModel } from "./decision-model-options";
import { BotModelPicker } from "./model-picker";
import { type ModelCatalogSpec, useBotBitOptions } from "./use-bot-models";

const TYPES: readonly IBitTypes[] = [IBitTypes.SystemOne];

export function BotDecisionModelField({
	value,
	location,
	onChange,
}: Readonly<{
	value: string | null;
	location: BotLocation;
	onChange: (value: string | null) => void;
}>) {
	const { t } = useTranslation("bots");
	const accepts = useCallback(
		(bit: IBit) => isBotDecisionModel(bit, location),
		[location],
	);
	const { options } = useBotBitOptions(TYPES, accepts);
	const catalog = useMemo<ModelCatalogSpec>(
		() => ({ types: TYPES, accepts }),
		[accepts],
	);
	const label = t("editor.brain.decision", "Decision model (System One)");
	return (
		<div className="flex flex-col gap-1.5">
			<span className="text-sm font-medium">{label}</span>
			<BotModelPicker
				label={label}
				value={value}
				options={options}
				catalog={catalog}
				fallback={{
					label: t("editor.brain.decisionDisabled", "Disabled"),
					hint: t(
						"editor.brain.decisionDisabledHint",
						"Use the usual response rules.",
					),
				}}
				onChange={onChange}
			/>
			<p className="text-xs leading-relaxed text-muted-foreground">
				{t(
					"editor.brain.decisionHint",
					"Decides when to answer or briefly acknowledge a call, wait for a participant, select relevant skills, filter background updates, and curate memory.",
				)}
				{location !== "desktop" &&
					` ${t("editor.brain.decisionHosted", "Server and managed-device bots use hosted decision models.")}`}
			</p>
		</div>
	);
}
