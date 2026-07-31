"use client";

import { createId } from "@paralleldrive/cuid2";
import {
	ArrowLeft,
	Bot,
	ChevronDown,
	Eye,
	HardDriveDownload,
	Loader2,
	Plug,
	ScanEye,
	ScanSearch,
	SlidersHorizontal,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { useInvalidateInvoke } from "../../../hooks";
import {
	type HuggingFaceGgufRepositoryImport,
	type HuggingFaceModelImport,
	applyHuggingFaceMlxImportToUserBit,
	createHuggingFaceUserMlxManifest,
	inspectHuggingFaceModelRepository,
	resolveHuggingFaceGgufSelection,
	validateHuggingFacePinnedGgufDownloadUrl,
} from "../../../lib/bit/huggingface-model-import";
import type { IBit, IMetadata } from "../../../lib/schema/bit/bit";
import { IBitTypes } from "../../../lib/schema/bit/bit";
import type { ILlmParameters } from "../../../lib/schema/bit/bit/llm-parameters";
import { humanFileSize } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import {
	Avatar,
	AvatarFallback,
	AvatarImage,
	Badge,
	Button,
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	Input,
	Label,
	Slider,
	Switch,
	Textarea,
	formatContextLength,
} from "../../ui";

const LOCAL_PROVIDER_NAME = "Local";
const MLX_PROVIDER_NAME = "MLX";
const DEFAULT_CONTEXT_LENGTH = "128000";

interface IProviderField {
	key: string;
	label: string;
	placeholder?: string;
	/** Pre-filled value when the provider is picked (still editable). */
	defaultValue?: string;
	description?: string;
	secret?: boolean;
	required?: boolean;
	multiline?: boolean;
	advanced?: boolean;
}

interface IProviderDef {
	key: string;
	providerName: string;
	label: string;
	description: string;
	primary: boolean;
	isAzure?: boolean;
	fields: IProviderField[];
	validate?: (values: Record<string, string>, isEdit: boolean) => string | null;
}

const apiKeyField = (placeholder: string): IProviderField => ({
	key: "api_key",
	label: "API key",
	placeholder,
	secret: true,
	required: true,
});

const modelIdField = (
	placeholder: string,
	label = "Model ID",
): IProviderField => ({
	key: "model_id",
	label,
	placeholder,
	required: true,
});

const endpointField = (
	placeholder: string,
	advanced = true,
): IProviderField => ({
	key: "endpoint",
	label: "Endpoint",
	placeholder,
	advanced,
	description: advanced ? "Override the default API endpoint" : undefined,
});

const PROVIDERS: IProviderDef[] = [
	{
		key: "openai",
		providerName: "custom:openai",
		label: "OpenAI",
		description: "GPT models from OpenAI",
		primary: true,
		fields: [
			apiKeyField("sk-…"),
			modelIdField("gpt-4o"),
			endpointField("https://api.openai.com/v1"),
		],
	},
	{
		key: "azure-openai",
		providerName: "custom:openai",
		label: "Azure OpenAI",
		description: "OpenAI deployments on Microsoft Azure",
		primary: true,
		isAzure: true,
		fields: [
			apiKeyField("Azure API key"),
			{
				key: "endpoint",
				label: "Endpoint",
				placeholder: "https://<resource>.openai.azure.com",
				required: true,
			},
			modelIdField("my-gpt-4o-deployment", "Deployment name"),
			{
				key: "version",
				label: "API version",
				placeholder: "2024-10-21",
			},
		],
	},
	{
		key: "bedrock",
		providerName: "custom:bedrock",
		label: "AWS Bedrock",
		description: "Bedrock models via API key (OpenAI-compatible endpoint)",
		primary: true,
		fields: [
			apiKeyField("Bedrock API key"),
			modelIdField("openai.gpt-oss-120b"),
			{
				key: "region",
				label: "Region",
				placeholder: "us-east-1",
				defaultValue: "us-east-1",
				description: "AWS region of the Bedrock runtime",
			},
			{
				key: "endpoint",
				label: "Endpoint",
				placeholder: "https://bedrock-runtime.us-east-1.amazonaws.com",
				advanced: true,
				description: "Override the region-derived Bedrock runtime endpoint",
			},
		],
	},
	{
		key: "anthropic",
		providerName: "custom:anthropic",
		label: "Anthropic",
		description: "Claude models from Anthropic",
		primary: true,
		fields: [
			apiKeyField("sk-ant-…"),
			modelIdField("claude-sonnet-4-5"),
			endpointField("https://api.anthropic.com"),
			{
				key: "version",
				label: "API version",
				placeholder: "2023-06-01",
				advanced: true,
			},
			{
				key: "beta",
				label: "Beta features",
				placeholder: "prompt-caching-2024-07-31",
				advanced: true,
			},
		],
	},
	{
		key: "gemini",
		providerName: "custom:gemini",
		label: "Google Gemini",
		description: "Gemini models via Google AI Studio",
		primary: true,
		fields: [
			apiKeyField("AIza…"),
			modelIdField("gemini-2.5-flash"),
			endpointField("https://generativelanguage.googleapis.com"),
		],
	},
	{
		key: "mistral",
		providerName: "custom:mistral",
		label: "Mistral",
		description: "Mistral models via La Plateforme",
		primary: true,
		fields: [
			apiKeyField("Mistral API key"),
			modelIdField("mistral-large-latest"),
			endpointField("https://api.mistral.ai"),
		],
	},
	{
		key: "groq",
		providerName: "custom:groq",
		label: "Groq",
		description: "Ultra-fast open-model inference",
		primary: true,
		fields: [
			apiKeyField("gsk_…"),
			modelIdField("llama-3.3-70b-versatile"),
			endpointField("https://api.groq.com"),
		],
	},
	{
		key: "openrouter",
		providerName: "custom:openrouter",
		label: "OpenRouter",
		description: "One key for hundreds of models",
		primary: true,
		fields: [
			apiKeyField("sk-or-…"),
			modelIdField("openai/gpt-4o"),
			endpointField("https://openrouter.ai/api/v1"),
		],
	},
	{
		key: "together",
		providerName: "custom:together",
		label: "Together AI",
		description: "Open-source models at scale",
		primary: true,
		fields: [
			apiKeyField("Together API key"),
			modelIdField("meta-llama/Llama-3.3-70B-Instruct-Turbo"),
			endpointField("https://api.together.xyz"),
		],
	},
	{
		key: "ollama",
		providerName: "custom:ollama",
		label: "Ollama",
		description: "Local models served by Ollama",
		primary: true,
		fields: [
			modelIdField("llama3.2", "Model"),
			{
				key: "endpoint",
				label: "Endpoint",
				placeholder: "http://localhost:11434",
				description: "Leave empty for the default local instance",
			},
		],
	},
	{
		key: "lmstudio",
		providerName: "custom:lmstudio",
		label: "LM Studio",
		description: "Local models served by LM Studio",
		primary: true,
		fields: [
			modelIdField("qwen2.5-7b-instruct", "Model"),
			{
				key: "endpoint",
				label: "Endpoint",
				placeholder: "http://localhost:1234",
				description: "Leave empty for the default local instance",
			},
		],
	},
	{
		key: "xai",
		providerName: "custom:xai",
		label: "xAI",
		description: "Grok models from xAI",
		primary: true,
		fields: [
			apiKeyField("xai-…"),
			modelIdField("grok-3"),
			endpointField("https://api.x.ai"),
		],
	},
	{
		key: "deepseek",
		providerName: "custom:deepseek",
		label: "DeepSeek",
		description: "DeepSeek chat and reasoner models",
		primary: true,
		fields: [
			apiKeyField("DeepSeek API key"),
			modelIdField("deepseek-chat"),
			endpointField("https://api.deepseek.com"),
		],
	},
	{
		key: "vertex",
		providerName: "custom:vertex",
		label: "Vertex AI",
		description: "Google Cloud Vertex AI",
		primary: true,
		fields: [
			modelIdField("gemini-2.5-flash"),
			{
				key: "service_account_json",
				label: "Service account JSON",
				placeholder: "Paste the service account key JSON",
				secret: true,
				multiline: true,
			},
			{
				key: "project_id",
				label: "Project ID",
				placeholder: "my-gcp-project",
				description: "Optional if the service account JSON contains it",
			},
			{
				key: "location",
				label: "Location",
				placeholder: "us-central1",
			},
			{
				key: "access_token",
				label: "Access token",
				placeholder: "Short-lived OAuth access token",
				secret: true,
				advanced: true,
				description: "Alternative to a service account key",
			},
		],
		validate: (values, isEdit) => {
			if (isEdit) return null;
			if (values.service_account_json?.trim() || values.access_token?.trim())
				return null;
			return "Vertex AI needs a service account JSON or an access token";
		},
	},
	{
		key: "huggingface",
		providerName: "custom:huggingface",
		label: "HuggingFace",
		description: "Hosted HF Inference Providers",
		primary: true,
		fields: [
			apiKeyField("hf_…"),
			modelIdField("meta-llama/Llama-3.3-70B-Instruct"),
			{
				key: "sub_provider",
				label: "Inference provider",
				placeholder: "together, fireworks-ai, …",
				advanced: true,
			},
			endpointField("https://router.huggingface.co"),
		],
	},
	{
		key: "cohere",
		providerName: "custom:cohere",
		label: "Cohere",
		description: "Command models from Cohere",
		primary: false,
		fields: [
			apiKeyField("Cohere API key"),
			modelIdField("command-a-03-2025"),
			endpointField("https://api.cohere.com"),
		],
	},
	{
		key: "perplexity",
		providerName: "custom:perplexity",
		label: "Perplexity",
		description: "Sonar models with built-in search",
		primary: false,
		fields: [
			apiKeyField("pplx-…"),
			modelIdField("sonar-pro"),
			endpointField("https://api.perplexity.ai"),
		],
	},
	{
		key: "moonshot",
		providerName: "custom:moonshot",
		label: "Moonshot AI",
		description: "Kimi models from Moonshot AI",
		primary: false,
		fields: [
			apiKeyField("Moonshot API key"),
			modelIdField("kimi-k2-0711-preview"),
			endpointField("https://api.moonshot.ai"),
		],
	},
	{
		key: "atlascloud",
		providerName: "custom:openai",
		label: "Atlas Cloud",
		description: "Open models on Atlas Cloud (OpenAI-compatible)",
		primary: false,
		fields: [
			apiKeyField("Atlas Cloud API key"),
			modelIdField("deepseek-ai/DeepSeek-V3.1"),
			{
				key: "endpoint",
				label: "Endpoint",
				placeholder: "https://api.atlascloud.ai/v1",
				defaultValue: "https://api.atlascloud.ai/v1",
				required: true,
			},
		],
	},
	{
		key: "minimax",
		providerName: "custom:openai",
		label: "MiniMax",
		description: "MiniMax models (OpenAI-compatible)",
		primary: false,
		fields: [
			apiKeyField("MiniMax API key"),
			modelIdField("MiniMax-M2"),
			{
				key: "endpoint",
				label: "Endpoint",
				placeholder: "https://api.minimax.io/v1",
				defaultValue: "https://api.minimax.io/v1",
				required: true,
				description: "China mainland: https://api.minimaxi.com/v1",
			},
		],
	},
	{
		key: "hyperbolic",
		providerName: "custom:hyperbolic",
		label: "Hyperbolic",
		description: "Open models on Hyperbolic",
		primary: false,
		fields: [
			apiKeyField("Hyperbolic API key"),
			modelIdField("meta-llama/Meta-Llama-3.1-70B-Instruct"),
			endpointField("https://api.hyperbolic.xyz"),
		],
	},
	{
		key: "galadriel",
		providerName: "custom:galadriel",
		label: "Galadriel",
		description: "Galadriel inference network",
		primary: false,
		fields: [
			apiKeyField("Galadriel API key"),
			modelIdField("gpt-4o"),
			endpointField("https://api.galadriel.com/v1/verified"),
		],
	},
	{
		key: "mira",
		providerName: "custom:mira",
		label: "Mira",
		description: "Mira decentralized inference",
		primary: false,
		fields: [
			apiKeyField("Mira API key"),
			modelIdField("Model ID"),
			endpointField("https://api.mira.network"),
		],
	},
	{
		key: "mozilla",
		providerName: "custom:mozilla",
		label: "Mozilla llamafile",
		description: "Self-hosted llamafile server",
		primary: false,
		fields: [
			{
				key: "api_key",
				label: "API key",
				placeholder: "Optional",
				secret: true,
			},
			modelIdField("Model ID"),
			{
				key: "endpoint",
				label: "Endpoint",
				placeholder: "http://localhost:8000/v1",
			},
		],
	},
];

const CLASSIFICATION_TRAITS: { key: string; label: string }[] = [
	{ key: "reasoning", label: "Reasoning" },
	{ key: "coding", label: "Coding" },
	{ key: "creativity", label: "Creativity" },
	{ key: "factuality", label: "Factuality" },
	{ key: "function_calling", label: "Function calling" },
	{ key: "multilinguality", label: "Multilinguality" },
	{ key: "speed", label: "Speed" },
	{ key: "cost", label: "Cost efficiency" },
	{ key: "safety", label: "Safety" },
	{ key: "openness", label: "Openness" },
];

const defaultClassification = (): Record<string, number> =>
	Object.fromEntries(CLASSIFICATION_TRAITS.map((t) => [t.key, 0.5]));

type WizardSource = "provider" | "huggingface";
type WizardStep = "pick" | "form";
type LocalModelFormat = "gguf" | "mlx";

function hasHuggingFaceMlxManifest(parameters: unknown): boolean {
	if (
		!parameters ||
		typeof parameters !== "object" ||
		Array.isArray(parameters)
	) {
		return false;
	}
	const manifest = (parameters as Record<string, unknown>).huggingface;
	if (!manifest || typeof manifest !== "object" || Array.isArray(manifest)) {
		return false;
	}
	const value = manifest as Record<string, unknown>;
	if (
		value.schema !== 1 ||
		value.format !== "mlx" ||
		typeof value.repo_id !== "string" ||
		!value.repo_id.trim() ||
		typeof value.revision !== "string" ||
		!value.revision.trim() ||
		!Array.isArray(value.files) ||
		value.files.length === 0
	) {
		return false;
	}
	return value.files.every(
		(file) =>
			!!file &&
			typeof file === "object" &&
			!Array.isArray(file) &&
			typeof (file as Record<string, unknown>).path === "string" &&
			!!String((file as Record<string, unknown>).path).trim() &&
			typeof (file as Record<string, unknown>).size === "number" &&
			Number.isSafeInteger((file as Record<string, unknown>).size) &&
			Number((file as Record<string, unknown>).size) >= 0,
	);
}

function providerDefForBit(bit: IBit): IProviderDef | null {
	const params = bit.parameters as ILlmParameters | undefined;
	const providerName = params?.provider?.provider_name;
	if (!providerName) return null;
	const providerParams = params?.provider?.params as
		| Record<string, unknown>
		| undefined;
	if (providerName === "custom:openai") {
		if (providerParams?.is_azure === true) {
			return PROVIDERS.find((p) => p.key === "azure-openai") ?? null;
		}
		const endpoint =
			typeof providerParams?.endpoint === "string"
				? providerParams.endpoint
				: "";
		if (endpoint.includes("atlascloud")) {
			return PROVIDERS.find((p) => p.key === "atlascloud") ?? null;
		}
		if (endpoint.includes("minimax")) {
			return PROVIDERS.find((p) => p.key === "minimax") ?? null;
		}
	}
	return (
		PROVIDERS.find((p) => p.providerName === providerName && !p.isAzure) ?? null
	);
}

export interface AddCustomModelDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	/** When set, the dialog edits this custom bit instead of creating one. */
	existingBit?: IBit | null;
	webMode?: boolean;
}

