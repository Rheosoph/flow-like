"use client";

import { useTranslation } from "@flow-like/locales";
import { MicIcon, MicOffIcon, PhoneIcon } from "lucide-react";
import { type ReactNode, useMemo } from "react";
import { toast } from "sonner";
import { useUpdateBot } from "../../hooks/use-bots";
import type { SpeechModelState } from "../../hooks/use-speech-model";
import { isHostedAudioProvider } from "../../lib/bit/hosted-audio";
import { STT_LOCAL_PROVIDER } from "../../lib/realtime/call-listening";
import { type IBit, IBitTypes } from "../../lib/schema/bit/bit";
import { cn } from "../../lib/utils";
import { useBackend } from "../../state/backend-state";
import type {
	IBot,
	IUpdateBotRequest,
} from "../../state/backend-state/bot-state";
import { Button } from "../ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "../ui/popover";
import { BotNotice, BotTag } from "./editor/bot-editor-parts";
import { BotModelPicker } from "./editor/steps/model-picker";
import {
	type ModelCatalogSpec,
	useBotBitOptions,
} from "./editor/steps/use-bot-models";

const TTS_LOCAL_PROVIDER = "local:any-tts";

/** Speech models the person's own client runs in a call: hosted audio, or a local model on desktop. */
function speechCatalog(
	type: IBitTypes,
	localProvider: string,
	desktop: boolean,
): ModelCatalogSpec {
	return {
		types: [type],
		accepts: (bit: IBit) => {
			const provider = bit.parameters?.provider?.provider_name;
			const name = typeof provider === "string" ? provider.trim() : "";
			return (
				isHostedAudioProvider(name) ||
				(desktop && name.toLowerCase() === localProvider)
			);
		},
	};
}

/**
 * The call button while the person can't be heard: greyed out, and opening why with the bot's
 * speech-to-text and voice pickers. Once a model resolves it offers to start the call.
 */
export function CallSpeechSetup({
	bot,
	name,
	speech,
	open,
	onOpenChange,
	canStart,
	onStart,
}: Readonly<{
	bot: IBot;
	name: string;
	speech: SpeechModelState;
	open: boolean;
	onOpenChange: (open: boolean) => void;
	/** The conversation's room is ready for a call. */
	canStart: boolean;
	onStart: () => void;
}>) {
	const { t } = useTranslation("bots");
	const ready = Boolean(speech.choice);
	const label = ready
		? t("crew.call.start", "Start a call")
		: t("crew.call.setup.title", "Calls need speech recognition");
	return (
		<Popover open={open} onOpenChange={onOpenChange}>
			<PopoverTrigger asChild>
				<Button
					type="button"
					variant="ghost"
					size="icon"
					data-slot="call-speech-setup"
					aria-disabled={!ready}
					className={cn(
						"shrink-0 text-muted-foreground",
						!ready && "opacity-50 hover:opacity-80",
					)}
					aria-label={label}
					title={label}
				>
					<PhoneIcon aria-hidden />
				</Button>
			</PopoverTrigger>
			<PopoverContent
				align="end"
				className="w-[min(22rem,calc(100vw-2rem))] p-0"
			>
				{speech.unsupported ? (
					<SetupHeader
						icon={<MicOffIcon className="size-4" aria-hidden />}
						title={t(
							"crew.call.setup.unsupportedTitle",
							"Calls aren't available here",
						)}
						body={t(
							"crew.call.setup.unsupportedBody",
							"This app can't turn speech into text, so {{name}} couldn't hear you.",
							{ name },
						)}
					/>
				) : (
					<>
						<SetupHeader
							ready={ready}
							icon={
								ready ? (
									<MicIcon className="size-4" aria-hidden />
								) : (
									<MicOffIcon className="size-4" aria-hidden />
								)
							}
							title={
								ready
									? t("crew.call.setup.readyTitle", "Ready to call {{name}}", {
											name,
										})
									: t("crew.call.setup.title", "Calls need speech recognition")
							}
							body={
								ready
									? t(
											"crew.call.setup.readyBody",
											"{{name}} can hear you now.",
											{ name },
										)
									: t(
											"crew.call.setup.body",
											"{{name}} hears you by turning your voice into text. Choose a speech-to-text model and you can call right away.",
											{ name },
										)
							}
						/>
						<SpeechModelFields bot={bot} name={name} speech={speech} />
						<div className="flex justify-end border-t px-4 py-3">
							<Button
								type="button"
								size="sm"
								disabled={!ready || !canStart}
								onClick={() => {
									onOpenChange(false);
									onStart();
								}}
							>
								<PhoneIcon aria-hidden />
								{t("crew.call.setup.start", "Start the call")}
							</Button>
						</div>
					</>
				)}
			</PopoverContent>
		</Popover>
	);
}

