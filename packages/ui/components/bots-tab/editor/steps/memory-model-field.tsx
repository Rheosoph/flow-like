"use client";

import { useTranslation } from "@flow-like/locales";
import { BotModelPicker } from "./model-picker";
import { useBotEmbeddingModelOptions } from "./use-bot-models";

/** The embedding model the bot remembers with; `null` leaves it to whatever runs the bot. */
export function BotMemoryModelField({
	value,
	onChange,
}: Readonly<{
	value: string | null;
	onChange: (embeddingModel: string | null) => void;
}>) {
	const { t } = useTranslation("bots");
	const { options, isLoading } = useBotEmbeddingModelOptions();
	const label = t("editor.brain.memoryModel");
	const none = !isLoading && options.length === 0;
	return (
		<div className="flex flex-col gap-1.5">
			<span className="text-sm font-medium">{label}</span>
			<BotModelPicker
				label={label}
				value={value}
				options={options}
				fallback={{
					label: t("editor.brain.memoryHost"),
					hint: t("editor.brain.memoryHostHint"),
				}}
				onChange={onChange}
			/>
			<p className="text-xs leading-relaxed text-muted-foreground">
				{none ? t("editor.brain.memoryNone") : t("editor.brain.memoryHint")}
			</p>
		</div>
	);
}