export function AddCustomModelDialog({
	open,
	onOpenChange,
	existingBit,
	webMode = false,
}: Readonly<AddCustomModelDialogProps>) {
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const isEdit = !!existingBit;
	const { canHostLlamaCPP, canHostMLX } = backend.capabilities();
	const canHostLocal = !webMode && (canHostLlamaCPP || canHostMLX);

	const [step, setStep] = useState<WizardStep>("pick");
	const [source, setSource] = useState<WizardSource>("provider");
	const [localFormat, setLocalFormat] = useState<LocalModelFormat>(
		canHostLlamaCPP ? "gguf" : "mlx",
	);
	const [providerKey, setProviderKey] = useState<string | null>(null);
	const [fieldValues, setFieldValues] = useState<Record<string, string>>({});
	const [contextLength, setContextLength] = useState(DEFAULT_CONTEXT_LENGTH);
	const [isVision, setIsVision] = useState(false);
	const [displayName, setDisplayName] = useState("");
	const [description, setDescription] = useState("");
	const [icon, setIcon] = useState("");
	const [tags, setTags] = useState("");
	const [classification, setClassification] = useState<Record<string, number>>(
		defaultClassification,
	);
	const [hfReference, setHfReference] = useState("");
	const [hfImport, setHfImport] = useState<HuggingFaceModelImport | null>(null);
	const [hfImportError, setHfImportError] = useState<string | null>(null);
	const [inspectingHf, setInspectingHf] = useState(false);
	const [ggufVariantId, setGgufVariantId] = useState("");
	const [ggufProjectorPath, setGgufProjectorPath] = useState("");
	const [hfDownload, setHfDownload] = useState("");
	const [hfFileName, setHfFileName] = useState("");
	const [hfRepo, setHfRepo] = useState("");
	const [hfSize, setHfSize] = useState("");
	const [mmprojDownload, setMmprojDownload] = useState("");
	const [mmprojFileName, setMmprojFileName] = useState("");
	const [mmprojSize, setMmprojSize] = useState("");
	const [detectingSize, setDetectingSize] = useState(false);
	const [detectingMmprojSize, setDetectingMmprojSize] = useState(false);
	const [saving, setSaving] = useState(false);
	const hfInspectionSequence = useRef(0);

	const providerDef = useMemo(
		() => PROVIDERS.find((p) => p.key === providerKey) ?? null,
		[providerKey],
	);

	useEffect(() => {
		if (!open) return;
		hfInspectionSequence.current += 1;
		setSaving(false);
		setDetectingSize(false);
		if (!existingBit) {
			setStep("pick");
			setSource("provider");
			setLocalFormat(canHostLlamaCPP ? "gguf" : "mlx");
			setProviderKey(null);
			setFieldValues({});
			setContextLength(DEFAULT_CONTEXT_LENGTH);
			setIsVision(false);
			setDisplayName("");
			setDescription("");
			setIcon("");
			setTags("");
			setClassification(defaultClassification());
			setHfReference("");
			setHfImport(null);
			setHfImportError(null);
			setInspectingHf(false);
			setGgufVariantId("");
			setGgufProjectorPath("");
			setHfDownload("");
			setHfFileName("");
			setHfRepo("");
			setHfSize("");
			setMmprojDownload("");
			setMmprojFileName("");
			setMmprojSize("");
			return;
		}

		const params = existingBit.parameters as ILlmParameters | undefined;
		const localProviderName = params?.provider?.provider_name;
		const isLocal =
			localProviderName === LOCAL_PROVIDER_NAME ||
			localProviderName === MLX_PROVIDER_NAME;
		const def = isLocal ? null : providerDefForBit(existingBit);
		const providerParams = (params?.provider?.params ?? {}) as Record<
			string,
			unknown
		>;

		setStep("form");
		setSource(isLocal ? "huggingface" : "provider");
		setLocalFormat(localProviderName === MLX_PROVIDER_NAME ? "mlx" : "gguf");
		setProviderKey(def?.key ?? null);
		const values: Record<string, string> = {};
		for (const field of def?.fields ?? []) {
			if (field.secret) continue;
			const raw =
				providerParams[field.key] ??
				(field.key === "model_id" ? params?.provider?.model_id : undefined) ??
				(field.key === "version" ? params?.provider?.version : undefined);
			if (raw !== undefined && raw !== null) values[field.key] = String(raw);
		}
		setFieldValues(values);
		setContextLength(String(params?.context_length ?? DEFAULT_CONTEXT_LENGTH));
		setIsVision(existingBit.type === IBitTypes.Vlm);
		setDisplayName(existingBit.meta?.en?.name ?? "");
		setDescription(existingBit.meta?.en?.description ?? "");
		setIcon(existingBit.meta?.en?.icon ?? "");
		setTags((existingBit.meta?.en?.tags ?? []).join(", "));
		setClassification({
			...defaultClassification(),
			...(params?.model_classification ?? {}),
		});
		setHfReference(existingBit.repository ?? "");
		setHfImport(null);
		setHfImportError(null);
		setInspectingHf(false);
		setGgufVariantId("");
		setGgufProjectorPath("");
		setHfDownload(existingBit.download_link ?? "");
		setHfFileName(existingBit.file_name ?? "");
		setHfRepo(existingBit.repository ?? "");
		setHfSize(existingBit.size ? String(existingBit.size) : "");

		const projection = (providerParams.projection ?? {}) as Record<
			string,
			unknown
		>;
		setMmprojDownload(String(projection.download_link ?? ""));
		setMmprojFileName(String(projection.file_name ?? ""));
		setMmprojSize(projection.size ? String(projection.size) : "");
	}, [open, existingBit, canHostLlamaCPP]);

	const setFieldValue = useCallback((key: string, value: string) => {
		setFieldValues((prev) => ({ ...prev, [key]: value }));
	}, []);

	const pickProvider = useCallback((key: string) => {
		setSource("provider");
		setProviderKey(key);
		const def = PROVIDERS.find((provider) => provider.key === key);
		const seeded: Record<string, string> = {};
		for (const field of def?.fields ?? []) {
			if (field.defaultValue) seeded[field.key] = field.defaultValue;
		}
		setFieldValues(seeded);
		setStep("form");
	}, []);

	const pickHuggingFace = useCallback(() => {
		setSource("huggingface");
		setLocalFormat(canHostLlamaCPP ? "gguf" : "mlx");
		setProviderKey(null);
		setStep("form");
	}, [canHostLlamaCPP]);

	const applyImportedMetadata = useCallback(
		(imported: HuggingFaceModelImport, kind: "llm" | "vlm") => {
			setDisplayName(imported.modelName);
			setDescription(
				(current) =>
					current.trim() ||
					`${imported.modelName} — ${imported.format.toUpperCase()} ${kind.toUpperCase()} imported from Hugging Face`,
			);
			setTags(imported.tags.join(", "));
			setContextLength(String(imported.contextLength));
			setIsVision(kind === "vlm");
			setHfRepo(imported.repositoryUrl);
		},
		[],
	);

	const applyGgufSelection = useCallback(
		(
			imported: HuggingFaceGgufRepositoryImport,
			variantId: string,
			projectorPath: string,
			kind: "llm" | "vlm",
		) => {
			const selection = resolveHuggingFaceGgufSelection(imported, {
				variantId,
				projectorPath: projectorPath || undefined,
				kind,
			});
			const model = selection.variant.files[0];
			setGgufVariantId(selection.variant.id);
			setGgufProjectorPath(selection.projector?.path ?? "");
			setHfDownload(model.downloadUrl);
			setHfFileName(model.path);
			setHfSize(String(model.size));
			setMmprojDownload(selection.projector?.downloadUrl ?? "");
			setMmprojFileName(selection.projector?.path ?? "");
			setMmprojSize(
				selection.projector ? String(selection.projector.size) : "",
			);
			setIsVision(selection.kind === "vlm");
			setHfImportError(null);
		},
		[],
	);

	const inspectHuggingFaceRepository = useCallback(
		async (reference = hfReference) => {
			const trimmed = reference.trim();
			if (!trimmed) {
				setHfImportError(
					"Enter a Hugging Face repository or direct GGUF file URL",
				);
				return;
			}
			const sequence = ++hfInspectionSequence.current;
			setInspectingHf(true);
			setHfImportError(null);
			try {
				const imported = await inspectHuggingFaceModelRepository(trimmed);
				if (sequence !== hfInspectionSequence.current) return;
				if (imported.access.private || imported.access.gated !== false) {
					throw new Error(
						"Private and gated repositories need Hugging Face authentication, which this direct-reference flow does not store",
					);
				}
				if (imported.format === "mlx" && !canHostMLX) {
					throw new Error(
						"That is an MLX repository, but MLX is only available on supported Apple devices",
					);
				}
				if (imported.format === "gguf" && !canHostLlamaCPP) {
					throw new Error(
						"That is a GGUF repository, but llama.cpp is unavailable on this device",
					);
				}

				setHfImport(imported);
				setLocalFormat(imported.format);
				setHfReference(trimmed);

				if (imported.format === "mlx") {
					applyImportedMetadata(imported, imported.kind);
					setGgufVariantId("");
					setGgufProjectorPath("");
					setHfDownload("");
					setHfFileName("");
					setHfSize("");
					setMmprojDownload("");
					setMmprojFileName("");
					setMmprojSize("");
					return;
				}

				const kind = imported.kind === "vlm" ? "vlm" : "llm";
				const requestedVariantId = imported.variants.find(
					(variant) => variant.requested,
				)?.id;
				const variantId =
					requestedVariantId ??
					imported.recommendedVariantId ??
					imported.variants.find(
						(variant) => variant.complete && !variant.split,
					)?.id ??
					"";
				const projectorPath =
					kind === "vlm" ? (imported.recommendedProjectorPath ?? "") : "";
				applyImportedMetadata(imported, kind);
				try {
					applyGgufSelection(imported, variantId, projectorPath, kind);
				} catch (error) {
					setGgufVariantId(variantId);
					setGgufProjectorPath(projectorPath);
					setHfImportError(
						error instanceof Error
							? error.message
							: "Choose a supported GGUF variant",
					);
				}
			} catch (error) {
				if (sequence !== hfInspectionSequence.current) return;
				setHfImport(null);
				setHfImportError(
					error instanceof Error
						? error.message
						: "Failed to inspect the Hugging Face repository",
				);
			} finally {
				if (sequence === hfInspectionSequence.current) {
					setInspectingHf(false);
				}
			}
		},
		[
			hfReference,
			canHostLlamaCPP,
			canHostMLX,
			applyImportedMetadata,
			applyGgufSelection,
		],
	);

	const changeLocalFormat = useCallback(
		(format: LocalModelFormat) => {
			if (
				(format === "gguf" && !canHostLlamaCPP) ||
				(format === "mlx" && !canHostMLX)
			) {
				return;
			}
			setLocalFormat(format);
			setHfImport((current) => (current?.format === format ? current : null));
			setHfImportError(null);
		},
		[canHostLlamaCPP, canHostMLX],
	);

	const changeVisionModel = useCallback(
		(vision: boolean) => {
			setIsVision(vision);
			if (hfImport?.format !== "gguf") return;
			const projectorPath = vision
				? ggufProjectorPath || hfImport.recommendedProjectorPath || ""
				: "";
			try {
				applyGgufSelection(
					hfImport,
					ggufVariantId,
					projectorPath,
					vision ? "vlm" : "llm",
				);
			} catch (error) {
				setGgufProjectorPath(projectorPath);
				setHfImportError(
					error instanceof Error
						? error.message
						: "Choose a compatible GGUF model and projector",
				);
			}
		},
		[hfImport, ggufProjectorPath, ggufVariantId, applyGgufSelection],
	);

	const applyHfUrl = useCallback((url: string) => {
		const trimmed = url.trim();
		if (!trimmed) return;
		try {
			const parsed = new URL(trimmed);
			const segments = parsed.pathname.split("/").filter(Boolean);
			const fileName = decodeURIComponent(segments[segments.length - 1] ?? "");
			if (fileName) setHfFileName((prev) => prev.trim() || fileName);
			const resolveIdx = parsed.pathname.indexOf("/resolve/");
			if (resolveIdx > 0) {
				const repo = `${parsed.origin}${parsed.pathname.slice(0, resolveIdx)}`;
				setHfRepo((prev) => prev.trim() || repo);
			}
		} catch {
			// Not a parsable URL yet — the user is still typing.
		}
	}, []);

	const detectSize = useCallback(
		async (
			url: string,
			apply: (size: string) => void,
			setBusy: (busy: boolean) => void,
			silent: boolean,
		) => {
			const trimmed = url.trim();
			if (!trimmed) return;
			setBusy(true);
			try {
				const res = await fetch(trimmed, { method: "HEAD" });
				const len = Number.parseInt(
					res.headers.get("content-length") ?? "",
					10,
				);
				if (Number.isFinite(len) && len > 0) {
					apply(String(len));
				} else if (!silent) {
					toast.info("Could not detect the file size — enter it manually.");
				}
			} catch {
				if (!silent)
					toast.info("Could not detect the file size — enter it manually.");
			} finally {
				setBusy(false);
			}
		},
		[],
	);

	const detectHfSize = useCallback(
		(silent: boolean) =>
			detectSize(hfDownload, setHfSize, setDetectingSize, silent),
		[detectSize, hfDownload],
	);

	const detectMmprojSize = useCallback(
		(silent: boolean) =>
			detectSize(mmprojDownload, setMmprojSize, setDetectingMmprojSize, silent),
		[detectSize, mmprojDownload],
	);

	const applyMmprojUrl = useCallback((url: string) => {
		const trimmed = url.trim();
		if (!trimmed) return;
		try {
			const segments = new URL(trimmed).pathname.split("/").filter(Boolean);
			const fileName = decodeURIComponent(segments[segments.length - 1] ?? "");
			if (fileName) setMmprojFileName((prev) => prev.trim() || fileName);
		} catch {
			// Not a parsable URL yet — the user is still typing.
		}
	}, []);

	const parsedTags = useMemo(
		() =>
			tags
				.split(",")
				.map((tag) => tag.trim())
				.filter(Boolean),
		[tags],
	);

	const validationError = useMemo((): string | null => {
		if (step !== "form") return null;
		if (!displayName.trim()) return "Give the model a display name";
		const ctx = Number.parseInt(contextLength, 10);
		if (!Number.isFinite(ctx) || ctx <= 0)
			return "Context length must be a positive number";
		if (source === "huggingface") {
			if (localFormat === "mlx") {
				if (!canHostMLX) return "MLX is unavailable on this device";
				if (
					hfImport?.format !== "mlx" &&
					!hasHuggingFaceMlxManifest(existingBit?.parameters)
				) {
					return "Paste and inspect a public MLX repository";
				}
				if (
					hfImport?.format === "mlx" &&
					(hfImport.kind === "vlm") !== isVision
				) {
					return hfImport.kind === "vlm"
						? "This MLX repository is a VLM; keep Vision model enabled"
						: "This MLX repository is an LLM and cannot be saved as a vision model";
				}
				return null;
			}
			if (!canHostLlamaCPP) {
				return "GGUF models are unavailable on this device";
			}
			if (hfImport?.format === "gguf") {
				try {
					resolveHuggingFaceGgufSelection(hfImport, {
						variantId: ggufVariantId,
						projectorPath: ggufProjectorPath || undefined,
						kind: isVision ? "vlm" : "llm",
					});
				} catch (error) {
					return error instanceof Error
						? error.message
						: "Choose a supported GGUF variant";
				}
			}
			if (!hfDownload.trim())
				return "A direct download link to the GGUF file is required";
			try {
				validateHuggingFacePinnedGgufDownloadUrl(hfDownload);
			} catch (error) {
				return `Model link: ${
					error instanceof Error
						? error.message
						: "Use an immutable Hugging Face URL"
				}`;
			}
			if (!hfFileName.trim()) return "File name is required";
			const size = Number.parseInt(hfSize, 10);
			if (!Number.isFinite(size) || size <= 0)
				return "File size is required — use Detect or enter it in bytes";
			if (isVision) {
				if (!mmprojDownload.trim())
					return "Vision needs a projector: add the mmproj download link";
				try {
					validateHuggingFacePinnedGgufDownloadUrl(mmprojDownload);
				} catch (error) {
					return `Projector link: ${
						error instanceof Error
							? error.message
							: "Use an immutable Hugging Face URL"
					}`;
				}
				if (!mmprojFileName.trim()) return "Projector file name is required";
				const projectorSize = Number.parseInt(mmprojSize, 10);
				if (!Number.isFinite(projectorSize) || projectorSize <= 0)
					return "Projector size is required — use Detect or enter it in bytes";
			}
			return null;
		}
		if (!providerDef) return "Pick a provider first";
		for (const field of providerDef.fields) {
			if (!field.required) continue;
			if (field.secret && isEdit) continue;
			if (!fieldValues[field.key]?.trim()) return `${field.label} is required`;
		}
		return providerDef.validate?.(fieldValues, isEdit) ?? null;
	}, [
		step,
		displayName,
		contextLength,
		source,
		localFormat,
		canHostLlamaCPP,
		canHostMLX,
		hfImport,
		ggufVariantId,
		ggufProjectorPath,
		hfDownload,
		hfFileName,
		hfSize,
		isVision,
		mmprojDownload,
		mmprojFileName,
		mmprojSize,
		providerDef,
		fieldValues,
		isEdit,
		existingBit?.parameters,
	]);

	const handleSave = useCallback(async () => {
		if (validationError) return;
		setSaving(true);
		try {
			const now = new Date().toISOString();
			const secs = Math.floor(Date.now() / 1000);
			const existingEn = existingBit?.meta?.en;
			const selectedImport =
				source === "huggingface" && hfImport?.format === localFormat
					? hfImport
					: null;
			const meta: IMetadata = {
				name: displayName.trim(),
				description: description.trim(),
				long_description: existingEn?.long_description ?? null,
				icon: icon.trim() || null,
				thumbnail: existingEn?.thumbnail ?? null,
				tags: parsedTags,
				preview_media: existingEn?.preview_media ?? [],
				age_rating: existingEn?.age_rating ?? null,
				docs_url: existingEn?.docs_url ?? selectedImport?.repositoryUrl ?? null,
				release_notes: existingEn?.release_notes ?? null,
				support_url: existingEn?.support_url ?? null,
				use_case:
					existingEn?.use_case ??
					(selectedImport ? (isVision ? "Vision and chat" : "Chat") : null),
				website: existingEn?.website ?? selectedImport?.repositoryUrl ?? null,
				organization_specific_values:
					existingEn?.organization_specific_values ?? null,
				created_at: existingEn?.created_at ?? {
					nanos_since_epoch: 0,
					secs_since_epoch: secs,
				},
				updated_at: { nanos_since_epoch: 0, secs_since_epoch: secs },
			};

			const isHf = source === "huggingface";
			const isMlx = isHf && localFormat === "mlx";
			const secrets: Record<string, unknown> = {};
			const existingParameters = (existingBit?.parameters ?? {}) as Record<
				string,
				unknown
			>;
			const existingProvider = (existingParameters.provider ?? {}) as Record<
				string,
				unknown
			>;
			const existingMlxParams = Object.fromEntries(
				Object.entries(
					(existingProvider.params ?? {}) as Record<string, unknown>,
				).filter(([key]) => key !== "projection" && key !== "huggingface"),
			);
			const params: Record<string, unknown> = isMlx ? existingMlxParams : {};
			let modelId: string | null = null;
			let version: string | null = null;

			if (!isHf) {
				if (!providerDef) return;
				for (const field of providerDef.fields) {
					const value = fieldValues[field.key]?.trim();
					if (!value) continue;
					if (field.secret) {
						secrets[field.key] = value;
						continue;
					}
					params[field.key] = value;
					if (field.key === "model_id") modelId = value;
					if (field.key === "version") version = value;
				}
				if (providerDef.isAzure) params.is_azure = true;
			}

			// llama.cpp needs the projector as its own artifact; it rides along in
			// the provider params and is materialised as a Projection bit at load.
			if (isHf && !isMlx && isVision && mmprojDownload.trim()) {
				const projectorSize = Number.parseInt(mmprojSize, 10);
				params.projection = {
					download_link: mmprojDownload.trim(),
					file_name: mmprojFileName.trim(),
					size:
						Number.isFinite(projectorSize) && projectorSize > 0
							? projectorSize
							: undefined,
				};
			}

			const importedMlx =
				isMlx && selectedImport?.format === "mlx" ? selectedImport : null;
			const mlxManifest = isMlx
				? importedMlx
					? createHuggingFaceUserMlxManifest(importedMlx)
					: (existingParameters.huggingface as
							| Record<string, unknown>
							| undefined)
				: undefined;
			if (mlxManifest) {
				modelId = String(mlxManifest.repo_id ?? "") || null;
				version = String(mlxManifest.revision ?? "") || null;
			}
			if (selectedImport) {
				modelId = selectedImport.repoId;
				version = selectedImport.revision;
			}

			let bit: IBit = {
				id: existingBit?.id ?? createId(),
				type: isVision ? IBitTypes.Vlm : IBitTypes.Llm,
				meta: { ...(existingBit?.meta ?? {}), en: meta },
				parameters: {
					context_length: Number.parseInt(contextLength, 10),
					provider: {
						provider_name: isHf
							? isMlx
								? MLX_PROVIDER_NAME
								: LOCAL_PROVIDER_NAME
							: (providerDef?.providerName ?? ""),
						model_id: modelId,
						version,
						params,
					},
					model_classification: { ...classification },
					...(mlxManifest ? { huggingface: mlxManifest } : {}),
				},
				download_link: isHf && !isMlx ? hfDownload.trim() : null,
				file_name: isHf && !isMlx ? hfFileName.trim() : null,
				size: isHf ? (isMlx ? 0 : Number.parseInt(hfSize, 10)) : null,
				repository: isHf ? hfRepo.trim() || null : null,
				dependencies: [],
				hash: existingBit?.hash ?? "",
				dependency_tree_hash: existingBit?.dependency_tree_hash ?? "",
				authors: selectedImport
					? [selectedImport.authorUrl]
					: (existingBit?.authors ?? []),
				hub: existingBit?.hub ?? "",
				version: null,
				license: selectedImport?.license ?? existingBit?.license ?? null,
				created: existingBit?.created ?? now,
				updated: now,
			};

			if (importedMlx) {
				const mapped = applyHuggingFaceMlxImportToUserBit(bit, importedMlx);
				const mappedParameters = mapped.parameters as ILlmParameters;
				bit = {
					...mapped,
					// Discovery supplies strong defaults; these fields deliberately
					// remain the user's final choices in this dialog.
					name: displayName.trim(),
					type: isVision ? IBitTypes.Vlm : IBitTypes.Llm,
					meta: { ...mapped.meta, en: meta },
					authors: [importedMlx.authorUrl],
					license: importedMlx.license,
					repository: importedMlx.repositoryUrl,
					parameters: {
						...mappedParameters,
						context_length: Number.parseInt(contextLength, 10),
						model_classification: { ...classification },
						huggingface: mlxManifest,
						provider: {
							...mappedParameters.provider,
							provider_name: MLX_PROVIDER_NAME,
							model_id: importedMlx.repoId,
							version: importedMlx.revision,
							params,
						},
					},
				};
			}

			const saved = await backend.bitState.upsertCustomBit(
				bit,
				Object.keys(secrets).length > 0 ? secrets : undefined,
			);

			// The library is user-wide, but configuring a model means you want to
			// use it: activate a newly created one in the profile you are in.
			if (!isEdit) {
				try {
					const settingsProfile = await backend.userState.getSettingsProfile();
					await backend.bitState.addBit(saved ?? bit, settingsProfile);
				} catch (error) {
					console.warn("Model saved but not added to the profile", error);
				}
			}

			await Promise.all([
				invalidate(backend.bitState.listCustomBits, []),
				invalidate(backend.bitState.getProfileBits, []),
				invalidate(backend.userState.getSettingsProfile, []),
				invalidate(backend.userState.getProfile, []),
				invalidate(
					backend.bitState.searchBits as unknown as () => Promise<IBit[]>,
					[],
				),
			]);
			toast.success(isEdit ? "Model updated" : "Custom model added");
			onOpenChange(false);
		} catch (error) {
			toast.error(
				`Failed to save model: ${
					error instanceof Error ? error.message : String(error)
				}`,
			);
		} finally {
			setSaving(false);
		}
	}, [
		validationError,
		existingBit,
		displayName,
		description,
		icon,
		parsedTags,
		source,
		localFormat,
		hfImport,
		providerDef,
		fieldValues,
		isVision,
		contextLength,
		classification,
		hfDownload,
		hfFileName,
		hfSize,
		hfRepo,
		mmprojDownload,
		mmprojFileName,
		mmprojSize,
		backend.bitState,
		backend.userState,
		invalidate,
		isEdit,
		onOpenChange,
	]);

	const title = useMemo(() => {
		if (isEdit) return "Edit custom model";
		if (step === "pick") return "Add a custom model";
		return source === "huggingface"
			? "HuggingFace model"
			: (providerDef?.label ?? "Configure model");
	}, [isEdit, step, source, providerDef]);

	const subtitle = useMemo(() => {
		if (isEdit) return "Only you can see and use this model.";
		if (step === "pick")
			return "Bring your own API key or run a model locally. Private to you.";
		return source === "huggingface"
			? localFormat === "mlx"
				? "Reference an MLX repository and run it locally on this Apple device."
				: "Reference GGUF weights and run them locally with llama.cpp."
			: (providerDef?.description ?? "");
	}, [isEdit, step, source, localFormat, providerDef]);

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="sm:max-w-2xl">
				<DialogHeader>
					<div className="flex items-center gap-2">
						{step === "form" && !isEdit && (
							<Button
								variant="ghost"
								size="icon"
								className="h-7 w-7 shrink-0 rounded-full"
								onClick={() => setStep("pick")}
							>
								<ArrowLeft className="h-4 w-4" />
								<span className="sr-only">Back to provider selection</span>
							</Button>
						)}
						<DialogTitle>{title}</DialogTitle>
					</div>
					<DialogDescription>{subtitle}</DialogDescription>
				</DialogHeader>

				{step === "pick" ? (
					<SourcePickStep
						canHostLocal={canHostLocal}
						canHostLlamaCPP={canHostLlamaCPP}
						canHostMLX={canHostMLX}
						onPickProvider={pickProvider}
						onPickHuggingFace={pickHuggingFace}
					/>
				) : (
					<div className="space-y-6">
						{source === "provider" && providerDef && (
							<ProviderConnectionSection
								def={providerDef}
								values={fieldValues}
								onChange={setFieldValue}
								isEdit={isEdit}
							/>
						)}
						{source === "huggingface" && (
							<LocalFormatSelector
								format={localFormat}
								canHostLlamaCPP={canHostLlamaCPP}
								canHostMLX={canHostMLX}
								onChange={changeLocalFormat}
							/>
						)}

						{source === "huggingface" && (
							<HuggingFaceRepositorySection
								reference={hfReference}
								imported={hfImport}
								error={hfImportError}
								inspecting={inspectingHf}
								disabled={saving}
								onReferenceChange={(value) => {
									setHfReference(value);
									setHfImport(null);
									setHfImportError(null);
								}}
								onInspect={(reference) =>
									void inspectHuggingFaceRepository(reference)
								}
							/>
						)}

						{source === "huggingface" &&
							localFormat === "gguf" &&
							hfImport?.format === "gguf" && (
								<GgufSelectionSection
									imported={hfImport}
									variantId={ggufVariantId}
									projectorPath={ggufProjectorPath}
									isVision={isVision}
									onVariantChange={(variantId) => {
										try {
											applyGgufSelection(
												hfImport,
												variantId,
												ggufProjectorPath,
												isVision ? "vlm" : "llm",
											);
										} catch (error) {
											setGgufVariantId(variantId);
											setHfImportError(
												error instanceof Error
													? error.message
													: "Choose a supported GGUF variant",
											);
										}
									}}
									onProjectorChange={(projectorPath) => {
										try {
											applyGgufSelection(
												hfImport,
												ggufVariantId,
												projectorPath,
												"vlm",
											);
										} catch (error) {
											setGgufProjectorPath(projectorPath);
											setHfImportError(
												error instanceof Error
													? error.message
													: "Choose a supported GGUF projector",
											);
										}
									}}
								/>
							)}

						{source === "huggingface" && localFormat === "gguf" && (
							<HuggingFaceSection
								download={hfDownload}
								fileName={hfFileName}
								repo={hfRepo}
								size={hfSize}
								detecting={detectingSize}
								onDownloadChange={setHfDownload}
								onDownloadBlur={(url) => {
									applyHfUrl(url);
									if (!hfSize.trim()) detectHfSize(true);
								}}
								onFileNameChange={setHfFileName}
								onRepoChange={setHfRepo}
								onSizeChange={setHfSize}
								onDetectSize={() => detectHfSize(false)}
							/>
						)}

						{source === "huggingface" && localFormat === "gguf" && isVision && (
							<ProjectorSection
								download={mmprojDownload}
								fileName={mmprojFileName}
								size={mmprojSize}
								detecting={detectingMmprojSize}
								onDownloadChange={setMmprojDownload}
								onDownloadBlur={(url) => {
									applyMmprojUrl(url);
									if (!mmprojSize.trim()) detectMmprojSize(true);
								}}
								onFileNameChange={setMmprojFileName}
								onSizeChange={setMmprojSize}
								onDetectSize={() => detectMmprojSize(false)}
							/>
						)}

						<ModelSettingsSection
							contextLength={contextLength}
							onContextLengthChange={setContextLength}
							isVision={isVision}
							onVisionChange={changeVisionModel}
						/>

						<MetadataSection
							name={displayName}
							onNameChange={setDisplayName}
							description={description}
							onDescriptionChange={setDescription}
							icon={icon}
							onIconChange={setIcon}
							tags={tags}
							onTagsChange={setTags}
							parsedTags={parsedTags}
						/>

						<CharacteristicsSection
							classification={classification}
							onChange={(key, value) =>
								setClassification((prev) => ({ ...prev, [key]: value }))
							}
						/>
					</div>
				)}

				{step === "form" && (
					<DialogFooter className="items-center gap-2 sm:justify-between">
						<p className="text-xs text-muted-foreground">
							{validationError ?? ""}
						</p>
						<div className="flex items-center gap-2">
							<Button
								variant="ghost"
								onClick={() => onOpenChange(false)}
								disabled={saving}
							>
								Cancel
							</Button>
							<Button
								onClick={handleSave}
								disabled={!!validationError || saving}
							>
								{saving && <Loader2 className="h-4 w-4 animate-spin" />}
								{isEdit ? "Save changes" : "Add model"}
							</Button>
						</div>
					</DialogFooter>
				)}
			</DialogContent>
		</Dialog>
	);
}

