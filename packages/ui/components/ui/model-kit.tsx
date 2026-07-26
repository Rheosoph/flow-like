"use client";

import {
	ArrowRightIcon,
	AudioLinesIcon,
	CloudIcon,
	Grid2X2Icon,
	HardDriveIcon,
	ImageIcon,
	type LucideIcon,
	MicIcon,
	TypeIcon,
	VideoIcon,
} from "lucide-react";
import { useMemo } from "react";
import { type IBit, IBitTypes } from "../../lib/schema/bit/bit";
import { cn } from "../../lib/utils";

export type Modality =
	| "text"
	| "image"
	| "audio"
	| "speech"
	| "embed"
	| "video";

interface ModalityInfo {
	label: string;
	icon: LucideIcon;
	/** CSS custom property holding this modality's hue. */
	color: string;
}

export const MODALITY: Record<Modality, ModalityInfo> = {
	text: { label: "Text", icon: TypeIcon, color: "var(--m-text)" },
	image: { label: "Image", icon: ImageIcon, color: "var(--m-image)" },
	audio: { label: "Audio", icon: MicIcon, color: "var(--m-audio)" },
	speech: { label: "Speech", icon: AudioLinesIcon, color: "var(--m-speech)" },
	embed: { label: "Embedding", icon: Grid2X2Icon, color: "var(--m-embed)" },
	video: { label: "Video", icon: VideoIcon, color: "var(--m-video)" },
};

export interface ModalityFlowSpec {
	inputs: Modality[];
	output: Modality;
}

/** What a model takes in and what it produces, derived from its bit type. */
export function bitModalities(type: IBitTypes): ModalityFlowSpec {
	switch (type) {
		case IBitTypes.Vlm:
			return { inputs: ["text", "image"], output: "text" };
		case IBitTypes.Stt:
			return { inputs: ["audio"], output: "text" };
		case IBitTypes.Tts:
			return { inputs: ["text"], output: "speech" };
		case IBitTypes.Embedding:
			return { inputs: ["text"], output: "embed" };
		case IBitTypes.ImageEmbedding:
			return { inputs: ["text", "image"], output: "embed" };
		case IBitTypes.ObjectDetection:
			return { inputs: ["image"], output: "text" };
		default:
			return { inputs: ["text"], output: "text" };
	}
}

export function ModalityToken({
	modality,
	compact = false,
	className,
}: Readonly<{
	modality: Modality;
	compact?: boolean;
	className?: string;
}>) {
	const info = MODALITY[modality];
	const Icon = info.icon;
	return (
		<span
			title={info.label}
			style={{ "--mc": info.color } as React.CSSProperties}
			className={cn(
				"inline-flex items-center gap-1.5 h-[22px] rounded-md text-[10.5px] font-semibold leading-none whitespace-nowrap",
				"text-[var(--mc)] bg-[color-mix(in_srgb,var(--mc)_14%,transparent)]",
				"border border-[color-mix(in_srgb,var(--mc)_26%,transparent)]",
				compact ? "w-[22px] justify-center px-0" : "px-2",
				className,
			)}
		>
			<Icon className="h-3 w-3 shrink-0" />
			{!compact && <span>{info.label}</span>}
		</span>
	);
}

/** inputs → output, the shape of the model at a glance. */
export function ModalityFlow({
	type,
	compact = false,
	plain = false,
	className,
}: Readonly<{
	type: IBitTypes;
	compact?: boolean;
	/** Icons and labels in running text — for dense lists where chips shout. */
	plain?: boolean;
	className?: string;
}>) {
	const flow = useMemo(() => bitModalities(type), [type]);

	if (plain) {
		return (
			<span className={cn("inline-flex items-center gap-1", className)}>
				{flow.inputs.map((modality, index) => {
					const info = MODALITY[modality];
					const Icon = info.icon;
					return (
						<span key={modality} className="inline-flex items-center gap-1">
							{index > 0 && <span aria-hidden="true">+</span>}
							<Icon className="h-3 w-3 shrink-0" />
							{info.label}
						</span>
					);
				})}
				<ArrowRightIcon className="h-3 w-3 shrink-0 opacity-60" />
				<span className="inline-flex items-center gap-1">
					{(() => {
						const Icon = MODALITY[flow.output].icon;
						return <Icon className="h-3 w-3 shrink-0" />;
					})()}
					{MODALITY[flow.output].label}
				</span>
			</span>
		);
	}

	return (
		<span
			className={cn("inline-flex items-center gap-1.5 flex-wrap", className)}
		>
			<span className="inline-flex items-center gap-1">
				{flow.inputs.map((modality) => (
					<ModalityToken key={modality} modality={modality} compact={compact} />
				))}
			</span>
			<ArrowRightIcon className="h-3.5 w-3.5 shrink-0 text-muted-foreground/50" />
			<ModalityToken modality={flow.output} compact={compact} />
		</span>
	);
}

export type Deployment = "hosted" | "local" | "remote";

const DEPLOYMENT: Record<
	Deployment,
	{ label: string; icon: LucideIcon; color: string }
> = {
	hosted: { label: "Hosted", icon: CloudIcon, color: "var(--dep-hosted)" },
	local: { label: "On-device", icon: HardDriveIcon, color: "var(--dep-local)" },
	remote: { label: "Remote", icon: CloudIcon, color: "var(--dep-remote)" },
};