function SetupHeader({
	icon,
	title,
	body,
	ready = false,
}: Readonly<{
	icon: ReactNode;
	title: string;
	body: string;
	ready?: boolean;
}>) {
	return (
		<div className="flex items-start gap-3 p-4">
			<span
				className={cn(
					"grid size-9 shrink-0 place-items-center rounded-xl",
					ready
						? "bg-primary/10 text-primary"
						: "bg-muted text-muted-foreground",
				)}
			>
				{icon}
			</span>
			<div className="flex min-w-0 flex-col gap-1">
				<p className="text-sm font-medium">{title}</p>
				<p className="text-xs leading-relaxed text-muted-foreground">{body}</p>
			</div>
		</div>
	);
}

function SpeechModelFields({
	bot,
	name,
	speech,
}: Readonly<{ bot: IBot; name: string; speech: SpeechModelState }>) {
	const { t } = useTranslation("bots");
	const desktop = useBackend().capabilities?.().canExecuteLocally ?? false;
	const stt = useMemo(
		() => speechCatalog(IBitTypes.Stt, STT_LOCAL_PROVIDER, desktop),
		[desktop],
	);
	const tts = useMemo(
		() => speechCatalog(IBitTypes.Tts, TTS_LOCAL_PROVIDER, desktop),
		[desktop],
	);
	const sttOptions = useBotBitOptions(stt.types, stt.accepts).options;
	const ttsOptions = useBotBitOptions(tts.types, tts.accepts).options;
	const { mutateAsync: updateBot } = useUpdateBot();
	const save = (request: IUpdateBotRequest) =>
		updateBot({ botId: bot.id, request }).catch((cause: unknown) => {
			console.warn(`Saving the speech models of bot ${bot.id} failed`, cause);
			toast.error(
				t("crew.call.setup.saveFailed", "The model couldn't be saved"),
			);
		});
	const sttLabel = t("editor.brain.pins.stt");
	const voiceLabel = t("crew.call.setup.voice", "Voice");
	const placeholder = t("crew.call.setup.choose", "Choose a model");
	return (
		<div className="flex flex-col gap-4 px-4 pb-4">
			<div className="flex flex-col gap-1.5">
				<FieldLabel
					label={sttLabel}
					tag={
						speech.choice ? undefined : t("crew.call.setup.needed", "Needed")
					}
				/>
				<BotModelPicker
					label={sttLabel}
					value={bot.stt_model ?? null}
					options={sttOptions}
					catalog={stt}
					placeholder={placeholder}
					onChange={(id) => void save({ stt_model: id })}
				/>
				{speech.pinnedUnusable && !speech.choice && (
					<BotNotice tone="warning">
						{t(
							"crew.call.setup.pinnedUnusable",
							"This model can't run in this app. Choose another one.",
						)}
					</BotNotice>
				)}
			</div>
			<div className="flex flex-col gap-1.5">
				<FieldLabel
					label={voiceLabel}
					tag={t("crew.call.setup.optional", "Optional")}
				/>
				<BotModelPicker
					label={voiceLabel}
					value={bot.tts_model ?? null}
					options={ttsOptions}
					catalog={tts}
					placeholder={placeholder}
					onChange={(id) => void save({ tts_model: id })}
				/>
				<p className="text-xs leading-relaxed text-muted-foreground">
					{t(
						"crew.call.setup.voiceHint",
						"Without one, {{name}} uses your default voice or answers in the chat.",
						{ name },
					)}
				</p>
			</div>
		</div>
	);
}

function FieldLabel({ label, tag }: Readonly<{ label: string; tag?: string }>) {
	return (
		<div className="flex items-center justify-between gap-2">
			<span className="text-sm font-medium">{label}</span>
			{tag && <BotTag className="text-[0.625rem]">{tag}</BotTag>}
		</div>
	);
}