function SectionHeading({
	icon: Icon,
	label,
	hint,
}: Readonly<{
	icon: React.ComponentType<{ className?: string }>;
	label: string;
	hint?: string;
}>) {
	return (
		<div className="space-y-0.5">
			<p className="flex items-center gap-2 text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
				<Icon className="h-3 w-3" />
				{label}
			</p>
			{hint && <p className="text-xs text-muted-foreground/50">{hint}</p>}
		</div>
	);
}

function ProviderMonogram({ label }: Readonly<{ label: string }>) {
	const initials = useMemo(() => {
		const words = label.split(/\s+/).filter(Boolean);
		if (words.length >= 2) return `${words[0][0]}${words[1][0]}`.toUpperCase();
		return label.slice(0, 2).toUpperCase();
	}, [label]);
	return (
		<div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-primary/20 bg-primary/10 text-xs font-semibold text-primary">
			{initials}
		</div>
	);
}

function ProviderTile({
	def,
	onPick,
}: Readonly<{ def: IProviderDef; onPick: (key: string) => void }>) {
	return (
		<button
			type="button"
			onClick={() => onPick(def.key)}
			className="flex items-center gap-3 rounded-lg border bg-card p-3 text-left transition-all hover:border-primary/40 hover:bg-accent/50"
		>
			<ProviderMonogram label={def.label} />
			<div className="min-w-0 flex-1">
				<p className="truncate text-sm font-medium">{def.label}</p>
				<p className="truncate text-xs text-muted-foreground">
					{def.description}
				</p>
			</div>
		</button>
	);
}