export function DeploymentBadge({
	kind,
	className,
}: Readonly<{ kind: Deployment; className?: string }>) {
	const info = DEPLOYMENT[kind];
	const Icon = info.icon;
	return (
		<span
			style={{ "--dc": info.color } as React.CSSProperties}
			className={cn(
				"inline-flex items-center gap-1.5 h-[22px] px-2 rounded-md text-[10.5px] font-semibold leading-none",
				"text-[var(--dc)] bg-[color-mix(in_srgb,var(--dc)_10%,transparent)]",
				"border border-[color-mix(in_srgb,var(--dc)_32%,transparent)]",
				className,
			)}
		>
			<Icon className="h-3 w-3 shrink-0" />
			{info.label}
		</span>
	);
}

/** Where a model runs, as a coloured dot plus running text. */
export function DeploymentLabel({
	kind,
	className,
}: Readonly<{ kind: Deployment; className?: string }>) {
	const info = DEPLOYMENT[kind];
	return (
		<span
			style={{ "--dc": info.color } as React.CSSProperties}
			className={cn("inline-flex items-center gap-1.5", className)}
		>
			<span
				aria-hidden="true"
				className="h-1.5 w-1.5 shrink-0 rounded-full bg-(--dc)"
			/>
			{info.label}
		</span>
	);
}

/** Stable hue per provider so the same provider always reads the same colour. */
function providerHue(seed: string): number {
	let hash = 0;
	for (let i = 0; i < seed.length; i++) {
		hash = (hash * 31 + seed.charCodeAt(i)) % 360;
	}
	return hash;
}

function providerMonogram(name: string): string {
	const cleaned = name.replace(/^(custom|hosted|local):/i, "").trim();
	const words = cleaned.split(/[\s\-_/.]+/).filter(Boolean);
	if (words.length === 0) return "??";
	if (words.length === 1) return words[0].slice(0, 2);
	return (words[0][0] + words[1][0]).toUpperCase();
}

/**
 * Provider identity tile: the model's own icon when it has one, otherwise a
 * monogram on a stable per-provider hue so a rail of cards stays scannable.
 */
export function ProviderGlyph({
	bit,
	size = 40,
	className,
}: Readonly<{ bit: IBit; size?: number; className?: string }>) {
	const icon = bit.meta?.en?.icon;
	const providerName = useMemo(() => {
		const params = bit.parameters as
			| { provider?: { provider_name?: string } }
			| undefined;
		return params?.provider?.provider_name ?? bit.hub ?? bit.id;
	}, [bit.parameters, bit.hub, bit.id]);

	const style = {
		width: size,
		height: size,
		fontSize: Math.round(size * 0.36),
	} as React.CSSProperties;

	if (icon) {
		return (
			<span
				style={style}
				className={cn(
					"grid shrink-0 place-items-center overflow-hidden rounded-[10px] border border-border/60 bg-muted",
					className,
				)}
			>
				<img
					src={icon}
					alt=""
					className="h-full w-full object-cover"
					loading="lazy"
				/>
			</span>
		);
	}

	const hue = providerHue(providerName);
	return (
		<span
			aria-hidden="true"
			style={{
				...style,
				background: `linear-gradient(150deg, hsl(${hue} 58% 52%), hsl(${(hue + 24) % 360} 62% 42%))`,
			}}
			className={cn(
				"grid shrink-0 place-items-center rounded-[10px] font-bold tracking-tight text-white",
				"shadow-[inset_0_1px_0_rgba(255,255,255,.25),0_2px_6px_rgba(0,0,0,.18)]",
				className,
			)}
		>
			{providerMonogram(providerName)}
		</span>
	);
}

/** Providers write their own names — title-casing them reads as a typo. */
const PROVIDER_NAMES: Record<string, string> = {
	anthropic: "Anthropic",
	azure: "Azure OpenAI",
	bedrock: "Amazon Bedrock",
	cohere: "Cohere",
	deepseek: "DeepSeek",
	galadriel: "Galadriel",
	gemini: "Google Gemini",
	groq: "Groq",
	huggingface: "Hugging Face",
	hyperbolic: "Hyperbolic",
	lmstudio: "LM Studio",
	local: "On-device",
	mira: "Mira",
	mistral: "Mistral",
	moonshot: "Moonshot",
	mozilla: "Mozilla",
	ollama: "Ollama",
	openai: "OpenAI",
	openrouter: "OpenRouter",
	perplexity: "Perplexity",
	together: "Together AI",
	vertex: "Vertex AI",
	voyageai: "Voyage AI",
	xai: "xAI",
};

/** Human-readable provider label for the byline under a model name. */
export function providerLabel(bit: IBit): string {
	const params = bit.parameters as
		| { provider?: { provider_name?: string } }
		| undefined;
	const raw = params?.provider?.provider_name;
	if (!raw) return "Unknown provider";
	const cleaned = raw
		.replace(/^custom:/i, "")
		.replace(/^hosted:?/i, "")
		.trim();
	if (!cleaned) return "Hosted";
	return (
		PROVIDER_NAMES[cleaned.toLowerCase()] ??
		cleaned.charAt(0).toUpperCase() + cleaned.slice(1)
	);
}
