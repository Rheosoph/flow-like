"use client";

import {
	Button,
	Card,
	CardContent,
	CardHeader,
	type HuggingFaceGgufSelectionOptions,
	type HuggingFaceModelImport,
	type IBit,
	IBitTypes,
	type IEmbeddingModelParameters,
	type ILlmParameters,
	IPooling,
	ISttDTypePreference,
	type ISttModelParameters,
	ISttModelType,
	ISttRuntimePreference,
	ITtsDTypePreference,
	type ITtsModelParameters,
	ITtsModelType,
	ITtsRuntimePreference,
	Input,
	Progress,
	Separator,
	applyHuggingFaceMlxImportToBit,
	buildMlxModelRootBit,
	createHuggingFaceGgufAdminDraft,
	createHuggingFaceMlxAssetBits,
	inferMlxAssetBitType,
	nowSystemTime,
	prepareMlxAssetBit,
	useBackend,
	useInvoke,
	validateMlxModelAssets,
} from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { createId } from "@paralleldrive/cuid2";
import {
	FileTextIcon,
	GaugeIcon,
	HashIcon,
	Loader2Icon,
	PackageIcon,
	TimerIcon,
	UploadCloudIcon,
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
import { HuggingFaceModelImporter, MlxAssetsConfiguration } from "./mlx-assets";
import {
	STTConfiguration,
	type SttAssetDraft,
	applySttModelPreset,
	defaultSttAssetLayout,
} from "./stt";
import {
	TTSConfiguration,
	type TtsAssetDraft,
	applyTtsModelPreset,
	defaultTtsAssetLayout,
} from "./tts";

const DEFAULT_LLM_PARAMETERS: ILlmParameters = {
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
		provider_name: "Local",
		model_id: null,
		version: null,
	},
};

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