function SourcePickStep({
	canHostLocal,
	canHostLlamaCPP,
	canHostMLX,
	onPickProvider,
	onPickHuggingFace,
}: Readonly<{
	canHostLocal: boolean;
	canHostLlamaCPP: boolean;
	canHostMLX: boolean;
	onPickProvider: (key: string) => void;
	onPickHuggingFace: () => void;
}>) {
	const [showMore, setShowMore] = useState(false);
	const primary = useMemo(() => PROVIDERS.filter((p) => p.primary), []);
	const secondary = useMemo(() => PROVIDERS.filter((p) => !p.primary), []);

	return (
		<div className="space-y-5">
			<div className="space-y-3">
				<SectionHeading
					icon={Plug}
					label="Connect a provider"
					hint="Use your own API key — requests go directly to the provider."
				/>
				<div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
					{primary.map((def) => (
						<ProviderTile key={def.key} def={def} onPick={onPickProvider} />
					))}
				</div>
				<Collapsible open={showMore} onOpenChange={setShowMore}>
					<CollapsibleTrigger asChild>
						<button
							type="button"
							className="flex items-center gap-1.5 text-xs text-muted-foreground/60 transition-colors hover:text-foreground"
						>
							<ChevronDown
								className={`h-3.5 w-3.5 transition-transform ${showMore ? "rotate-180" : ""}`}
							/>
							More providers ({secondary.length})
						</button>
					</CollapsibleTrigger>
					<CollapsibleContent>
						<div className="grid grid-cols-1 gap-2 pt-2 sm:grid-cols-2">
							{secondary.map((def) => (
								<ProviderTile key={def.key} def={def} onPick={onPickProvider} />
							))}
						</div>
					</CollapsibleContent>
				</Collapsible>
			</div>

			{canHostLocal && (
				<div className="space-y-3 border-t border-border/20 pt-4">
					<SectionHeading
						icon={HardDriveDownload}
						label="Run locally"
						hint="Download model weights and run them on this device."
					/>
					<button
						type="button"
						onClick={onPickHuggingFace}
						className="flex w-full items-center gap-3 rounded-lg border bg-card p-3 text-left transition-all hover:border-primary/40 hover:bg-accent/50"
					>
						<div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-amber-500/30 bg-amber-500/10 text-amber-600">
							<HardDriveDownload className="h-4 w-4" />
						</div>
						<div className="min-w-0 flex-1">
							<p className="text-sm font-medium">HuggingFace model</p>
							<p className="text-xs text-muted-foreground">
								{canHostLlamaCPP && canHostMLX
									? "Reference a GGUF or MLX repository and run it offline"
									: canHostMLX
										? "Reference an MLX repository and run it offline"
										: "Reference a GGUF model and run it offline"}
							</p>
						</div>
					</button>
				</div>
			)}
		</div>
	);
}

