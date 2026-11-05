"use client";

import { useId } from "react";
import {
	HOSTED_AUDIO_PROVIDERS,
	validateHostedAudioParameters,
} from "../../lib/bit/hosted-audio";
import { IBitTypes } from "../../lib/schema/bit/bit";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import { record } from "./bit-editor-model";

export function HostedAudioConfiguration({
	parameters,
	bitType,
	onChange,
}: {
	parameters: Record<string, unknown>;
	bitType?: IBitTypes;
	onChange: (parameters: Record<string, unknown>) => void;
}) {
	const id = useId();
	const provider = record(parameters.provider);
	const settings = record(provider.params);
	const selected = HOSTED_AUDIO_PROVIDERS.find(
		(item) => item.value === provider.provider_name,
	);
	const error = validateHostedAudioParameters(parameters);
	const updateDefault = (
		key: "default_voice" | "default_language",
		value: string,
	) => {
		const next = { ...parameters };
		if (value) next[key] = value;
		else delete next[key];
		onChange(next);
	};
	return (
		<div className="space-y-4 rounded-xl border p-4">
			<div>
				<h3 className="font-medium">Hosted audio</h3>
				<p className="mt-1 text-sm text-muted-foreground">
					Use a speech model through the server. Choose a model that supports
					this Bit's speech task.
				</p>
			</div>
			<div className="grid gap-4 sm:grid-cols-2">
				<div className="space-y-2">
					<Label htmlFor={`${id}-provider`}>Provider</Label>
					<select
						id={`${id}-provider`}
						className="h-9 w-full rounded-md border bg-background px-3 text-sm"
						value={String(provider.provider_name ?? "")}
						onChange={(event) =>
							onChange({
								provider: {
									provider_name: event.target.value,
									model_id: "",
									params:
										typeof settings.tier === "string" && settings.tier.trim()
											? { tier: settings.tier }
											: {},
								},
							})
						}
					>
						{!selected && (
							<option value={String(provider.provider_name ?? "")}>
								Choose a provider
							</option>
						)}
						{HOSTED_AUDIO_PROVIDERS.map((item) => (
							<option key={item.value} value={item.value}>
								{item.label}
							</option>
						))}
					</select>
				</div>
				<div className="space-y-2">
					<Label htmlFor={`${id}-model`}>Model ID</Label>
					<Input
						id={`${id}-model`}
						maxLength={256}
						value={String(provider.model_id ?? "")}
						onChange={(event) =>
							onChange({
								...parameters,
								provider: { ...provider, model_id: event.target.value },
							})
						}
					/>
				</div>
				<div className="space-y-2">
					<Label htmlFor={`${id}-tier`}>Access tier</Label>
					<Input
						id={`${id}-tier`}
						placeholder="Server default (ENTERPRISE)"
						value={String(settings.tier ?? "")}
						onChange={(event) =>
							onChange({
								...parameters,
								provider: {
									...provider,
									params: event.target.value.trim()
										? { tier: event.target.value.trim() }
										: {},
								},
							})
						}
					/>
				</div>
				{(bitType === undefined || bitType === IBitTypes.Tts) && (
					<div className="space-y-2">
						<Label htmlFor={`${id}-voice`}>Default voice (optional)</Label>
						<Input
							id={`${id}-voice`}
							maxLength={64}
							placeholder="Provider voice ID, for example af_heart"
							value={String(parameters.default_voice ?? "")}
							onChange={(event) =>
								updateDefault("default_voice", event.target.value)
							}
						/>
						<p className="text-xs text-muted-foreground">
							Used when the speech request does not choose a voice.
						</p>
					</div>
				)}
				{(bitType === undefined || bitType === IBitTypes.Stt) && (
					<div className="space-y-2">
						<Label htmlFor={`${id}-language`}>
							Default language (optional)
						</Label>
						<Input
							id={`${id}-language`}
							maxLength={16}
							placeholder="Auto-detect, or a code such as en"
							value={String(parameters.default_language ?? "")}
							onChange={(event) =>
								updateDefault("default_language", event.target.value)
							}
						/>
						<p className="text-xs text-muted-foreground">
							Used when the transcription request does not choose a language.
						</p>
					</div>
				)}
			</div>
			{selected && (
				<p className="text-xs text-muted-foreground">
					Configure {selected.secret} on the server. Audio uses the server's
					speech tariff.
				</p>
			)}
			{error && (
				<p role="alert" className="text-sm text-destructive">
					{error}
				</p>
			)}
		</div>
	);
}