const DEFAULT_STT_PARAMETERS: ISttModelParameters = {
	assets: [],
	default_language: "auto",
	dtype: ISttDTypePreference.Auto,
	languages: [],
	model_type: ISttModelType.WhisperLargeV3Turbo,
	provider: {
		provider_name: "local:any-speech-to-text",
		model_id: null,
		version: null,
	},
	runtime: ISttRuntimePreference.Auto,
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

function getDefaultMlxAssetBit(fileName = ""): IBit {
	return {
		...DEFAULT_BIT,
		id: createId(),
		type: inferMlxAssetBitType(fileName),
		download_link: "",
		file_name: fileName,
		parameters: {},
	};
}

export default function Page() {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
		true,
	);
	const [type, setType] = useState<IBitTypes>(IBitTypes.Llm);
	const [localStt, setLocalStt] = useState<boolean>(false);
	const [bit, setBit] = useState<IBit>(DEFAULT_BIT);
	const [loading, setLoading] = useState<boolean>(false);
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
	const [sttAssets, setSttAssets] = useState<SttAssetDraft[]>([]);
	const [mlxAssets, setMlxAssets] = useState<IBit[]>([]);
	const skipNextTypeResetRef = useRef(false);
	const importedGgufDownloadRef = useRef<string | null>(null);
	const [progress, setProgress] = useState<number>(0);

	const [progressDownloaded, setProgressDownloaded] = useState<number | null>(
		null,
	);
	const [progressTotal, setProgressTotal] = useState<number | null>(null);
	const [progressLabel, setProgressLabel] = useState<string | null>(null);
	const [progressBit, setProgressBit] = useState<IBit | undefined>(undefined);
	const lastSampleRef = useRef<{ t: number; downloaded: number } | null>(null);
	const [speedBps, setSpeedBps] = useState<number>(0);
	const [etaSec, setEtaSec] = useState<number | null>(null);

	function getDefaultBit(type: IBitTypes): IBit {
		return {
			...DEFAULT_BIT,
			id: createId(),
			parameters: {},
			type: type,
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

	function getDefaultSttAssetBit(): IBit {
		return {
			...getDefaultBit(IBitTypes.File),
			download_link: "",
			file_name: "",
			parameters: {},
		};
	}

	const applyHuggingFaceRepositoryImport = useCallback(
		(
			imported: HuggingFaceModelImport,
			options?: HuggingFaceGgufSelectionOptions,
		) => {
			if (imported.format === "gguf") {
				const importedDraft = createHuggingFaceGgufAdminDraft(
					bit,
					imported,
					() => getDefaultBit(IBitTypes.Projection),
					options,
				);
				const targetType =
					importedDraft.selection.kind === "vlm"
						? IBitTypes.Vlm
						: IBitTypes.Llm;
				if (type !== targetType) {
					skipNextTypeResetRef.current = true;
					setType(targetType);
				}
				importedGgufDownloadRef.current =
					importedDraft.root.download_link ?? null;
				setBit(importedDraft.root);
				setProjection(importedDraft.projection);
				setMlxAssets([]);
				return;
			}

			const targetType =
				imported.kind === "vlm" ? IBitTypes.Vlm : IBitTypes.Llm;
			if (type !== targetType) {
				skipNextTypeResetRef.current = true;
				setType(targetType);
			}
			importedGgufDownloadRef.current = null;
			setBit((current) => applyHuggingFaceMlxImportToBit(current, imported));
			setMlxAssets(
				createHuggingFaceMlxAssetBits(imported, getDefaultMlxAssetBit),
			);
			setProjection(undefined);
		},
		[bit, type],
	);

	const isLocalSttBit =
		bit.type === IBitTypes.Stt &&
		(bit.parameters as ILlmParameters | undefined)?.provider?.provider_name ===
			"local:any-speech-to-text";
	const isMlxModel =
		(bit.type === IBitTypes.Llm || bit.type === IBitTypes.Vlm) &&
		(bit.parameters as ILlmParameters | undefined)?.provider?.provider_name ===
			"MLX";

	const uploadBit = useCallback(
		async (bit: IBit): Promise<IBit> => {
			if (!profile.data) {
				throw new Error("User profile is not available");
			}

			let finalBit = { ...bit };
			let receivedFinalBit = false;

			await backend.apiState.stream(
				profile.data,
				`admin/bit/${bit.id}`,
				{
					method: "PUT",
					body: JSON.stringify(bit),
				},
				(data: Record<string, unknown>) => {
					console.log("Received data:", data);

					const pRaw = data?.percent;
					const dlRaw = data?.downloaded;
					const totRaw = data?.total;

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
						const clamped = Math.max(0, Math.min(100, percent));
						setProgress(clamped);
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

					// Completed single upload
					if (data?.id) {
						finalBit = data as IBit;
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
			setSttAssets([]);
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
		setSttAssets([]);
	}

	const prefillLLM = useCallback(async () => {
		if (importedGgufDownloadRef.current) {
			if (importedGgufDownloadRef.current === bit.download_link) return;
			importedGgufDownloadRef.current = null;
		}
		if (
			isMlxModel ||
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
			// Repo from Download Link
			if (!bit.repository || bit.repository === "") {
				bit.repository = bit.download_link.split("/resolve/")[0];
			}
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
						tags: tags,
						name: modelName || old.meta.en.name,
					},
				},
				file_name: old.download_link?.split("/").pop()?.split("?")[0] || "",
				repository: bit.repository,
				authors: [userInfo.authorUrl],
				license: license,
				size: size,
				parameters,
			}));
		} catch (error) {
			console.error("Error pre-filling LLM parameters:", error);
		} finally {
			setLoading(false);
		}
	}, [bit, isMlxModel]);

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
			if (!bit.repository || bit.repository === "") {
				bit.repository = bit.download_link.split("/resolve/")[0];
			}
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
						tags: tags,
						name: modelName || old.meta.en.name,
					},
				},
				file_name: old.download_link?.split("/").pop()?.split("?")[0] || "",
				repository: bit.repository,
				authors: [userInfo.authorUrl],
				license: license,
				size: size,
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
				const tokenizer = await guessedModelLink(
					downloadLink,
					"tokenizer.json",
				);
				const tokenizerConfig = await guessedModelLink(
					downloadLink,
					"tokenizer_config.json",
				);
				const specialTokensMap = await guessedModelLink(
					downloadLink,
					"special_tokens_map.json",
				);
				const config = await guessedModelLink(downloadLink, "config.json");

				setTokenizer((old) => ({
					...(old || getDefaultBit(IBitTypes.Tokenizer)),
					download_link: tokenizer,
				}));

				setTokenizerConfig((old) => ({
					...(old || getDefaultBit(IBitTypes.TokenizerConfig)),
					download_link: tokenizerConfig,
				}));

				setSpecialTokensMap((old) => ({
					...(old || getDefaultBit(IBitTypes.SpecialTokensMap)),
					download_link: specialTokensMap,
				}));

				setConfig((old) => ({
					...(old || getDefaultBit(IBitTypes.Config)),
					download_link: config,
				}));

				if (textEmbeddingModel)
					setTextEmbeddingModel((old) => ({
						...(old || getDefaultBit(IBitTypes.Embedding)),
						repository: repo,
					}));
			}

			if (bit.type === IBitTypes.ImageEmbedding) {
				const imageEmbeddingPreprocessor = await guessedModelLink(
					bit.download_link,
					"preprocessor_config.json",
				);
				const imageEmbeddingConfig = await guessedModelLink(
					bit.download_link,
					"config.json",
				);

				setImageEmbeddingPreprocessor((old) => ({
					...(old || getDefaultBit(IBitTypes.PreprocessorConfig)),
					download_link: imageEmbeddingPreprocessor,
				}));

				setImageEmbeddingConfig((old) => ({
					...(old || getDefaultBit(IBitTypes.Config)),
					download_link: imageEmbeddingConfig,
				}));
			}
		} catch (error) {
			console.error("Error pre-filling LLM parameters:", error);
		} finally {
			setLoading(false);
		}
	}, [bit, textEmbeddingModel]);

	useEffect(() => {
		if (skipNextTypeResetRef.current) {
			skipNextTypeResetRef.current = false;
			return;
		}
		importedGgufDownloadRef.current = null;
		if (
			(type === IBitTypes.Llm ||
				type === IBitTypes.Vlm ||
				type === IBitTypes.Stt) &&
			!(type === IBitTypes.Stt && localStt)
		) {
			setBit((old) => ({
				...old,
				type,
				parameters: {
					...DEFAULT_LLM_PARAMETERS,
				},
			}));
			prefillLLM();
		}

		if (type === IBitTypes.Stt && localStt) {
			const assets = defaultSttAssetLayout(
				DEFAULT_STT_PARAMETERS.model_type,
				getDefaultSttAssetBit,
			);
			setSttAssets(assets);
			setBit((old) =>
				applySttModelPreset(
					{
						...old,
						type,
						parameters: DEFAULT_STT_PARAMETERS,
					},
					DEFAULT_STT_PARAMETERS.model_type,
					assets,
				),
			);
		}

		if (type === IBitTypes.Embedding || type === IBitTypes.ImageEmbedding) {
			setBit((old) => ({
				...old,
				type,
				parameters: {
					...DEFAULT_EMBEDDING_PARAMETERS,
				},
			}));

			prefillEmbeddingModel();
		}

		if (type === IBitTypes.Tts) {
			const assets = defaultTtsAssetLayout(
				DEFAULT_TTS_PARAMETERS.model_type,
				getDefaultTtsAssetBit,
			);
			setTtsAssets(assets);
			setBit((old) =>
				applyTtsModelPreset(
					{
						...old,
						type,
						parameters: DEFAULT_TTS_PARAMETERS,
					},
					DEFAULT_TTS_PARAMETERS.model_type,
					assets,
				),
			);
		}

		setMlxAssets([]);
		setDefaultDependencies(type);
	}, [type, localStt]);

	useEffect(() => {
		if (!isMlxModel) return;
		setMlxAssets((current) => {
			if (current.length > 0) return current;
			const requiredFiles = [
				"config.json",
				"tokenizer.json",
				"tokenizer_config.json",
				"model.safetensors",
				...(bit.type === IBitTypes.Vlm ? ["preprocessor_config.json"] : []),
			];
			return requiredFiles.map(getDefaultMlxAssetBit);
		});
		setBit((current) =>
			current.download_link || current.file_name || current.size
				? {
						...current,
						download_link: "",
						file_name: "",
						size: 0,
					}
				: current,
		);
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, [isMlxModel, bit.type]);

	useEffect(() => {
		if (
			(bit.type === IBitTypes.Llm ||
				bit.type === IBitTypes.Vlm ||
				bit.type === IBitTypes.Stt) &&
			!isLocalSttBit
		) {
			prefillLLM();
		}
		if (
			bit.type === IBitTypes.Embedding ||
			bit.type === IBitTypes.ImageEmbedding
		) {
			prefillEmbeddingModel();
		}
	}, [bit.download_link, textEmbeddingModel?.download_link]);

	return (
		<main className="flex grow h-full min-h-0 bg-background overflow-hidden flex-col w-full">
			<div className="flex-1 min-h-0 overflow-y-auto p-4">
				<h1>{`Add a new Bit`}</h1>
				<p className="max-w-screen-md">
					{`This page is for adding new bits, which are the building blocks of extra models available to the user. You can add bits here by providing the necessary information.`}
				</p>
				<div className="max-w-screen-md flex flex-row items-center gap-2 mt-4">
					<button
						className={`p-4 transition-all border bg-card hover:bg-card/80 rounded-lg ${type === IBitTypes.Llm ? "border-primary bg-primary/50 text-primary-foreground" : ""}`}
						onClick={() => setType(IBitTypes.Llm)}
					>
						LLM
					</button>
					<button
						className={`p-4 transition-all border bg-card hover:bg-card/80 rounded-lg ${type === IBitTypes.Vlm ? "border-primary bg-primary/50 text-primary-foreground" : ""}`}
						onClick={() => setType(IBitTypes.Vlm)}
					>
						VLM
					</button>
					<button
						className={`p-4 transition-all border bg-card hover:bg-card/80 rounded-lg ${type === IBitTypes.Tts ? "border-primary bg-primary/50 text-primary-foreground" : ""}`}
						onClick={() => setType(IBitTypes.Tts)}
					>
						TTS
					</button>
					<button
						className={`p-4 transition-all border bg-card hover:bg-card/80 rounded-lg ${type === IBitTypes.Stt && !localStt ? "border-primary bg-primary/50 text-primary-foreground" : ""}`}
						onClick={() => {
							setLocalStt(false);
							setType(IBitTypes.Stt);
						}}
					>
						{t("hostedStt", "Hosted STT")}
					</button>
					<button
						className={`p-4 transition-all border bg-card hover:bg-card/80 rounded-lg ${type === IBitTypes.Stt && localStt ? "border-primary bg-primary/50 text-primary-foreground" : ""}`}
						onClick={() => {
							setLocalStt(true);
							setType(IBitTypes.Stt);
						}}
					>
						{t("localStt", "Local STT")}
					</button>
					<button
						className={`p-4 transition-all border bg-card hover:bg-card/80 rounded-lg ${type === IBitTypes.Embedding ? "border-primary bg-primary/50 text-primary-foreground" : ""}`}
						onClick={() => {
							setLocalStt(false);
							setType(IBitTypes.Embedding);
						}}
					>
						{t("embedding", "Embedding")}
					</button>
					<button
						className={`p-4 transition-all border bg-card hover:bg-card/80 rounded-lg ${type === IBitTypes.ImageEmbedding ? "border-primary bg-primary/50 text-primary-foreground" : ""}`}
						onClick={() => setType(IBitTypes.ImageEmbedding)}
					>
						{t("imageEmbedding", "Image Embedding")}
					</button>
					<button
						className={`p-4 transition-all border bg-card hover:bg-card/80 rounded-lg ${type === IBitTypes.ObjectDetection ? "border-primary bg-primary/50 text-primary-foreground" : ""}`}
						onClick={() => setType(IBitTypes.ObjectDetection)}
					>
						{t("classification", "Classification")}
					</button>
				</div>
				<br />
				{type === IBitTypes.Llm || type === IBitTypes.Vlm ? (
					<>
						<HuggingFaceModelImporter
							disabled={loading}
							onImported={applyHuggingFaceRepositoryImport}
						/>
						<Separator className="my-4" />
					</>
				) : null}
				{bit.type !== IBitTypes.Tts && !isLocalSttBit && !isMlxModel ? (
					<>
						<div className="max-w-screen-lg flex flex-row items-center gap-2 w-full">
							{loading ? (
								<Loader2Icon className="w-4 h-4 animate-spin" rotate={2} />
							) : null}
							<Input
								disabled={loading}
								className="max-w-screen-md"
								value={bit.download_link ?? ""}
								onChange={(e) =>
									setBit((old) => ({
										...old,
										download_link: e.target.value.trim(),
									}))
								}
								placeholder={`File URL (ONNX/GGUF/Safetensors)`}
							/>
						</div>
						<br />
					</>
				) : null}
				{bit.type === IBitTypes.Llm ||
				bit.type === IBitTypes.Vlm ||
				(bit.type === IBitTypes.Stt && !isLocalSttBit) ? (
					<>
						<LLMConfiguration bit={bit} setBit={setBit} />
						<Separator className="my-4" />
					</>
				) : null}
				{isMlxModel ? (
					<>
						<MlxAssetsConfiguration
							assets={mlxAssets}
							setAssets={setMlxAssets}
							createAsset={getDefaultMlxAssetBit}
						/>
						<Separator className="my-4" />
					</>
				) : null}
				{bit.type === IBitTypes.Vlm && projection && !isMlxModel ? (
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
				{isLocalSttBit ? (
					<>
						<STTConfiguration
							bit={bit}
							setBit={setBit}
							assetBits={sttAssets}
							setAssetBits={setSttAssets}
							createAssetBit={getDefaultSttAssetBit}
						/>
						<Separator className="my-4" />
					</>
				) : null}
				{bit.type === IBitTypes.Embedding ||
				bit.type === IBitTypes.ImageEmbedding ? (
					<>
						<div className="flex flex-col items-start gap-6 w-full max-w-screen-lg">
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
						etaSec={etaSec ?? null}
					/>
				)}
				<Button
					className="mt-4 w-full max-w-screen-lg"
					disabled={loading}
					onClick={async () => {
						if (!profile.data) {
							toast.error("You must be logged in to add a bit.");
							return;
						}
						setLoading(true);
						try {
							let dependencies: IBit[] = [];
							if (isMlxModel) {
								const manifestErrors = validateMlxModelAssets(
									mlxAssets,
									bit.type === IBitTypes.Vlm,
								);
								if (manifestErrors.length > 0) {
									throw new Error(manifestErrors.join(". "));
								}

								for (const [assetIndex, asset] of mlxAssets.entries()) {
									setProgress(0);
									setProgressDownloaded(null);
									setProgressTotal(asset.size ?? null);
									setProgressLabel(
										`Uploading MLX file ${assetIndex + 1} of ${mlxAssets.length}: ${asset.file_name}`,
									);
									setProgressBit(asset);
									lastSampleRef.current = null;
									setSpeedBps(0);
									setEtaSec(null);
									const registered = await uploadBit(
										prepareMlxAssetBit(asset, bit),
									);
									dependencies.push(registered);
									setMlxAssets((current) =>
										current.map((currentAsset) =>
											currentAsset.id === asset.id ? registered : currentAsset,
										),
									);
								}
							}
							if (bit.type === IBitTypes.Embedding) {
								if (
									!tokenizer ||
									!tokenizerConfig ||
									!specialTokensMap ||
									!config
								) {
									throw new Error(
										"Missing required dependencies for Embedding model",
									);
								}

								const tokenizerRegistration: IBit = await uploadBit(
									mergeBitParameters(tokenizer, bit),
								);
								dependencies.push(tokenizerRegistration);
								const tokenizerConfigRegistration: IBit = await uploadBit(
									mergeBitParameters(tokenizerConfig, bit),
								);

								dependencies.push(tokenizerConfigRegistration);
								const specialTokensMapRegistration: IBit = await uploadBit(
									mergeBitParameters(specialTokensMap, bit),
								);

								dependencies.push(specialTokensMapRegistration);
								const configRegistration: IBit = await uploadBit(
									mergeBitParameters(config, bit),
								);

								dependencies.push(configRegistration);

								const response: IBit = await uploadBit({
									...bit,
									dependencies: dependencies.map(
										(dep) => `${dep.hub}:${dep.id}`,
									),
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

								const tokenizerRegistration: IBit = await uploadBit(
									mergeBitParameters(tokenizer, textEmbeddingModel),
								);
								dependencies.push(tokenizerRegistration);
								const tokenizerConfigRegistration: IBit = await uploadBit(
									mergeBitParameters(tokenizerConfig, textEmbeddingModel),
								);
								dependencies.push(tokenizerConfigRegistration);
								const specialTokensMapRegistration: IBit = await uploadBit(
									mergeBitParameters(specialTokensMap, textEmbeddingModel),
								);
								dependencies.push(specialTokensMapRegistration);

								const configRegistration: IBit = await uploadBit(
									mergeBitParameters(config, textEmbeddingModel),
								);
								dependencies.push(configRegistration);

								const textEmbeddingModelRegistration: IBit = await uploadBit({
									...textEmbeddingModel,
									license: bit.license,
									authors: bit.authors,
									dependencies: dependencies.map(
										(dep) => `${dep.hub}:${dep.id}`,
									),
								});

								dependencies = [textEmbeddingModelRegistration];

								const imageEmbeddingPreprocessorRegistration: IBit =
									await uploadBit(
										mergeBitParameters(imageEmbeddingPreprocessor, bit),
									);
								dependencies.push(imageEmbeddingPreprocessorRegistration);
								const imageEmbeddingConfigRegistration: IBit = await uploadBit(
									mergeBitParameters(imageEmbeddingConfig, bit),
								);
								dependencies.push(imageEmbeddingConfigRegistration);

								const response: IBit = await uploadBit({
									...bit,
									dependencies: dependencies.map(
										(dep) => `${dep.hub}:${dep.id}`,
									),
								});

								await backend.apiState.put(
									profile.data,
									`admin/bit/${response.id}/en`,
									bit.meta.en,
								);
							}

							if (bit.type === IBitTypes.Vlm && !isMlxModel) {
								if (!projection) {
									throw new Error("Projection is required for VLM");
								}

								const projectionRegistration: IBit = await uploadBit({
									...projection,
									license: bit.license,
									authors: bit.authors,
									repository: bit.repository,
								});
								dependencies.push(projectionRegistration);
							}

							if (bit.type === IBitTypes.Tts) {
								if (ttsAssets.length === 0) {
									throw new Error("TTS models require at least one asset");
								}

								const registeredAssets = [];
								for (const asset of ttsAssets) {
									if (!asset.relativePath) {
										throw new Error(
											"Every TTS asset needs a model-relative path",
										);
									}
									if (asset.required && !asset.bit.download_link) {
										throw new Error(
											`Missing download link for required TTS asset ${asset.relativePath}`,
										);
									}
									if (!asset.required && !asset.bit.download_link) continue;

									const registered = await uploadBit(
										mergeAssetParameters(asset.bit, bit),
									);
									dependencies.push(registered);
									registeredAssets.push({
										bit: `${registered.hub}:${registered.id}`,
										relative_path: asset.relativePath,
										required: asset.required,
									});
								}

								const response: IBit = await uploadBit({
									...bit,
									download_link: bit.download_link || null,
									file_name: bit.file_name || null,
									size: bit.download_link ? bit.size : 0,
									dependencies: dependencies.map(
										(dep) => `${dep.hub}:${dep.id}`,
									),
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

							if (isLocalSttBit) {
								if (sttAssets.length === 0) {
									throw new Error(
										"Local STT models require at least one asset",
									);
								}

								const registeredAssets = [];
								for (const asset of sttAssets) {
									if (!asset.relativePath) {
										throw new Error(
											"Every STT asset needs a model-relative path",
										);
									}
									if (asset.required && !asset.bit.download_link) {
										throw new Error(
											`Missing download link for required STT asset ${asset.relativePath}`,
										);
									}
									if (!asset.required && !asset.bit.download_link) continue;

									const registered = await uploadBit(
										mergeAssetParameters(asset.bit, bit),
									);
									dependencies.push(registered);
									registeredAssets.push({
										bit: `${registered.hub}:${registered.id}`,
										relative_path: asset.relativePath,
										required: asset.required,
									});
								}

								const response: IBit = await uploadBit({
									...bit,
									download_link: null,
									file_name: bit.file_name || null,
									size: 0,
									dependencies: dependencies.map(
										(dep) => `${dep.hub}:${dep.id}`,
									),
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
								(bit.type === IBitTypes.Stt && !isLocalSttBit)
							) {
								const bitToUpload = isMlxModel
									? buildMlxModelRootBit(bit, dependencies)
									: {
											...bit,
											dependencies: dependencies.map(
												(dep) => `${dep.hub}:${dep.id}`,
											),
										};
								if (isMlxModel) {
									setProgressLabel("Registering the virtual MLX model");
									setProgressBit(bit);
								}
								const response: IBit = await uploadBit(bitToUpload);
								await backend.apiState.put(
									profile.data,
									`admin/bit/${response.id}/en`,
									bit.meta.en,
								);
							}

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
							setSttAssets([]);
							setMlxAssets([]);
							setLocalStt(false);
							setType(IBitTypes.Llm);
						} catch (error: unknown) {
							toast.error(
								`Failed to add bit: ${error instanceof Error ? error.message : error}`,
							);
						}
						setLoading(false);
					}}
				>
					{loading ? (
						<Loader2Icon className="w-4 h-4 animate-spin" rotate={2} />
					) : isMlxModel ? (
						t("uploadMlxModel", "Upload MLX model")
					) : (
						t("addBit", "Add Bit")
					)}
				</Button>
			</div>
		</main>
	);
}

function mergeBitParameters(bit: IBit, parent: IBit): IBit {
	return {
		...bit,
		license: parent.license,
		authors: parent.authors,
		repository: parent.repository,
	};
}

function mergeAssetParameters(bit: IBit, parent: IBit): IBit {
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
		<Card className="mt-4 w-full max-w-screen-lg">
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
	const value = bytes / Math.pow(1024, i);
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