function LocalFormatSelector({
	format,
	canHostLlamaCPP,
	canHostMLX,
	onChange,
}: Readonly<{
	format: LocalModelFormat;
	canHostLlamaCPP: boolean;
	canHostMLX: boolean;
	onChange: (format: LocalModelFormat) => void;
}>) {
	return (
		<div className="space-y-3">
			<SectionHeading
				icon={HardDriveDownload}
				label="Local runtime"
				hint="The model stays private to your account. Its files are downloaded directly from Hugging Face to this device."
			/>
			<div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
				<button
					type="button"
					disabled={!canHostLlamaCPP}
					onClick={() => onChange("gguf")}
					className={`rounded-lg border p-3 text-left transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${
						format === "gguf"
							? "border-primary bg-primary/5"
							: "bg-card hover:border-primary/40"
					}`}
				>
					<div className="flex items-center justify-between gap-2">
						<p className="text-sm font-medium">GGUF</p>
						<Badge variant="outline">llama.cpp</Badge>
					</div>
					<p className="mt-1 text-xs text-muted-foreground">
						One model file, with an optional vision projector.
					</p>
					{!canHostLlamaCPP && (
						<p className="mt-1 text-xs text-muted-foreground">
							Unavailable on this device
						</p>
					)}
				</button>
				<button
					type="button"
					disabled={!canHostMLX}
					onClick={() => onChange("mlx")}
					className={`rounded-lg border p-3 text-left transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${
						format === "mlx"
							? "border-primary bg-primary/5"
							: "bg-card hover:border-primary/40"
					}`}
				>
					<div className="flex items-center justify-between gap-2">
						<p className="text-sm font-medium">MLX</p>
						<Badge variant="outline">Apple only</Badge>
					</div>
					<p className="mt-1 text-xs text-muted-foreground">
						A complete multi-file repository for Apple silicon.
					</p>
					{!canHostMLX && (
						<p className="mt-1 text-xs text-muted-foreground">
							Unavailable on this device
						</p>
					)}
				</button>
			</div>
		</div>
	);
}

