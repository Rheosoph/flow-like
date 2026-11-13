"use client";

import { useTranslation } from "@flow-like/locales";
import { IBitTypes } from "../../../../lib/schema/bit/bit";
import type { BotModelPin } from "../bot-draft";
import { BotModelPicker } from "./model-picker";
import {
	type ModelCatalogSpec,
	isImageCapableBit,
	useBotBitOptions,
} from "./use-bot-models";

type PinKind = "stt" | "tts" | "image" | "vision" | "subtask";

const PINS: readonly {
	pin: BotModelPin;
	kind: PinKind;
	catalog: ModelCatalogSpec;
}[] = [
	{ pin: "sttModel", kind: "stt", catalog: { types: [IBitTypes.Stt] } },
	{ pin: "ttsModel", kind: "tts", catalog: { types: [IBitTypes.Tts] } },
	{
		pin: "imageModel",
		kind: "image",
		catalog: {
			types: [IBitTypes.ImageGeneration, IBitTypes.Vlm],
			accepts: isImageCapableBit,
		},
	},
	{ pin: "visionModel", kind: "vision", catalog: { types: [IBitTypes.Vlm] } },
	{
		pin: "subtaskModel",
		kind: "subtask",
		catalog: { types: [IBitTypes.Llm, IBitTypes.Vlm] },
	},
];

function BotModelPinField({
	kind,
	catalog,
	value,
	onChange,
}: Readonly<{
	kind: PinKind;
	catalog: ModelCatalogSpec;
	value: string | null;
	onChange: (bitId: string | null) => void;
}>) {
	const { t } = useTranslation("bots");
	const { options } = useBotBitOptions(catalog.types, catalog.accepts);
	const label = t(`editor.brain.pins.${kind}`);
	return (
		<div className="flex flex-col gap-1.5">
			<span className="text-sm font-medium">{label}</span>
			<BotModelPicker
				label={label}
				value={value}
				options={options}
				catalog={catalog}
				fallback={
					kind === "subtask"
						? {
								label: t("editor.brain.pins.ownModel"),
								hint: t("editor.brain.pins.ownModelHint"),
							}
						: {
								label: t("editor.brain.pins.host"),
								hint: t("editor.brain.pins.hostHint"),
							}
				}
				onChange={onChange}
			/>
			<p className="text-xs leading-relaxed text-muted-foreground">
				{t(`editor.brain.pins.${kind}Hint`)}
			</p>
		</div>
	);
}

/** The speech, image and vision models the bot always uses; `null` leaves each to whatever runs the bot. */
export function BotModelPinsField({
	values,
	onChange,
}: Readonly<{
	values: Readonly<Record<BotModelPin, string | null>>;
	onChange: (pin: BotModelPin, bitId: string | null) => void;
}>) {
	const { t } = useTranslation("bots");
	return (
		<div className="flex flex-col gap-4">
			<div className="flex flex-col gap-1">
				<span className="text-sm font-medium">
					{t("editor.brain.pins.title")}
				</span>
				<p className="text-xs leading-relaxed text-muted-foreground">
					{t("editor.brain.pins.hint")}
				</p>
			</div>
			{PINS.map(({ pin, kind, catalog }) => (
				<BotModelPinField
					key={pin}
					kind={kind}
					catalog={catalog}
					value={values[pin]}
					onChange={(bitId) => onChange(pin, bitId)}
				/>
			))}
		</div>
	);
}
