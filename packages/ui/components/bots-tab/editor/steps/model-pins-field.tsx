"use client";

import { useTranslation } from "@flow-like/locales";
import { IBitTypes } from "../../../../lib/schema/bit/bit";
import type { BotModelPin } from "../bot-draft";
import { BotModelPicker } from "./model-picker";
import { useBotBitOptions } from "./use-bot-models";

const STT_TYPES: readonly IBitTypes[] = [IBitTypes.Stt];
const TTS_TYPES: readonly IBitTypes[] = [IBitTypes.Tts];
const IMAGE_TYPES: readonly IBitTypes[] = [IBitTypes.ImageGeneration];
const VISION_TYPES: readonly IBitTypes[] = [IBitTypes.Vlm];

type PinKind = "stt" | "tts" | "image" | "vision";

const PINS: readonly {
	pin: BotModelPin;
	kind: PinKind;
	types: readonly IBitTypes[];
}[] = [
	{ pin: "sttModel", kind: "stt", types: STT_TYPES },
	{ pin: "ttsModel", kind: "tts", types: TTS_TYPES },
	{ pin: "imageModel", kind: "image", types: IMAGE_TYPES },
	{ pin: "visionModel", kind: "vision", types: VISION_TYPES },
];

function BotModelPinField({
	kind,
	types,
	value,
	onChange,
}: Readonly<{
	kind: PinKind;
	types: readonly IBitTypes[];
	value: string | null;
	onChange: (bitId: string | null) => void;
}>) {
	const { t } = useTranslation("bots");
	const { options, isLoading } = useBotBitOptions(types);
	const label = t(`editor.brain.pins.${kind}`);
	const none = !isLoading && options.length === 0;
	return (
		<div className="flex flex-col gap-1.5">
			<span className="text-sm font-medium">{label}</span>
			<BotModelPicker
				label={label}
				value={value}
				options={options}
				fallback={{
					label: t("editor.brain.pins.host"),
					hint: t("editor.brain.pins.hostHint"),
				}}
				onChange={onChange}
			/>
			<p className="text-xs leading-relaxed text-muted-foreground">
				{none
					? t("editor.brain.pins.none")
					: t(`editor.brain.pins.${kind}Hint`)}
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
			{PINS.map(({ pin, kind, types }) => (
				<BotModelPinField
					key={pin}
					kind={kind}
					types={types}
					value={values[pin]}
					onChange={(bitId) => onChange(pin, bitId)}
				/>
			))}
		</div>
	);
}
