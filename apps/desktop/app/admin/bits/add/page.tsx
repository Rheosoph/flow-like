"use client";

import { createId } from "@paralleldrive/cuid2";
import {
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	type IBit,
	IBitTypes,
	type IEmbeddingModelParameters,
	type ILlmParameters,
	type IMetadata,
	type IModelProvider,
	IPooling,
	ITtsDTypePreference,
	type ITtsModelParameters,
	ITtsModelType,
	ITtsRuntimePreference,
	Input,
	Label,
	Progress,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Separator,
	Textarea,
	nowSystemTime,
	useBackend,
	useInvoke,
} from "@tm9657/flow-like-ui";
import {
	AudioLines,
	Binary,
	Cpu,
	Eye,
	FileTextIcon,
	GaugeIcon,
	HashIcon,
	ImageIcon,
	Loader2Icon,
	Mic,
	PackageIcon,
	ScanLine,
	TimerIcon,
	UploadCloudIcon,
	X,
	Zap,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import {
	getContextLength,
	getModelLicense,
	getModelName,
	getModelSize,
	getModelTags,
	getOriginalRepo,
	getUserInfo,
	guessedModelLink,
} from "../utils";
import { DependencyConfiguration } from "./dependency";
import { EmbeddingConfiguration } from "./embedding";
import { LLMConfiguration } from "./llm";
import { MetaConfiguration } from "./meta";
import {
	TTSConfiguration,
	type TtsAssetDraft,
	applyTtsModelPreset,
	defaultTtsAssetLayout,
} from "./tts";

// ── constants ──────────────────────────────────────────────────────────────

const HOSTED_PROVIDER_OPTIONS = [
	"Hosted",
	"hosted:openrouter",
	"hosted:openai",
	"hosted:anthropic",
	"hosted:azure",
	"hosted:vertex",
] as const;

type BitMode =
	| "local-llm"
	| "hosted-llm"
	| "hosted-stt"
	| "vlm"
	| "tts"
	| "embedding"
	| "image-embedding"
	| "classification";

// ── helpers ────────────────────────────────────────────────────────────────

function createDefaultLlmParameters(providerName = "Local"): ILlmParameters {
	return {
		context_length: 2048,
		model_classification: {
			cost: 0.3,
			creativity: 0.3,
			factuality: 0.3,
			function_calling: 0.3,
			multilinguality: 0.3,
			openness: 0.3,
			reasoning: 0.3,
			coding: 0.3,
			safety: 0.3,
			speed: 0.3,
		},
		provider: {
			provider_name: providerName,
			model_id: null,
			version: null,
			params: providerName.toLowerCase().startsWith("hosted") ? {} : undefined,
		},
	};
}

function isHostedProviderName(providerName?: null | string) {
	const normalized = providerName?.trim().toLowerCase() ?? "";
	return normalized === "hosted" || normalized.startsWith("hosted:");
}

const DEFAULT_LLM_PARAMETERS: ILlmParameters = createDefaultLlmParameters();

const DEFAULT_EMBEDDING_PARAMETERS: IEmbeddingModelParameters = {
	input_length: 2048,
	languages: [],
	pooling: IPooling.Mean,
	provider: {
		provider_name: "Local",
		model_id: null,
		version: null,
	},
	prefix: {
		paragraph: "",
		query: "",
	},
	vector_length: 1024,
};

const DEFAULT_TTS_PARAMETERS: ITtsModelParameters = {
	assets: [],
	default_language: null,
	default_voice: null,
	dtype: ITtsDTypePreference.Auto,
	languages: [],
	model_type: ITtsModelType.Kokoro,
	provider: {
		provider_name: "local:any-tts",
		model_id: null,
		version: null,
	},
	runtime: ITtsRuntimePreference.Auto,
	voices: [],
};

const DEFAULT_BIT: IBit = {
	id: createId(),
	authors: [],
	created: new Date().toISOString(),
	updated: new Date().toISOString(),
	dependencies: [],
	dependency_tree_hash: "",
	download_link: "",
	file_name: "",
	hash: "",
	hub: "",
	meta: {
		en: {
			name: "",
			description: "",
			long_description: "",
			tags: [],
			icon: "",
			thumbnail: "",
			preview_media: [],
			website: "",
			support_url: "",
			docs_url: "",
			created_at: nowSystemTime(),
			updated_at: nowSystemTime(),
			age_rating: null,
			use_case: "",
			organization_specific_values: null,
			release_notes: "",
		},
	},
	name: "",
	parameters: {},
	type: IBitTypes.Llm,
	repository: "",
	size: 0,
	license: "",
	version: "0.0.1",
};

// ── HostedLLMForm ──────────────────────────────────────────────────────────

function HostedLLMForm({
	bit,
	setBit,
	loading,
	onSubmit,
}: {
	bit: IBit;
	setBit: React.Dispatch<React.SetStateAction<IBit>>;
	loading: boolean;
	onSubmit: () => void;
}) {
	const params = (bit.parameters as ILlmParameters) ?? {};
	const provider = params.provider ?? { provider_name: "Hosted" };
	const providerParams = (provider.params ?? {}) as Record<string, unknown>;

	const [tagInput, setTagInput] = useState("");
	const [authorInput, setAuthorInput] = useState("");
	const tags = bit.meta?.en?.tags ?? [];
	const authors = bit.authors ?? [];

	const updateParam = (key: keyof ILlmParameters, value: unknown) => {
		setBit((old) => ({
			...old,
			parameters: { ...old.parameters, [key]: value },
		}));
	};

	const updateProvider = (update: Partial<IModelProvider>) => {
		setBit((old) => ({
			...old,
			parameters: {
				...old.parameters,
				provider: {
					...((old.parameters as ILlmParameters).provider ?? {}),
					...update,
				},
			},
		}));
	};

	const updateProviderParam = (key: string, value: string) => {
		setBit((old) => {
			const current = ((old.parameters as ILlmParameters).provider?.params ??
				{}) as Record<string, unknown>;
			return {
				...old,
				parameters: {
					...old.parameters,
					provider: {
						...((old.parameters as ILlmParameters).provider ?? {}),
						params: { ...current, [key]: value },
					},
				},
			};
		});
	};

	const updateMeta = (key: keyof IMetadata, value: unknown) => {
		setBit((old) => ({
			...old,
			meta: { ...old.meta, en: { ...(old.meta?.en ?? {}), [key]: value } },
		}));
	};

	const addTag = () => {
		const t = tagInput.trim();
		if (t && !tags.includes(t)) {
			updateMeta("tags", [...tags, t]);
			setTagInput("");
		}
	};

	const removeTag = (tag: string) => {
		updateMeta(
			"tags",
			tags.filter((t) => t !== tag),
		);
	};

	const addAuthor = () => {
		const a = authorInput.trim();
		if (a && !authors.includes(a)) {
			setBit((old) => ({ ...old, authors: [...authors, a] }));
			setAuthorInput("");
		}
	};

	const removeAuthor = (author: string) => {
		setBit((old) => ({
			...old,
			authors: (old.authors ?? []).filter((a) => a !== author),
		}));
	};

	return (
		<div className="space-y-4 max-w-3xl">
			{/* identity */}
			<Card>
				<CardHeader>
					<CardTitle>Model Identity</CardTitle>
					<CardDescription>
						The slug drives auto-computed capability scores — no manual sliders
						needed.
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					<div className="grid gap-4 sm:grid-cols-2">
						<div className="space-y-2">
							<Label htmlFor="hosted-slug">Model Slug *</Label>
							<Input
								id="hosted-slug"
								value={bit.name ?? ""}
								onChange={(e) =>
									setBit((old) => ({ ...old, name: e.target.value.trim() }))
								}
								placeholder="step-3-5-flash"
							/>
							<p className="text-xs text-muted-foreground">
								Used to auto-compute capability scores.
							</p>
						</div>
						<div className="space-y-2">
							<Label htmlFor="hosted-display-name">Display Name *</Label>
							<Input
								id="hosted-display-name"
								value={bit.meta?.en?.name ?? ""}
								onChange={(e) => updateMeta("name", e.target.value)}
								placeholder="Step 3.5 Flash"
							/>
						</div>
					</div>
					<div className="space-y-2">
						<Label htmlFor="hosted-description">Description</Label>
						<Textarea
							id="hosted-description"
							rows={2}
							value={bit.meta?.en?.description ?? ""}
							onChange={(e) => updateMeta("description", e.target.value)}
							placeholder="Brief description of the model…"
						/>
					</div>
					<div className="space-y-2">
						<Label htmlFor="hosted-long-description">Long Description</Label>
						<Textarea
							id="hosted-long-description"
							rows={4}
							value={bit.meta?.en?.long_description ?? ""}
							onChange={(e) => updateMeta("long_description", e.target.value)}
							placeholder="Detailed description of the model's capabilities and use cases…"
						/>
					</div>
				</CardContent>
			</Card>

			{/* provider */}
			<Card>
				<CardHeader>
					<CardTitle>Provider Settings</CardTitle>
					<CardDescription>
						Configure which provider routes and serves this model.
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					<div className="grid gap-4 sm:grid-cols-3">
						<div className="space-y-2">
							<Label>Provider *</Label>
							<Select
								value={provider.provider_name ?? "Hosted"}
								onValueChange={(v) => updateProvider({ provider_name: v })}
							>
								<SelectTrigger>
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									{HOSTED_PROVIDER_OPTIONS.map((p) => (
										<SelectItem key={p} value={p}>
											{p}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
						</div>
						<div className="space-y-2">
							<Label htmlFor="hosted-model-id">Model ID</Label>
							<Input
								id="hosted-model-id"
								value={provider.model_id ?? ""}
								onChange={(e) =>
									updateProvider({ model_id: e.target.value || null })
								}
								placeholder="@preset/prod-free"
							/>
						</div>
						<div className="space-y-2">
							<Label htmlFor="hosted-context">Context Length</Label>
							<Input
								id="hosted-context"
								type="number"
								value={params.context_length ?? 2048}
								onChange={(e) =>
									updateParam(
										"context_length",
										Number.parseInt(e.target.value) || 2048,
									)
								}
							/>
						</div>
					</div>
					<div className="grid gap-4 sm:grid-cols-2">
						<div className="space-y-2">
							<Label htmlFor="hosted-endpoint">Endpoint</Label>
							<Input
								id="hosted-endpoint"
								value={
									typeof providerParams.endpoint === "string"
										? providerParams.endpoint
										: ""
								}
								onChange={(e) =>
									updateProviderParam("endpoint", e.target.value)
								}
								placeholder="https://api.example.com/v1"
							/>
						</div>
						<div className="space-y-2">
							<Label htmlFor="hosted-tier">Tier</Label>
							<Input
								id="hosted-tier"
								value={
									typeof providerParams.tier === "string"
										? providerParams.tier
										: ""
								}
								onChange={(e) => updateProviderParam("tier", e.target.value)}
								placeholder="FREE"
							/>
						</div>
					</div>
				</CardContent>
			</Card>

			{/* registry info */}
			<Card>
				<CardHeader>
					<CardTitle>Registry Info</CardTitle>
					<CardDescription>
						Hub, version, license, and repository.
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					<div className="grid gap-4 sm:grid-cols-2">
						<div className="space-y-2">
							<Label htmlFor="hosted-hub">Hub</Label>
							<Input
								id="hosted-hub"
								value={bit.hub ?? ""}
								onChange={(e) =>
									setBit((old) => ({ ...old, hub: e.target.value }))
								}
								placeholder="https://flow-like.com/models"
							/>
						</div>
						<div className="space-y-2">
							<Label htmlFor="hosted-version">Version</Label>
							<Input
								id="hosted-version"
								value={bit.version ?? "0.0.1"}
								onChange={(e) =>
									setBit((old) => ({ ...old, version: e.target.value }))
								}
								placeholder="0.0.1"
							/>
						</div>
						<div className="space-y-2">
							<Label htmlFor="hosted-license">License</Label>
							<Input
								id="hosted-license"
								value={bit.license ?? ""}
								onChange={(e) =>
									setBit((old) => ({ ...old, license: e.target.value }))
								}
								placeholder="e.g. MIT, Apache-2.0"
							/>
						</div>
						<div className="space-y-2">
							<Label htmlFor="hosted-repository">Repository URL</Label>
							<Input
								id="hosted-repository"
								value={bit.repository ?? ""}
								onChange={(e) =>
									setBit((old) => ({ ...old, repository: e.target.value }))
								}
								placeholder="https://huggingface.co/…"
							/>
						</div>
					</div>
					<div className="grid gap-4 sm:grid-cols-2">
						<div className="space-y-2">
							<Label htmlFor="hosted-website">Website URL</Label>
							<Input
								id="hosted-website"
								value={bit.meta?.en?.website ?? ""}
								onChange={(e) => updateMeta("website", e.target.value)}
								placeholder="https://example.com"
							/>
						</div>
						<div className="space-y-2">
							<Label htmlFor="hosted-use-case">Use Case</Label>
							<Input
								id="hosted-use-case"
								value={bit.meta?.en?.use_case ?? ""}
								onChange={(e) => updateMeta("use_case", e.target.value)}
								placeholder="e.g. Chat, Code, Analysis"
							/>
						</div>
					</div>
				</CardContent>
			</Card>

			{/* media & authors */}
			<Card>
				<CardHeader>
					<CardTitle>Media &amp; Authors</CardTitle>
					<CardDescription>Icon, thumbnail, authors, and tags.</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					<div className="grid gap-4 sm:grid-cols-2">
						<div className="space-y-2">
							<Label htmlFor="hosted-icon">Icon URL</Label>
							<Input
								id="hosted-icon"
								value={bit.meta?.en?.icon ?? ""}
								onChange={(e) => updateMeta("icon", e.target.value)}
								placeholder="https://example.com/icon.png"
							/>
						</div>
						<div className="space-y-2">
							<Label htmlFor="hosted-thumbnail">Thumbnail URL</Label>
							<Input
								id="hosted-thumbnail"
								value={bit.meta?.en?.thumbnail ?? ""}
								onChange={(e) => updateMeta("thumbnail", e.target.value)}
								placeholder="https://example.com/thumbnail.png"
							/>
						</div>
					</div>
					<div className="space-y-2">
						<Label>Authors</Label>
						<div className="flex gap-2">
							<Input
								value={authorInput}
								onChange={(e) => setAuthorInput(e.target.value)}
								onKeyDown={(e) => {
									if (e.key === "Enter") {
										e.preventDefault();
										addAuthor();
									}
								}}
								placeholder="Add author and press Enter"
							/>
							<Button type="button" variant="outline" onClick={addAuthor}>
								Add
							</Button>
						</div>
						{authors.length > 0 && (
							<div className="flex flex-wrap gap-1 pt-1">
								{authors.map((a) => (
									<Badge key={a} variant="secondary" className="gap-1">
										{a}
										<button type="button" onClick={() => removeAuthor(a)}>
											<X className="h-3 w-3" />
										</button>
									</Badge>
								))}
							</div>
						)}
					</div>
					<div className="space-y-2">
						<Label>Tags</Label>
						<div className="flex gap-2">
							<Input
								value={tagInput}
								onChange={(e) => setTagInput(e.target.value)}
								onKeyDown={(e) => {
									if (e.key === "Enter") {
										e.preventDefault();
										addTag();
									}
								}}
								placeholder="Add tag and press Enter"
							/>
							<Button type="button" variant="outline" onClick={addTag}>
								Add
							</Button>
						</div>
						{tags.length > 0 && (
							<div className="flex flex-wrap gap-1 pt-1">
								{tags.map((tag) => (
									<Badge key={tag} variant="secondary" className="gap-1">
										{tag}
										<button type="button" onClick={() => removeTag(tag)}>
											<X className="h-3 w-3" />
										</button>
									</Badge>
								))}
							</div>
						)}
					</div>
				</CardContent>
			</Card>

			<Button
				className="w-full max-w-3xl"
				disabled={loading || !bit.name?.trim()}
				onClick={onSubmit}
			>
				{loading ? (
					<Loader2Icon className="mr-2 h-4 w-4 animate-spin" />
				) : (
					<Zap className="mr-2 h-4 w-4" />
				)}
				Add Hosted Model
			</Button>
		</div>
	);
}

// ── mode selector ──────────────────────────────────────────────────────────

const MODES: { id: BitMode; label: string; icon: React.ReactNode }[] = [
	{ id: "local-llm", label: "Local LLM", icon: <Cpu className="h-4 w-4" /> },
	{ id: "hosted-llm", label: "Hosted LLM", icon: <Zap className="h-4 w-4" /> },
	{ id: "vlm", label: "VLM", icon: <Eye className="h-4 w-4" /> },
	{ id: "tts", label: "TTS", icon: <AudioLines className="h-4 w-4" /> },
	{ id: "hosted-stt", label: "STT", icon: <Mic className="h-4 w-4" /> },
	{ id: "embedding", label: "Embedding", icon: <Binary className="h-4 w-4" /> },
	{
		id: "image-embedding",
		label: "Image Embedding",
		icon: <ImageIcon className="h-4 w-4" />,
	},
	{
		id: "classification",
		label: "Classification",
		icon: <ScanLine className="h-4 w-4" />,
	},
];

// ── main page ──────────────────────────────────────────────────────────────

export default function Page() {
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
		true,
	);

	const [mode, setMode] = useState<BitMode>("local-llm");
	const [bit, setBit] = useState<IBit>(DEFAULT_BIT);
	const [loading, setLoading] = useState(false);
	const [projection, setProjection] = useState<IBit | undefined>(undefined);
	const [textEmbeddingModel, setTextEmbeddingModel] = useState<
		IBit | undefined
	>(undefined);
	const [tokenizer, setTokenizer] = useState<IBit | undefined>(undefined);
	const [tokenizerConfig, setTokenizerConfig] = useState<IBit | undefined>(
		undefined,
	);
	const [specialTokensMap, setSpecialTokensMap] = useState<IBit | undefined>(
		undefined,
	);
	const [config, setConfig] = useState<IBit | undefined>(undefined);
	const [imageEmbeddingPreprocessor, setImageEmbeddingPreprocessor] = useState<
		IBit | undefined
	>(undefined);
	const [imageEmbeddingConfig, setImageEmbeddingConfig] = useState<
		IBit | undefined
	>(undefined);
	const [ttsAssets, setTtsAssets] = useState<TtsAssetDraft[]>([]);
	const [progress, setProgress] = useState(0);
	const [progressDownloaded, setProgressDownloaded] = useState<number | null>(
		null,
	);
	const [progressTotal, setProgressTotal] = useState<number | null>(null);
	const [progressLabel, setProgressLabel] = useState<string | null>(null);
	const [progressBit, setProgressBit] = useState<IBit | undefined>(undefined);
	const lastSampleRef = useRef<{ t: number; downloaded: number } | null>(null);
	const [speedBps, setSpeedBps] = useState(0);
	const [etaSec, setEtaSec] = useState<number | null>(null);

	// derived
	const bitType: IBitTypes = (() => {
		if (mode === "local-llm" || mode === "hosted-llm") return IBitTypes.Llm;
		if (mode === "vlm") return IBitTypes.Vlm;
		if (mode === "tts") return IBitTypes.Tts;
		if (mode === "hosted-stt") return IBitTypes.Stt;
		if (mode === "embedding") return IBitTypes.Embedding;
		if (mode === "image-embedding") return IBitTypes.ImageEmbedding;
		return IBitTypes.ObjectDetection;
	})();

	const isHostedMode = mode === "hosted-llm" || mode === "hosted-stt";

	const isHostedModel =
		(bit.type === IBitTypes.Llm || bit.type === IBitTypes.Stt) &&
		isHostedProviderName(
			(bit.parameters as ILlmParameters | undefined)?.provider?.provider_name,
		);

	function getDefaultBit(type: IBitTypes): IBit {
		return {
			...DEFAULT_BIT,
			id: createId(),
			parameters:
				type === IBitTypes.Llm || type === IBitTypes.Vlm
					? createDefaultLlmParameters(isHostedMode ? "Hosted" : "Local")
					: {},
			type,
		};
	}

	function getDefaultTtsAssetBit(): IBit {
		return {
			...getDefaultBit(IBitTypes.File),
			download_link: "",
			file_name: "",
			parameters: {},
		};
	}

	// ── upload helper ──────────────────────────────────────────────────────

	const uploadBit = useCallback(
		async (bitToUpload: IBit): Promise<IBit> => {
			if (!profile.data) throw new Error("User profile is not available");
			let finalBit = { ...bitToUpload };
			let receivedFinalBit = false;
			await backend.apiState.stream(
				profile.data,
				`admin/bit/${bitToUpload.id}`,
				{ method: "PUT", body: JSON.stringify(bitToUpload) },
				(data: Record<string, unknown>) => {
					const dlRaw = data?.downloaded;
					const totRaw = data?.total;
					const pRaw = data?.percent;
					const downloaded =
						typeof dlRaw === "string"
							? Number(dlRaw)
							: (dlRaw as number | undefined);
					const total =
						typeof totRaw === "string"
							? Number(totRaw)
							: (totRaw as number | undefined);
					let percent =
						typeof pRaw === "string"
							? Number(pRaw)
							: (pRaw as number | undefined);
					if (
						(percent == null || !Number.isFinite(percent)) &&
						typeof downloaded === "number" &&
						typeof total === "number" &&
						total > 0
					) {
						percent = (downloaded / total) * 100;
					}
					if (typeof percent === "number" && Number.isFinite(percent)) {
						setProgress(Math.max(0, Math.min(100, percent)));
					}
					if (typeof downloaded === "number") setProgressDownloaded(downloaded);
					if (typeof total === "number") setProgressTotal(total);
					if (typeof downloaded === "number") {
						const now = Date.now();
						const last = lastSampleRef.current;
						if (last) {
							const dt = (now - last.t) / 1000;
							if (dt >= 0.25 && downloaded >= last.downloaded) {
								const bps = (downloaded - last.downloaded) / dt;
								if (Number.isFinite(bps)) {
									setSpeedBps(bps);
									if (
										typeof total === "number" &&
										total > downloaded &&
										bps > 0
									) {
										setEtaSec((total - downloaded) / bps);
									}
								}
								lastSampleRef.current = { t: now, downloaded };
							}
						} else {
							lastSampleRef.current = { t: now, downloaded };
						}
					}
					if (data?.id) {
						finalBit = data as unknown as IBit;
						receivedFinalBit = true;
					}
				},
			);
			if (!receivedFinalBit) {
				throw new Error("Bit upload did not complete");
			}
			return finalBit;
		},
		[backend.apiState, profile.data],
	);

	// ── prefill helpers ────────────────────────────────────────────────────

	const prefillLLM = useCallback(async () => {
		if (
			isHostedModel ||
			!bit.download_link ||
			bit.download_link === "" ||
			(bit.type !== IBitTypes.Llm &&
				bit.type !== IBitTypes.Vlm &&
				bit.type !== IBitTypes.Stt)
		)
			return;
		setLoading(true);
		try {
			const size = await getModelSize(bit.download_link);
			if (!bit.repository || bit.repository === "")
				bit.repository = bit.download_link.split("/resolve/")[0];
			bit.repository =
				(await getOriginalRepo(bit.repository)) ?? bit.repository;
			const userInfo = await getUserInfo(bit.repository);
			const license = await getModelLicense(bit.repository);
			const tags = await getModelTags(bit.repository);
			const modelName = await getModelName(bit.repository);
			const parameters: ILlmParameters = {
				...bit.parameters,
				context_length: (await getContextLength(bit.download_link)) || 2048,
			};
			setBit((old) => ({
				...old,
				meta: {
					...old.meta,
					en: {
						...old.meta.en,
						icon: userInfo.avatarUrl,
						tags,
						name: modelName || old.meta.en.name,
					},
				},
				file_name: old.download_link?.split("/").pop()?.split("?")[0] || "",
				repository: bit.repository,
				authors: [userInfo.authorUrl],
				license,
				size,
				parameters,
			}));
		} catch (error) {
			console.error("Error pre-filling LLM parameters:", error);
		} finally {
			setLoading(false);
		}
	}, [bit, isHostedModel]);

	const prefillEmbeddingModel = useCallback(async () => {
		if (
			!bit.download_link ||
			bit.download_link === "" ||
			(bit.type !== IBitTypes.Embedding &&
				bit.type !== IBitTypes.ImageEmbedding)
		)
			return;
		setLoading(true);
		try {
			const size = await getModelSize(bit.download_link);
			if (!bit.repository || bit.repository === "")
				bit.repository = bit.download_link.split("/resolve/")[0];
			bit.repository =
				(await getOriginalRepo(bit.repository)) ?? bit.repository;
			const userInfo = await getUserInfo(bit.repository);
			const license = await getModelLicense(bit.repository);
			const tags = await getModelTags(bit.repository);
			const modelName = await getModelName(bit.repository);
			setBit((old) => ({
				...old,
				meta: {
					...old.meta,
					en: {
						...old.meta.en,
						icon: userInfo.avatarUrl,
						tags,
						name: modelName || old.meta.en.name,
					},
				},
				file_name: old.download_link?.split("/").pop()?.split("?")[0] || "",
				repository: bit.repository,
				authors: [userInfo.authorUrl],
				license,
				size,
			}));

			if (
				bit.type === IBitTypes.Embedding ||
				(bit.type === IBitTypes.ImageEmbedding &&
					textEmbeddingModel?.download_link)
			) {
				const downloadLink =
					bit.type === IBitTypes.ImageEmbedding
						? textEmbeddingModel?.download_link
						: bit.download_link;
				let repo = bit.repository;
				if (downloadLink && downloadLink !== "")
					repo = (await getOriginalRepo(downloadLink)) ?? repo;
				const [
					tokenizerUrl,
					tokenizerConfigUrl,
					specialTokensMapUrl,
					configUrl,
				] = await Promise.all([
					guessedModelLink(downloadLink, "tokenizer.json"),
					guessedModelLink(downloadLink, "tokenizer_config.json"),
					guessedModelLink(downloadLink, "special_tokens_map.json"),
					guessedModelLink(downloadLink, "config.json"),
				]);
				setTokenizer((old) => ({
					...(old || getDefaultBit(IBitTypes.Tokenizer)),
					download_link: tokenizerUrl,
				}));
				setTokenizerConfig((old) => ({
					...(old || getDefaultBit(IBitTypes.TokenizerConfig)),
					download_link: tokenizerConfigUrl,
				}));
				setSpecialTokensMap((old) => ({
					...(old || getDefaultBit(IBitTypes.SpecialTokensMap)),
					download_link: specialTokensMapUrl,
				}));
				setConfig((old) => ({
					...(old || getDefaultBit(IBitTypes.Config)),
					download_link: configUrl,
				}));
				if (textEmbeddingModel)
					setTextEmbeddingModel((old) => ({
						...(old || getDefaultBit(IBitTypes.Embedding)),
						repository: repo,
					}));
			}

			if (bit.type === IBitTypes.ImageEmbedding) {
				const [preprocessorUrl, imgConfigUrl] = await Promise.all([
					guessedModelLink(bit.download_link, "preprocessor_config.json"),
					guessedModelLink(bit.download_link, "config.json"),
				]);
				setImageEmbeddingPreprocessor((old) => ({
					...(old || getDefaultBit(IBitTypes.PreprocessorConfig)),
					download_link: preprocessorUrl,
				}));
				setImageEmbeddingConfig((old) => ({
					...(old || getDefaultBit(IBitTypes.Config)),
					download_link: imgConfigUrl,
				}));
			}
		} catch (error) {
			console.error("Error pre-filling embedding model:", error);
		} finally {
			setLoading(false);
		}
	}, [bit, textEmbeddingModel]);

	// ── reset on mode change ───────────────────────────────────────────────

	function setDefaultDependencies(type: IBitTypes) {
		if (type === IBitTypes.Vlm) {
			setProjection(getDefaultBit(IBitTypes.Projection));
			setTokenizer(undefined);
			setTokenizerConfig(undefined);
			setSpecialTokensMap(undefined);
			setConfig(undefined);
			setImageEmbeddingPreprocessor(undefined);
			setImageEmbeddingConfig(undefined);
			setTextEmbeddingModel(undefined);
			return;
		}
		if (type === IBitTypes.Embedding) {
			setProjection(undefined);
			setTokenizer(getDefaultBit(IBitTypes.Tokenizer));
			setTokenizerConfig(getDefaultBit(IBitTypes.TokenizerConfig));
			setSpecialTokensMap(getDefaultBit(IBitTypes.SpecialTokensMap));
			setConfig(getDefaultBit(IBitTypes.Config));
			setImageEmbeddingPreprocessor(undefined);
			setImageEmbeddingConfig(undefined);
			setTextEmbeddingModel(undefined);
			return;
		}
		if (type === IBitTypes.ImageEmbedding) {
			setProjection(undefined);
			setTokenizer(getDefaultBit(IBitTypes.Tokenizer));
			setTokenizerConfig(getDefaultBit(IBitTypes.TokenizerConfig));
			setSpecialTokensMap(getDefaultBit(IBitTypes.SpecialTokensMap));
			setConfig(getDefaultBit(IBitTypes.Config));
			setImageEmbeddingPreprocessor(
				getDefaultBit(IBitTypes.PreprocessorConfig),
			);
			setImageEmbeddingConfig(getDefaultBit(IBitTypes.Config));
			setTextEmbeddingModel(getDefaultBit(IBitTypes.Embedding));
			return;
		}
		if (type === IBitTypes.Tts) {
			setProjection(undefined);
			setTokenizer(undefined);
			setTokenizerConfig(undefined);
			setSpecialTokensMap(undefined);
			setConfig(undefined);
			setImageEmbeddingPreprocessor(undefined);
			setImageEmbeddingConfig(undefined);
			setTextEmbeddingModel(undefined);
			return;
		}
		setProjection(undefined);
		setTokenizer(undefined);
		setTokenizerConfig(undefined);
		setSpecialTokensMap(undefined);
		setConfig(undefined);
		setImageEmbeddingPreprocessor(undefined);
		setImageEmbeddingConfig(undefined);
		setTextEmbeddingModel(undefined);
		setTtsAssets([]);
	}

	useEffect(() => {
		const providerName = isHostedMode ? "Hosted" : "Local";
		const ttsAssetLayout =
			bitType === IBitTypes.Tts
				? defaultTtsAssetLayout(
						DEFAULT_TTS_PARAMETERS.model_type,
						getDefaultTtsAssetBit,
					)
				: [];
		setBit((old) => {
			const nextBit = {
				...old,
				id: createId(),
				type: bitType,
				parameters:
					bitType === IBitTypes.Llm ||
					bitType === IBitTypes.Vlm ||
					bitType === IBitTypes.Stt
						? createDefaultLlmParameters(providerName)
						: bitType === IBitTypes.Tts
							? DEFAULT_TTS_PARAMETERS
							: bitType === IBitTypes.Embedding ||
									bitType === IBitTypes.ImageEmbedding
								? { ...DEFAULT_EMBEDDING_PARAMETERS }
								: {},
				download_link:
					isHostedMode || bitType === IBitTypes.Tts ? "" : old.download_link,
				file_name:
					isHostedMode || bitType === IBitTypes.Tts ? "" : old.file_name,
				size: isHostedMode || bitType === IBitTypes.Tts ? 0 : old.size,
				name: "",
			};

			if (bitType === IBitTypes.Tts) {
				return applyTtsModelPreset(
					nextBit,
					DEFAULT_TTS_PARAMETERS.model_type,
					ttsAssetLayout,
				);
			}

			return nextBit;
		});
		if (bitType === IBitTypes.Tts) setTtsAssets(ttsAssetLayout);
		setDefaultDependencies(bitType);
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, [mode]);

	useEffect(() => {
		if (
			(bit.type === IBitTypes.Llm ||
				bit.type === IBitTypes.Vlm ||
				bit.type === IBitTypes.Stt) &&
			!isHostedModel
		) {
			prefillLLM();
		}
		if (
			bit.type === IBitTypes.Embedding ||
			bit.type === IBitTypes.ImageEmbedding
		) {
			prefillEmbeddingModel();
		}
	}, [
		bit.download_link,
		bit.type,
		isHostedModel,
		textEmbeddingModel?.download_link,
	]);

	// ── submit ─────────────────────────────────────────────────────────────

	const handleSubmit = useCallback(async () => {
		if (!profile.data) {
			toast.error("You must be logged in to add a bit.");
			return;
		}
		setLoading(true);
		try {
			let dependencies: IBit[] = [];

			if (bit.type === IBitTypes.Embedding) {
				if (!tokenizer || !tokenizerConfig || !specialTokensMap || !config) {
					throw new Error("Missing required dependencies for Embedding model");
				}
				const tokReg = await uploadBit(mergeBitParameters(tokenizer, bit));
				dependencies.push(tokReg);
				const tokCfgReg = await uploadBit(
					mergeBitParameters(tokenizerConfig, bit),
				);
				dependencies.push(tokCfgReg);
				const stmReg = await uploadBit(
					mergeBitParameters(specialTokensMap, bit),
				);
				dependencies.push(stmReg);
				const cfgReg = await uploadBit(mergeBitParameters(config, bit));
				dependencies.push(cfgReg);
				const response = await uploadBit({
					...bit,
					dependencies: dependencies.map((d) => `${d.hub}:${d.id}`),
				});
				await backend.apiState.put(
					profile.data,
					`admin/bit/${response.id}/en`,
					bit.meta.en,
				);
			}

			if (bit.type === IBitTypes.ImageEmbedding) {
				if (
					!textEmbeddingModel ||
					!tokenizer ||
					!tokenizerConfig ||
					!specialTokensMap ||
					!config ||
					!imageEmbeddingPreprocessor ||
					!imageEmbeddingConfig
				) {
					throw new Error(
						"Missing required dependencies for Image Embedding model",
					);
				}
				textEmbeddingModel.license = bit.license;
				textEmbeddingModel.authors = bit.authors;
				const tokReg = await uploadBit(
					mergeBitParameters(tokenizer, textEmbeddingModel),
				);
				dependencies.push(tokReg);
				const tokCfgReg = await uploadBit(
					mergeBitParameters(tokenizerConfig, textEmbeddingModel),
				);
				dependencies.push(tokCfgReg);
				const stmReg = await uploadBit(
					mergeBitParameters(specialTokensMap, textEmbeddingModel),
				);
				dependencies.push(stmReg);
				const cfgReg = await uploadBit(
					mergeBitParameters(config, textEmbeddingModel),
				);
				dependencies.push(cfgReg);
				const textEmbReg = await uploadBit({
					...textEmbeddingModel,
					license: bit.license,
					authors: bit.authors,
					dependencies: dependencies.map((d) => `${d.hub}:${d.id}`),
				});
				dependencies = [textEmbReg];
				const ppReg = await uploadBit(
					mergeBitParameters(imageEmbeddingPreprocessor, bit),
				);
				dependencies.push(ppReg);
				const imgCfgReg = await uploadBit(
					mergeBitParameters(imageEmbeddingConfig, bit),
				);
				dependencies.push(imgCfgReg);
				const response = await uploadBit({
					...bit,
					dependencies: dependencies.map((d) => `${d.hub}:${d.id}`),
				});
				await backend.apiState.put(
					profile.data,
					`admin/bit/${response.id}/en`,
					bit.meta.en,
				);
			}

			if (bit.type === IBitTypes.Vlm) {
				if (!projection) throw new Error("Projection is required for VLM");
				const projReg = await uploadBit({
					...projection,
					license: bit.license,
					authors: bit.authors,
					repository: bit.repository,
				});
				dependencies.push(projReg);
			}

			if (bit.type === IBitTypes.Tts) {
				if (ttsAssets.length === 0) {
					throw new Error("TTS models require at least one asset");
				}

				const registeredAssets = [];
				for (const asset of ttsAssets) {
					if (!asset.relativePath) {
						throw new Error("Every TTS asset needs a model-relative path");
					}
					if (asset.required && !asset.bit.download_link) {
						throw new Error(
							`Missing download link for required TTS asset ${asset.relativePath}`,
						);
					}
					if (!asset.required && !asset.bit.download_link) continue;

					const registered = await uploadBit(
						mergeTtsAssetParameters(asset.bit, bit),
					);
					dependencies.push(registered);
					registeredAssets.push({
						bit: `${registered.hub}:${registered.id}`,
						relative_path: asset.relativePath,
						required: asset.required,
					});
				}

				const response = await uploadBit({
					...bit,
					download_link: bit.download_link || null,
					file_name: bit.file_name || null,
					size: bit.download_link ? bit.size : 0,
					dependencies: dependencies.map((d) => `${d.hub}:${d.id}`),
					parameters: {
						...bit.parameters,
						assets: registeredAssets,
					},
				});
				await backend.apiState.put(
					profile.data,
					`admin/bit/${response.id}/en`,
					bit.meta.en,
				);
			}

			if (
				bit.type === IBitTypes.Vlm ||
				bit.type === IBitTypes.Llm ||
				bit.type === IBitTypes.Stt
			) {
				const response = await uploadBit({
					...bit,
					dependencies: dependencies.map((d) => `${d.hub}:${d.id}`),
				});
				await backend.apiState.put(
					profile.data,
					`admin/bit/${response.id}/en`,
					bit.meta.en,
				);
			}

			toast.success("Bit added successfully");
			setBit({ ...DEFAULT_BIT, id: createId() });
			setProjection(undefined);
			setTokenizer(undefined);
			setTokenizerConfig(undefined);
			setSpecialTokensMap(undefined);
			setConfig(undefined);
			setImageEmbeddingPreprocessor(undefined);
			setImageEmbeddingConfig(undefined);
			setTextEmbeddingModel(undefined);
			setTtsAssets([]);
		} catch (error: unknown) {
			toast.error(
				`Failed to add bit: ${error instanceof Error ? error.message : error}`,
			);
		} finally {
			setLoading(false);
		}
	}, [
		bit,
		backend.apiState,
		config,
		imageEmbeddingConfig,
		imageEmbeddingPreprocessor,
		profile.data,
		projection,
		specialTokensMap,
		textEmbeddingModel,
		ttsAssets,
		tokenizer,
		tokenizerConfig,
		uploadBit,
	]);

	// ── render ─────────────────────────────────────────────────────────────

	return (
		<main className="flex grow h-full min-h-0 bg-background overflow-hidden flex-col w-full">
			<div className="flex-1 min-h-0 overflow-y-auto p-6">
				<div className="space-y-6">
					<div>
						<h1 className="text-3xl font-bold">Add New Bit</h1>
						<p className="text-muted-foreground">
							Register a new model or asset to the registry.
						</p>
					</div>

					{/* mode selector */}
					<Card>
						<CardHeader>
							<CardTitle>Bit Type</CardTitle>
							<CardDescription>
								Choose what kind of bit you want to add.
							</CardDescription>
						</CardHeader>
						<CardContent>
							<div className="flex flex-wrap gap-2">
								{MODES.map(({ id, label, icon }) => (
									<Button
										key={id}
										variant={mode === id ? "default" : "outline"}
										size="sm"
										onClick={() => setMode(id)}
										className="gap-2"
									>
										{icon}
										{label}
									</Button>
								))}
							</div>
						</CardContent>
					</Card>

					{/* hosted llm — quick form */}
					{isHostedMode ? (
						<HostedLLMForm
							bit={bit}
							setBit={setBit}
							loading={loading}
							onSubmit={handleSubmit}
						/>
					) : (
						<>
							{/* download link for non-hosted file-backed models */}
							{bit.type !== IBitTypes.Tts ? (
								<div className="flex flex-row items-center gap-2 max-w-3xl">
									{loading ? (
										<Loader2Icon className="w-4 h-4 animate-spin shrink-0" />
									) : null}
									<Input
										disabled={loading}
										value={bit.download_link ?? ""}
										onChange={(e) =>
											setBit((old) => ({
												...old,
												download_link: e.target.value.trim(),
											}))
										}
										placeholder="File URL (ONNX/GGUF/Safetensors)"
									/>
								</div>
							) : null}

							{/* model-type-specific configuration */}
							{bit.type === IBitTypes.Llm ||
							bit.type === IBitTypes.Vlm ||
							bit.type === IBitTypes.Stt ? (
								<>
									<LLMConfiguration
										bit={bit}
										setBit={setBit}
										isHosted={false}
									/>
									<Separator className="my-4" />
								</>
							) : null}
							{bit.type === IBitTypes.Vlm && projection ? (
								<>
									<DependencyConfiguration
										defaultBit={getDefaultBit(IBitTypes.Projection)}
										name="Projection"
										bit={projection}
										setBit={setProjection}
									/>
									<Separator className="my-4" />
								</>
							) : null}
							{bit.type === IBitTypes.Tts ? (
								<>
									<TTSConfiguration
										bit={bit}
										setBit={setBit}
										assetBits={ttsAssets}
										setAssetBits={setTtsAssets}
										createAssetBit={getDefaultTtsAssetBit}
									/>
									<Separator className="my-4" />
								</>
							) : null}
							{bit.type === IBitTypes.Embedding ||
							bit.type === IBitTypes.ImageEmbedding ? (
								<>
									<div className="flex flex-col items-start gap-6 w-full max-w-5xl">
										<EmbeddingConfiguration bit={bit} setBit={setBit} />
										{textEmbeddingModel && (
											<DependencyConfiguration
												defaultBit={getDefaultBit(IBitTypes.Embedding)}
												name="Relevant Text Embedding Model"
												bit={textEmbeddingModel}
												setBit={setTextEmbeddingModel}
											/>
										)}
										{tokenizer && (
											<DependencyConfiguration
												defaultBit={getDefaultBit(IBitTypes.Tokenizer)}
												name="Tokenizer"
												bit={tokenizer}
												setBit={setTokenizer}
											/>
										)}
										{tokenizerConfig && (
											<DependencyConfiguration
												defaultBit={getDefaultBit(IBitTypes.TokenizerConfig)}
												name="Tokenizer Config"
												bit={tokenizerConfig}
												setBit={setTokenizerConfig}
											/>
										)}
										{specialTokensMap && (
											<DependencyConfiguration
												defaultBit={getDefaultBit(IBitTypes.SpecialTokensMap)}
												name="Special Tokens Map"
												bit={specialTokensMap}
												setBit={setSpecialTokensMap}
											/>
										)}
										{config && (
											<DependencyConfiguration
												defaultBit={getDefaultBit(IBitTypes.Config)}
												name="Config"
												bit={config}
												setBit={setConfig}
											/>
										)}
										{imageEmbeddingPreprocessor && (
											<DependencyConfiguration
												defaultBit={getDefaultBit(IBitTypes.PreprocessorConfig)}
												name="Image Embedding Preprocessor"
												bit={imageEmbeddingPreprocessor}
												setBit={setImageEmbeddingPreprocessor}
											/>
										)}
										{imageEmbeddingConfig && (
											<DependencyConfiguration
												defaultBit={getDefaultBit(IBitTypes.Config)}
												name="Image Embedding Config"
												bit={imageEmbeddingConfig}
												setBit={setImageEmbeddingConfig}
											/>
										)}
									</div>
									<Separator className="my-4" />
								</>
							) : null}

							<MetaConfiguration bit={bit} setBit={setBit} />

							{(progress > 0 || loading) && (
								<UploadProgressCard
									percent={progress}
									downloaded={progressDownloaded ?? undefined}
									total={progressTotal ?? undefined}
									label={progressLabel ?? undefined}
									bit={progressBit}
									speedBps={speedBps}
									etaSec={etaSec}
								/>
							)}

							<Button
								className="mt-4 w-full max-w-5xl"
								onClick={handleSubmit}
								disabled={loading}
							>
								{loading ? (
									<Loader2Icon className="w-4 h-4 animate-spin mr-2" />
								) : null}
								Add Bit
							</Button>
						</>
					)}
				</div>
			</div>
		</main>
	);
}

// ── helpers ────────────────────────────────────────────────────────────────

function mergeBitParameters(bit: IBit, parent: IBit): IBit {
	return {
		...bit,
		license: parent.license,
		authors: parent.authors,
		repository: parent.repository,
	};
}

function mergeTtsAssetParameters(bit: IBit, parent: IBit): IBit {
	return {
		...bit,
		license: bit.license || parent.license,
		authors: bit.authors?.length ? bit.authors : parent.authors,
		repository: bit.repository || parent.repository,
	};
}

function UploadProgressCard(props: {
	percent: number;
	downloaded?: number;
	total?: number;
	label?: string;
	bit?: IBit;
	speedBps?: number;
	etaSec?: number | null;
}) {
	const { percent, downloaded, total, label, bit, speedBps, etaSec } = props;
	return (
		<Card className="mt-4 w-full max-w-5xl">
			<CardHeader className="flex flex-row items-center justify-between space-y-0">
				<div className="flex items-center gap-2 text-sm text-muted-foreground">
					<UploadCloudIcon className="h-4 w-4" />
					<span>{label ?? "Uploading…"}</span>
				</div>
				{bit ? (
					<div className="flex items-center gap-3 text-xs text-muted-foreground">
						<span className="inline-flex items-center gap-1">
							<PackageIcon className="h-3 w-3" />
							{bit.type}
						</span>
						<span className="inline-flex items-center gap-1">
							<FileTextIcon className="h-3 w-3" />
							{bit.file_name || bit.name || bit.id}
						</span>
						{bit.version ? (
							<span className="inline-flex items-center gap-1">
								<HashIcon className="h-3 w-3" />
								{bit.version}
							</span>
						) : null}
					</div>
				) : null}
			</CardHeader>
			<CardContent className="space-y-2">
				<div className="flex items-center gap-3">
					<Progress
						className="flex-1"
						value={Number.isFinite(percent) ? percent : 0}
					/>
					<span className="w-12 text-right text-sm tabular-nums">
						{Math.round(Number.isFinite(percent) ? percent : 0)}%
					</span>
				</div>
				<div className="flex flex-wrap items-center gap-x-6 gap-y-1 text-xs text-muted-foreground">
					<span className="inline-flex items-center gap-1">
						<GaugeIcon className="h-3 w-3" />
						{formatBytes(downloaded ?? 0)}
						{typeof total === "number" ? ` / ${formatBytes(total)}` : ""}
					</span>
					{speedBps && speedBps > 0 ? (
						<span className="inline-flex items-center gap-1">
							<UploadCloudIcon className="h-3 w-3" />
							{formatBytes(speedBps)}/s
						</span>
					) : null}
					{etaSec != null && Number.isFinite(etaSec) ? (
						<span className="inline-flex items-center gap-1">
							<TimerIcon className="h-3 w-3" />~{formatTime(etaSec)}
						</span>
					) : null}
				</div>
			</CardContent>
		</Card>
	);
}

function formatBytes(bytes: number): string {
	if (!Number.isFinite(bytes) || bytes < 0) return "0 B";
	const units = ["B", "KB", "MB", "GB", "TB"];
	const i = Math.min(
		Math.floor(Math.log(bytes || 1) / Math.log(1024)),
		units.length - 1,
	);
	const value = bytes / 1024 ** i;
	return `${value >= 100 ? value.toFixed(0) : value >= 10 ? value.toFixed(1) : value.toFixed(2)} ${units[i]}`;
}

function formatTime(sec: number): string {
	if (!Number.isFinite(sec) || sec < 0) return "—";
	const s = Math.round(sec);
	const h = Math.floor(s / 3600);
	const m = Math.floor((s % 3600) / 60);
	const r = s % 60;
	if (h > 0) return `${h}h ${m}m`;
	if (m > 0) return `${m}m ${r}s`;
	return `${r}s`;
}
