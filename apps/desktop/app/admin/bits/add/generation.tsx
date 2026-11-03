import {
	Badge,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	GENERATION_ASSET_LABELS,
	GENERATION_MODEL_PRESETS,
	type GenerationAssetDraft,
	type GenerationModelKind,
	type IBit,
	Label,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	applyGenerationModelPreset,
	createGenerationAssetDrafts,
	defaultGenerationPreset,
	humanFileSize,
	validateGenerationAssets,
} from "@flow-like/flow-like-ui";
import { HostedImageGenerationConfiguration } from "@flow-like/flow-like-ui/components/bits/hosted-image-generation-configuration";
import {
	createHostedImageBit,
	isHostedImageGeneration,
	updateHostedImageBit,
} from "@flow-like/flow-like-ui/lib/bit/hosted-image-generation";
import { type Dispatch, type SetStateAction, useId } from "react";
import { DependencyConfiguration } from "./dependency";

export function GenerationConfiguration({
	bit,
	setBit,
	kind,
	assets,
	setAssets,
	createAssetBit,
	disabled = false,
}: {
	bit: IBit;
	setBit: Dispatch<SetStateAction<IBit>>;
	kind: GenerationModelKind;
	assets: GenerationAssetDraft[];
	setAssets: Dispatch<SetStateAction<GenerationAssetDraft[]>>;
	createAssetBit: () => IBit;
	disabled?: boolean;
}) {
	const modelId = useId();
	const hosted = isHostedImageGeneration(bit);
	const presets = GENERATION_MODEL_PRESETS.filter(
		(preset) => preset.kind === kind,
	);
	const selected =
		presets.find(
			(preset) => preset.id === bit.parameters?.provider?.model_id,
		) ?? defaultGenerationPreset(kind);
	const totalSize = assets.reduce(
		(total, asset) => total + Math.max(0, asset.bit.size ?? 0),
		0,
	);
	const errors = hosted ? [] : validateGenerationAssets(assets);

	return (
		<fieldset
			disabled={disabled}
			className="space-y-6 w-full max-w-screen-lg min-w-0"
		>
			{kind === "image" && (
				<div className="space-y-2">
					<Label htmlFor={`${modelId}-execution`}>Run images</Label>
					<select
						id={`${modelId}-execution`}
						className="h-9 w-full rounded-md border bg-background px-3 text-sm"
						value={hosted ? "hosted" : "local"}
						onChange={(event) => {
							if (event.target.value === "hosted") {
								setAssets([]);
								setBit((current) => createHostedImageBit(current));
							} else {
								const preset = defaultGenerationPreset("image");
								const drafts = createGenerationAssetDrafts(
									preset.id,
									createAssetBit,
								);
								setAssets(drafts);
								setBit((current) =>
									applyGenerationModelPreset(current, preset.id, drafts),
								);
							}
						}}
					>
						<option value="local">Locally with downloaded weights</option>
						<option value="hosted">Hosted provider</option>
					</select>
				</div>
			)}
			{hosted ? (
				<HostedImageGenerationConfiguration
					parameters={bit.parameters ?? {}}
					onChange={(parameters) =>
						setBit((current) => updateHostedImageBit(current, parameters))
					}
				/>
			) : (
				<Card>
					<CardHeader>
						<CardTitle>
							{kind === "image"
								? "Image Generation Model"
								: "Video Generation Model"}
						</CardTitle>
						<CardDescription>{selected.description}</CardDescription>
					</CardHeader>
					<CardContent className="space-y-4">
						<div className="space-y-2">
							<Label htmlFor={modelId}>Model preset</Label>
							<Select
								value={selected.id}
								disabled={disabled}
								onValueChange={(presetId) => {
									const drafts = createGenerationAssetDrafts(
										presetId,
										createAssetBit,
									);
									setAssets(drafts);
									setBit((current) =>
										applyGenerationModelPreset(current, presetId, drafts),
									);
								}}
							>
								<SelectTrigger id={modelId}>
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									{presets.map((preset) => (
										<SelectItem key={preset.id} value={preset.id}>
											{preset.label}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
						</div>
						<div className="flex flex-wrap items-center gap-2 text-sm">
							<Badge variant="secondary">{selected.license}</Badge>
							<span>
								{assets.length} files, {humanFileSize(totalSize)} total
							</span>
							<a
								href={selected.repository}
								target="_blank"
								rel="noreferrer"
								className="underline underline-offset-4"
							>
								Model source
							</a>
						</div>
						<p className="text-sm text-muted-foreground whitespace-pre-line">
							{selected.notes}
						</p>
						<p className="text-sm text-muted-foreground">
							Review the model files below and the metadata before registering
							the model.
						</p>
						{errors.length > 0 && (
							<ul
								className="list-disc pl-5 text-sm text-destructive"
								aria-live="polite"
							>
								{errors.map((error) => (
									<li key={error}>{error}</li>
								))}
							</ul>
						)}
					</CardContent>
				</Card>
			)}
			{!hosted &&
				assets.map((asset) => (
					<DependencyConfiguration
						key={asset.key}
						name={GENERATION_ASSET_LABELS[asset.role]}
						defaultBit={asset.bit}
						bit={asset.bit}
						setBit={(update) =>
							setAssets((current) =>
								current.map((candidate) => {
									if (candidate.key !== asset.key) return candidate;
									const nextBit =
										typeof update === "function"
											? update(candidate.bit)
											: update;
									return nextBit ? { ...candidate, bit: nextBit } : candidate;
								}),
							)
						}
					/>
				))}
		</fieldset>
	);
}