function HuggingFaceRepositorySection({
	reference,
	imported,
	error,
	inspecting,
	disabled,
	onReferenceChange,
	onInspect,
}: Readonly<{
	reference: string;
	imported: HuggingFaceModelImport | null;
	error: string | null;
	inspecting: boolean;
	disabled: boolean;
	onReferenceChange: (value: string) => void;
	onInspect: (reference?: string) => void;
}>) {
	const fileCount =
		imported?.format === "mlx"
			? imported.assets.length
			: imported?.variants.length;
	const selectedSize =
		imported?.format === "mlx"
			? imported.totalSize
			: imported?.variants.find(
					(variant) => variant.id === imported.recommendedVariantId,
				)?.totalSize;

	return (
		<div className="space-y-3">
			<SectionHeading
				icon={ScanSearch}
				label="Hugging Face repository"
				hint="Paste a repository or direct GGUF file URL. Flow-Like pins the current commit and fills the model details."
			/>
			<div className="flex flex-col gap-2 sm:flex-row">
				<Input
					id="custom-model-hf-reference"
					value={reference}
					disabled={disabled || inspecting}
					onChange={(event) => onReferenceChange(event.target.value)}
					onPaste={(event) => {
						const pasted = event.clipboardData.getData("text").trim();
						if (!pasted || disabled || inspecting) return;
						event.preventDefault();
						onReferenceChange(pasted);
						onInspect(pasted);
					}}
					onKeyDown={(event) => {
						if (event.key !== "Enter") return;
						event.preventDefault();
						onInspect();
					}}
					placeholder="https://huggingface.co/owner/model"
					autoComplete="off"
					spellCheck={false}
				/>
				<Button
					type="button"
					variant="outline"
					className="shrink-0"
					disabled={disabled || inspecting || !reference.trim()}
					onClick={() => onInspect()}
				>
					{inspecting ? (
						<Loader2 className="h-3.5 w-3.5 animate-spin" />
					) : (
						<ScanSearch className="h-3.5 w-3.5" />
					)}
					Inspect &amp; fill
				</Button>
			</div>

			{error && <p className="text-xs text-destructive">{error}</p>}

			{imported && (
				<div className="space-y-2 rounded-lg border bg-muted/25 p-3">
					<div className="flex flex-wrap items-center gap-2">
						<span className="text-sm font-medium">{imported.repoId}</span>
						<Badge variant="secondary">{imported.format.toUpperCase()}</Badge>
						<Badge variant="outline">{imported.revision.slice(0, 10)}</Badge>
						{fileCount !== undefined && (
							<Badge variant="outline">
								{fileCount}{" "}
								{imported.format === "mlx" ? "runtime files" : "variants"}
							</Badge>
						)}
						{selectedSize !== undefined && (
							<Badge variant="outline">{humanFileSize(selectedSize)}</Badge>
						)}
						<Badge variant="outline">{imported.license}</Badge>
					</div>
					<p className="text-xs text-muted-foreground">
						This user-only model keeps immutable Hugging Face references. Files
						download directly to the device when needed and are not copied into
						the shared store or CDN.
					</p>
					{imported.warnings.map((warning) => (
						<p key={warning} className="text-xs text-amber-600">
							{warning}
						</p>
					))}
				</div>
			)}
		</div>
	);
}

