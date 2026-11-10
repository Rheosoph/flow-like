"use client";

import { useTranslation } from "@flow-like/locales";
import { BotModelPicker } from "./model-picker";
import {
	EMBEDDING_CATALOG,
	useBotEmbeddingModelOptions,
} from "./use-bot-models";

/** The embedding model the bot remembers with; `null` leaves it to whatever runs the bot. */
export function BotMemoryModelField({
	value,
	onChange,
}: Readonly<{
	value: string | null;
	onChange: (embeddingModel: string | null) => void;
}>) {
	const { t } = useTranslation("bots");
	const { options } = useBotEmbeddingModelOptions();
	const label = t("editor.brain.memoryModel");
	return (
		<div className="flex flex-col gap-1.5">
			<span className="text-sm font-medium">{label}</span>
			<BotModelPicker
				label={label}
				value={value}
				options={options}
				catalog={EMBEDDING_CATALOG}
				fallback={{
					label: t("editor.brain.memoryHost"),
					hint: t("editor.brain.memoryHostHint"),
				}}
				onChange={onChange}
			/>
			<p className="text-xs leading-relaxed text-muted-foreground">
				{t("editor.brain.memoryHint")}
			</p>
		</div>
	);
}