function GgufSelectionSection({
	imported,
	variantId,
	projectorPath,
	isVision,
	onVariantChange,
	onProjectorChange,
}: Readonly<{
	imported: HuggingFaceGgufRepositoryImport;
	variantId: string;
	projectorPath: string;
	isVision: boolean;
	onVariantChange: (variantId: string) => void;
	onProjectorChange: (projectorPath: string) => void;
}>) {
	return (
		<div className="space-y-3">
			<SectionHeading
				icon={SlidersHorizontal}
				label="GGUF selection"
				hint="Choose one complete, single-file quantization. Split GGUF variants are listed but not supported yet."
			/>
			<div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
				<div className="space-y-1.5">
					<Label htmlFor="custom-model-gguf-variant" className="text-xs">
						Quantization<span className="text-destructive"> *</span>
					</Label>
					<select
						id="custom-model-gguf-variant"
						value={variantId}
						onChange={(event) => onVariantChange(event.target.value)}
						className="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-xs outline-none focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50"
					>
						<option value="" disabled>
							Choose a GGUF variant
						</option>
						{imported.variants.map((variant) => (
							<option
								key={variant.id}
								value={variant.id}
								disabled={!variant.complete || variant.split}
							>
								{variant.label} · {humanFileSize(variant.totalSize)}
								{variant.split
									? " · split (unsupported)"
									: !variant.complete
										? " · incomplete"
										: ""}
							</option>
						))}
					</select>
				</div>

				{isVision && (
					<div className="space-y-1.5">
						<Label htmlFor="custom-model-gguf-projector" className="text-xs">
							Vision projector<span className="text-destructive"> *</span>
						</Label>
						<select
							id="custom-model-gguf-projector"
							value={projectorPath}
							onChange={(event) => onProjectorChange(event.target.value)}
							className="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-xs outline-none focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50"
						>
							<option value="" disabled>
								Choose an mmproj file
							</option>
							{imported.projectors.map((projector) => (
								<option key={projector.path} value={projector.path}>
									{projector.path} · {humanFileSize(projector.size)}
								</option>
							))}
						</select>
					</div>
				)}
			</div>
		</div>
	);
}

function ProviderFieldInput({
	field,
	value,
	onChange,
	isEdit,
}: Readonly<{
	field: IProviderField;
	value: string;
	onChange: (value: string) => void;
	isEdit: boolean;
}>) {
	const placeholder =
		field.secret && isEdit
			? "Stored securely — type to replace"
			: field.placeholder;
	const inputId = `custom-model-${field.key}`;
	return (
		<div className="space-y-1.5">
			<Label htmlFor={inputId} className="text-xs">
				{field.label}
				{field.required && <span className="text-destructive"> *</span>}
			</Label>
			{field.multiline ? (
				<Textarea
					id={inputId}
					value={value}
					onChange={(e) => onChange(e.target.value)}
					placeholder={placeholder}
					rows={4}
					className="font-mono text-xs"
					autoComplete="off"
					spellCheck={false}
				/>
			) : (
				<Input
					id={inputId}
					type={field.secret ? "password" : "text"}
					value={value}
					onChange={(e) => onChange(e.target.value)}
					placeholder={placeholder}
					autoComplete="off"
					spellCheck={false}
				/>
			)}
			{field.description && (
				<p className="text-xs text-muted-foreground/60">{field.description}</p>
			)}
		</div>
	);
}

function ProviderConnectionSection({
	def,
	values,
	onChange,
	isEdit,
}: Readonly<{
	def: IProviderDef;
	values: Record<string, string>;
	onChange: (key: string, value: string) => void;
	isEdit: boolean;
}>) {
	const [showAdvanced, setShowAdvanced] = useState(false);
	const basicFields = useMemo(
		() => def.fields.filter((f) => !f.advanced),
		[def],
	);
	const advancedFields = useMemo(
		() => def.fields.filter((f) => f.advanced),
		[def],
	);

	return (
		<div className="space-y-3">
			<div className="flex items-center justify-between">
				<SectionHeading icon={Plug} label="Connection" />
				{isEdit && (
					<Badge variant="outline" className="text-[10px]">
						{def.label}
					</Badge>
				)}
			</div>
			<div className="space-y-3">
				{basicFields.map((field) => (
					<ProviderFieldInput
						key={field.key}
						field={field}
						value={values[field.key] ?? ""}
						onChange={(v) => onChange(field.key, v)}
						isEdit={isEdit}
					/>
				))}
			</div>
			{advancedFields.length > 0 && (
				<Collapsible open={showAdvanced} onOpenChange={setShowAdvanced}>
					<CollapsibleTrigger asChild>
						<button
							type="button"
							className="flex items-center gap-1.5 text-xs text-muted-foreground/60 transition-colors hover:text-foreground"
						>
							<ChevronDown
								className={`h-3.5 w-3.5 transition-transform ${showAdvanced ? "rotate-180" : ""}`}
							/>
							Advanced options
						</button>
					</CollapsibleTrigger>
					<CollapsibleContent>
						<div className="space-y-3 pt-2">
							{advancedFields.map((field) => (
								<ProviderFieldInput
									key={field.key}
									field={field}
									value={values[field.key] ?? ""}
									onChange={(v) => onChange(field.key, v)}
									isEdit={isEdit}
								/>
							))}
						</div>
					</CollapsibleContent>
				</Collapsible>
			)}
		</div>
	);
}

function HuggingFaceSection({
	download,
	fileName,
	repo,
	size,
	detecting,
	onDownloadChange,
	onDownloadBlur,
	onFileNameChange,
	onRepoChange,
	onSizeChange,
	onDetectSize,
}: Readonly<{
	download: string;
	fileName: string;
	repo: string;
	size: string;
	detecting: boolean;
	onDownloadChange: (value: string) => void;
	onDownloadBlur: (value: string) => void;
	onFileNameChange: (value: string) => void;
	onRepoChange: (value: string) => void;
	onSizeChange: (value: string) => void;
	onDetectSize: () => void;
}>) {
	const parsedSize = Number.parseInt(size, 10);
	return (
		<div className="space-y-3">
			<SectionHeading
				icon={HardDriveDownload}
				label="Model files"
				hint="Paste the direct download link of a GGUF file — the rest is filled in automatically."
			/>
			<div className="space-y-1.5">
				<Label htmlFor="custom-model-hf-download" className="text-xs">
					Download link (GGUF)<span className="text-destructive"> *</span>
				</Label>
				<Input
					id="custom-model-hf-download"
					value={download}
					onChange={(e) => onDownloadChange(e.target.value)}
					onBlur={(e) => onDownloadBlur(e.target.value)}
					placeholder="https://huggingface.co/<owner>/<repo>/resolve/<commit-sha>/model-Q4_K_M.gguf"
					autoComplete="off"
					spellCheck={false}
				/>
			</div>
			<div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
				<div className="space-y-1.5">
					<Label htmlFor="custom-model-hf-file" className="text-xs">
						File name<span className="text-destructive"> *</span>
					</Label>
					<Input
						id="custom-model-hf-file"
						value={fileName}
						onChange={(e) => onFileNameChange(e.target.value)}
						placeholder="model-Q4_K_M.gguf"
						autoComplete="off"
						spellCheck={false}
					/>
				</div>
				<div className="space-y-1.5">
					<Label htmlFor="custom-model-hf-size" className="text-xs">
						File size (bytes)<span className="text-destructive"> *</span>
					</Label>
					<div className="flex items-center gap-2">
						<Input
							id="custom-model-hf-size"
							value={size}
							onChange={(e) => onSizeChange(e.target.value.replace(/\D/g, ""))}
							placeholder="0"
							inputMode="numeric"
							autoComplete="off"
						/>
						<Button
							variant="outline"
							size="sm"
							onClick={onDetectSize}
							disabled={detecting || !download.trim()}
							className="shrink-0"
						>
							{detecting ? (
								<Loader2 className="h-3.5 w-3.5 animate-spin" />
							) : (
								<ScanSearch className="h-3.5 w-3.5" />
							)}
							Detect
						</Button>
					</div>
					{Number.isFinite(parsedSize) && parsedSize > 0 && (
						<p className="text-xs text-muted-foreground/60">
							≈ {humanFileSize(parsedSize)}
						</p>
					)}
				</div>
			</div>
			<div className="space-y-1.5">
				<Label htmlFor="custom-model-hf-repo" className="text-xs">
					Repository
				</Label>
				<Input
					id="custom-model-hf-repo"
					value={repo}
					onChange={(e) => onRepoChange(e.target.value)}
					placeholder="https://huggingface.co/<owner>/<repo>"
					autoComplete="off"
					spellCheck={false}
				/>
			</div>
		</div>
	);
}

/**
 * The multimodal projector. llama.cpp loads images through a separate mmproj
 * file, so a local vision model is two artifacts, not one.
 */
function ProjectorSection({
	download,
	fileName,
	size,
	detecting,
	onDownloadChange,
	onDownloadBlur,
	onFileNameChange,
	onSizeChange,
	onDetectSize,
}: Readonly<{
	download: string;
	fileName: string;
	size: string;
	detecting: boolean;
	onDownloadChange: (value: string) => void;
	onDownloadBlur: (value: string) => void;
	onFileNameChange: (value: string) => void;
	onSizeChange: (value: string) => void;
	onDetectSize: () => void;
}>) {
	const parsedSize = Number.parseInt(size, 10);
	return (
		<div className="space-y-3">
			<SectionHeading
				icon={ScanEye}
				label="Vision projector"
				hint="Vision runs through a separate mmproj file — usually next to the model in the same repo."
			/>
			<div className="space-y-1.5">
				<Label htmlFor="custom-model-mmproj-download" className="text-xs">
					Projector link (mmproj)<span className="text-destructive"> *</span>
				</Label>
				<Input
					id="custom-model-mmproj-download"
					value={download}
					onChange={(e) => onDownloadChange(e.target.value)}
					onBlur={(e) => onDownloadBlur(e.target.value)}
					placeholder="https://huggingface.co/<owner>/<repo>/resolve/<commit-sha>/mmproj-F16.gguf"
					autoComplete="off"
					spellCheck={false}
				/>
			</div>
			<div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
				<div className="space-y-1.5">
					<Label htmlFor="custom-model-mmproj-file" className="text-xs">
						File name<span className="text-destructive"> *</span>
					</Label>
					<Input
						id="custom-model-mmproj-file"
						value={fileName}
						onChange={(e) => onFileNameChange(e.target.value)}
						placeholder="mmproj-F16.gguf"
						autoComplete="off"
						spellCheck={false}
					/>
				</div>
				<div className="space-y-1.5">
					<Label htmlFor="custom-model-mmproj-size" className="text-xs">
						File size (bytes)<span className="text-destructive"> *</span>
					</Label>
					<div className="flex items-center gap-2">
						<Input
							id="custom-model-mmproj-size"
							value={size}
							onChange={(e) => onSizeChange(e.target.value.replace(/\D/g, ""))}
							placeholder="0"
							inputMode="numeric"
							autoComplete="off"
						/>
						<Button
							variant="outline"
							size="sm"
							onClick={onDetectSize}
							disabled={detecting || !download.trim()}
							className="shrink-0"
						>
							{detecting ? (
								<Loader2 className="h-3.5 w-3.5 animate-spin" />
							) : (
								<ScanSearch className="h-3.5 w-3.5" />
							)}
							Detect
						</Button>
					</div>
					{Number.isFinite(parsedSize) && parsedSize > 0 && (
						<p className="text-xs text-muted-foreground/60">
							≈ {humanFileSize(parsedSize)}
						</p>
					)}
				</div>
			</div>
		</div>
	);
}

function ModelSettingsSection({
	contextLength,
	onContextLengthChange,
	isVision,
	onVisionChange,
}: Readonly<{
	contextLength: string;
	onContextLengthChange: (value: string) => void;
	isVision: boolean;
	onVisionChange: (value: boolean) => void;
}>) {
	const parsed = Number.parseInt(contextLength, 10);
	return (
		<div className="space-y-3">
			<SectionHeading icon={SlidersHorizontal} label="Model settings" />
			<div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
				<div className="space-y-1.5">
					<Label htmlFor="custom-model-context" className="text-xs">
						Context length (tokens)
					</Label>
					<Input
						id="custom-model-context"
						value={contextLength}
						onChange={(e) =>
							onContextLengthChange(e.target.value.replace(/\D/g, ""))
						}
						placeholder={DEFAULT_CONTEXT_LENGTH}
						inputMode="numeric"
						autoComplete="off"
					/>
					{Number.isFinite(parsed) && parsed > 0 && (
						<p className="text-xs text-muted-foreground/60">
							≈ {formatContextLength(parsed)}
						</p>
					)}
				</div>
				<div className="flex items-center justify-between gap-3 rounded-lg border bg-card/50 p-3">
					<div className="flex min-w-0 items-center gap-2.5">
						<Eye className="h-4 w-4 shrink-0 text-cyan-500" />
						<div className="min-w-0">
							<Label htmlFor="custom-model-vision" className="text-xs">
								Supports vision
							</Label>
							<p className="text-xs text-muted-foreground/60">
								The model accepts images as input
							</p>
						</div>
					</div>
					<Switch
						id="custom-model-vision"
						checked={isVision}
						onCheckedChange={onVisionChange}
					/>
				</div>
			</div>
		</div>
	);
}

function MetadataSection({
	name,
	onNameChange,
	description,
	onDescriptionChange,
	icon,
	onIconChange,
	tags,
	onTagsChange,
	parsedTags,
}: Readonly<{
	name: string;
	onNameChange: (value: string) => void;
	description: string;
	onDescriptionChange: (value: string) => void;
	icon: string;
	onIconChange: (value: string) => void;
	tags: string;
	onTagsChange: (value: string) => void;
	parsedTags: string[];
}>) {
	return (
		<div className="space-y-3">
			<SectionHeading
				icon={Bot}
				label="Display"
				hint="How this model shows up in your catalog and pickers."
			/>
			<div className="flex items-start gap-3">
				<Avatar className="mt-6 h-9 w-9 shrink-0 border border-border/50">
					<AvatarImage src={icon.trim() || undefined} />
					<AvatarFallback className="bg-muted">
						<Bot className="h-4 w-4 text-muted-foreground" />
					</AvatarFallback>
				</Avatar>
				<div className="min-w-0 flex-1 space-y-3">
					<div className="space-y-1.5">
						<Label htmlFor="custom-model-name" className="text-xs">
							Display name<span className="text-destructive"> *</span>
						</Label>
						<Input
							id="custom-model-name"
							value={name}
							onChange={(e) => onNameChange(e.target.value)}
							placeholder="My GPT-4o"
							autoComplete="off"
						/>
					</div>
					<div className="space-y-1.5">
						<Label htmlFor="custom-model-icon" className="text-xs">
							Icon URL
						</Label>
						<Input
							id="custom-model-icon"
							value={icon}
							onChange={(e) => onIconChange(e.target.value)}
							placeholder="https://… (optional)"
							autoComplete="off"
							spellCheck={false}
						/>
					</div>
				</div>
			</div>
			<div className="space-y-1.5">
				<Label htmlFor="custom-model-description" className="text-xs">
					Description
				</Label>
				<Textarea
					id="custom-model-description"
					value={description}
					onChange={(e) => onDescriptionChange(e.target.value)}
					placeholder="What is this model good at?"
					rows={2}
				/>
			</div>
			<div className="space-y-1.5">
				<Label htmlFor="custom-model-tags" className="text-xs">
					Tags
				</Label>
				<Input
					id="custom-model-tags"
					value={tags}
					onChange={(e) => onTagsChange(e.target.value)}
					placeholder="chat, coding, … (comma separated)"
					autoComplete="off"
				/>
				{parsedTags.length > 0 && (
					<div className="flex flex-wrap gap-1 pt-1">
						{parsedTags.map((tag) => (
							<Badge key={tag} variant="secondary" className="text-[10px]">
								{tag}
							</Badge>
						))}
					</div>
				)}
			</div>
		</div>
	);
}

function CharacteristicsSection({
	classification,
	onChange,
}: Readonly<{
	classification: Record<string, number>;
	onChange: (key: string, value: number) => void;
}>) {
	const [open, setOpen] = useState(false);
	return (
		<Collapsible open={open} onOpenChange={setOpen}>
			<CollapsibleTrigger asChild>
				<button
					type="button"
					className="flex w-full items-center gap-2 text-left"
				>
					<ChevronDown
						className={`h-3.5 w-3.5 text-muted-foreground/60 transition-transform ${open ? "rotate-180" : ""}`}
					/>
					<SectionHeading
						icon={SlidersHorizontal}
						label="Model characteristics"
						hint="Optional — drives automatic model selection (e.g. the Find Model node)."
					/>
				</button>
			</CollapsibleTrigger>
			<CollapsibleContent>
				<div className="grid grid-cols-1 gap-x-6 gap-y-4 pt-4 sm:grid-cols-2">
					{CLASSIFICATION_TRAITS.map((trait) => {
						const value = classification[trait.key] ?? 0.5;
						return (
							<div key={trait.key} className="space-y-1.5">
								<div className="flex items-center justify-between text-xs">
									<span className="text-muted-foreground">{trait.label}</span>
									<span className="tabular-nums text-muted-foreground/50">
										{Math.round(value * 100)}%
									</span>
								</div>
								<Slider
									value={[value]}
									onValueChange={([v]) => onChange(trait.key, v)}
									min={0}
									max={1}
									step={0.05}
									className="h-1"
								/>
							</div>
						);
					})}
				</div>
			</CollapsibleContent>
		</Collapsible>
	);
}
