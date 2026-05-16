"use client";

import {
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	type IBit,
	type IBitModelClassification,
	IBitTypes,
	type ILlmParameters,
	type IMetadata,
	type IModelProvider,
	type IVlmParameters,
	Input,
	Label,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Skeleton,
	Textarea,
	useBackend,
	useInvoke,
	useQuery,
	useQueryClient,
} from "@tm9657/flow-like-ui";
import { useDebounce } from "@uidotdev/usehooks";
import { Plus, RefreshCw, Save, Search, Trash2, Wrench } from "lucide-react";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";

const ITEMS_PER_PAGE_OPTIONS = [12, 24, 48, 96];

const ALL_BIT_TYPES = [
	IBitTypes.Llm,
	IBitTypes.Vlm,
	IBitTypes.Tts,
	IBitTypes.Stt,
	IBitTypes.Embedding,
	IBitTypes.ImageEmbedding,
	IBitTypes.File,
	IBitTypes.Media,
	IBitTypes.Template,
	IBitTypes.Tokenizer,
	IBitTypes.TokenizerConfig,
	IBitTypes.SpecialTokensMap,
	IBitTypes.Config,
	IBitTypes.Course,
	IBitTypes.PreprocessorConfig,
	IBitTypes.Projection,
	IBitTypes.Project,
	IBitTypes.Board,
	IBitTypes.Other,
	IBitTypes.ObjectDetection,
];

const MODEL_BIT_TYPES = [IBitTypes.Llm, IBitTypes.Vlm, IBitTypes.Tts, IBitTypes.Stt] as const;
const HOSTED_FILTER = "hosted";
const HOSTED_PROVIDER_OPTIONS = [
	"Hosted",
	"hosted:openrouter",
	"hosted:openai",
	"hosted:anthropic",
	"hosted:azure",
	"hosted:vertex",
] as const;
const DEFAULT_MODEL_CLASSIFICATION: IBitModelClassification = {
	coding: 0.3,
	cost: 0.3,
	creativity: 0.3,
	factuality: 0.3,
	function_calling: 0.3,
	multilinguality: 0.3,
	openness: 0.3,
	reasoning: 0.3,
	safety: 0.3,
	speed: 0.3,
};

type BitFilterValue = "all" | typeof HOSTED_FILTER | IBitTypes;

function asRecord(value: unknown): Record<string, unknown> {
	if (!value || typeof value !== "object" || Array.isArray(value)) {
		return {};
	}
	return value as Record<string, unknown>;
}

function isHostedProviderName(providerName?: null | string) {
	const normalized = providerName?.trim().toLowerCase() ?? "";
	return normalized === "hosted" || normalized.startsWith("hosted:");
}

function getProviderParams(
	provider: IModelProvider | Record<string, unknown> | undefined,
) {
	return asRecord(provider?.params);
}

function normalizeModelParameters(
	parameters: unknown,
): ILlmParameters | IVlmParameters {
	const current = asRecord(parameters);
	const provider = asRecord(current.provider);
	return {
		...current,
		context_length:
			typeof current.context_length === "number"
				? current.context_length
				: 2048,
		model_classification: {
			...DEFAULT_MODEL_CLASSIFICATION,
			...asRecord(current.model_classification),
		},
		provider: {
			...provider,
			provider_name:
				typeof provider.provider_name === "string" && provider.provider_name
					? provider.provider_name
					: "Local",
			model_id:
				typeof provider.model_id === "string" ? provider.model_id : null,
			version: typeof provider.version === "string" ? provider.version : null,
			params: getProviderParams(provider),
		},
	} as ILlmParameters | IVlmParameters;
}

function isHostedBit(bit: IBit | null | undefined) {
	if (
		!bit ||
		!MODEL_BIT_TYPES.includes(bit.type as (typeof MODEL_BIT_TYPES)[number])
	) {
		return false;
	}
	const parameters = normalizeModelParameters(bit.parameters);
	return isHostedProviderName(parameters.provider?.provider_name);
}

function parseDelimitedList(value: string) {
	return value
		.split(/[\n,]/)
		.map((item) => item.trim())
		.filter(Boolean);
}

function cloneBit(bit: IBit): IBit {
	return JSON.parse(JSON.stringify(bit)) as IBit;
}

function getEnglishMeta(bit: IBit): IMetadata {
	const fallbackMeta = Object.values(bit.meta ?? {})[0];
	return (
		bit.meta?.en ??
		fallbackMeta ?? {
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
			created_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
			updated_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
			age_rating: null,
			use_case: "",
			organization_specific_values: null,
			release_notes: "",
		}
	);
}

export default function EditBitsPage() {
	const backend = useBackend();
	const queryClient = useQueryClient();
	const router = useRouter();

	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const [searchTerm, setSearchTerm] = useState("");
	const [selectedType, setSelectedType] = useState<BitFilterValue>("all");
	const [currentPage, setCurrentPage] = useState(1);
	const [itemsPerPage, setItemsPerPage] = useState(24);
	const [selectedId, setSelectedId] = useState<string | null>(null);
	const [draft, setDraft] = useState<IBit | null>(null);
	const [authorsText, setAuthorsText] = useState("");
	const [dependenciesText, setDependenciesText] = useState("");
	const [tagsText, setTagsText] = useState("");
	const [parametersText, setParametersText] = useState("{}");
	const [parametersError, setParametersError] = useState<string | null>(null);
	const [providerParamsText, setProviderParamsText] = useState("{}");
	const [providerParamsError, setProviderParamsError] = useState<string | null>(
		null,
	);
	const [isSaving, setIsSaving] = useState(false);
	const [isDeleting, setIsDeleting] = useState(false);
	const [isRepairingTts, setIsRepairingTts] = useState(false);
	const debouncedSearch = useDebounce(searchTerm, 250);

	const queryParams = useMemo(
		() => ({
			search: debouncedSearch.trim() || undefined,
			limit: itemsPerPage,
			offset: (currentPage - 1) * itemsPerPage,
			bit_types:
				selectedType === "all"
					? undefined
					: selectedType === HOSTED_FILTER
						? [...MODEL_BIT_TYPES]
						: [selectedType as IBitTypes],
		}),
		[debouncedSearch, currentPage, itemsPerPage, selectedType],
	);

	const bits = useQuery<IBit[]>({
		queryKey: ["bit-search", queryParams],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.post<IBit[]>(profile.data, "bit", queryParams);
		},
		enabled: !!profile.data,
	});

	const selectedBit = useQuery<IBit>({
		queryKey: ["bit", selectedId],
		queryFn: async () => {
			if (!profile.data || !selectedId) throw new Error("Bit not selected");
			return backend.apiState.get<IBit>(profile.data, `bit/${selectedId}`);
		},
		enabled: !!profile.data && !!selectedId,
	});

	useEffect(() => {
		if (!bits.data?.length) {
			setSelectedId(null);
			return;
		}

		setSelectedId((current) => {
			if (current && bits.data.some((bit) => bit.id === current)) {
				return current;
			}
			return bits.data[0]?.id ?? null;
		});
	}, [bits.data]);

	useEffect(() => {
		if (!selectedBit.data) return;
		const nextDraft = cloneBit(selectedBit.data);
		nextDraft.meta = {
			...nextDraft.meta,
			en: getEnglishMeta(nextDraft),
		};
		setDraft(nextDraft);
		setAuthorsText((nextDraft.authors ?? []).join(", "));
		setDependenciesText((nextDraft.dependencies ?? []).join("\n"));
		setTagsText((nextDraft.meta.en?.tags ?? []).join(", "));
		setParametersText(JSON.stringify(nextDraft.parameters ?? {}, null, 2));
		if (
			MODEL_BIT_TYPES.includes(
				nextDraft.type as (typeof MODEL_BIT_TYPES)[number],
			)
		) {
			const modelParameters = normalizeModelParameters(nextDraft.parameters);
			setProviderParamsText(
				JSON.stringify(getProviderParams(modelParameters.provider), null, 2),
			);
		} else {
			setProviderParamsText("{}");
		}
		setParametersError(null);
		setProviderParamsError(null);
	}, [selectedBit.data]);

	useEffect(() => {
		setCurrentPage(1);
	}, [debouncedSearch, itemsPerPage, selectedType]);

	const visibleBits = useMemo(
		() =>
			(bits.data ?? []).filter((bit) => {
				const hasMeta =
					bit.meta?.en ?? Object.values(bit.meta ?? {}).length > 0;
				if (!hasMeta) return false;
				if (selectedType === HOSTED_FILTER) {
					return isHostedBit(bit);
				}
				return true;
			}),
		[bits.data, selectedType],
	);

	const hasMorePages = visibleBits.length === itemsPerPage;

	const updateDraft = useCallback(
		<K extends keyof IBit>(key: K, value: IBit[K]) => {
			setDraft((current) => (current ? { ...current, [key]: value } : current));
		},
		[],
	);

	const updateMeta = useCallback(
		<K extends keyof IMetadata>(key: K, value: IMetadata[K]) => {
			setDraft((current) => {
				if (!current) return current;
				return {
					...current,
					meta: {
						...current.meta,
						en: {
							...getEnglishMeta(current),
							[key]: value,
						},
					},
				};
			});
		},
		[],
	);

	const applyParsedParameters = useCallback(
		(parsed: unknown) => {
			setDraft((current) => {
				if (!current) return current;
				return {
					...current,
					parameters: parsed,
				};
			});
			setParametersText(JSON.stringify(parsed ?? {}, null, 2));
			if (
				draft &&
				MODEL_BIT_TYPES.includes(draft.type as (typeof MODEL_BIT_TYPES)[number])
			) {
				const modelParameters = normalizeModelParameters(parsed);
				setProviderParamsText(
					JSON.stringify(getProviderParams(modelParameters.provider), null, 2),
				);
			}
			setParametersError(null);
			setProviderParamsError(null);
		},
		[draft],
	);

	const updateStructuredParameters = useCallback(
		(
			updater: (
				current: ILlmParameters | IVlmParameters,
			) => ILlmParameters | IVlmParameters,
		) => {
			setDraft((current) => {
				if (!current) return current;
				const nextParameters = updater(
					normalizeModelParameters(current.parameters),
				);
				setParametersText(JSON.stringify(nextParameters, null, 2));
				setProviderParamsText(
					JSON.stringify(getProviderParams(nextParameters.provider), null, 2),
				);
				setParametersError(null);
				setProviderParamsError(null);
				return {
					...current,
					parameters: nextParameters,
				};
			});
		},
		[],
	);

	const handleParametersBlur = useCallback(() => {
		const trimmed = parametersText.trim();
		const textToParse = trimmed || "{}";
		try {
			const parsed = JSON.parse(textToParse);
			applyParsedParameters(parsed);
		} catch (error) {
			setParametersError(
				error instanceof Error
					? error.message
					: "Parameters must be valid JSON",
			);
		}
	}, [applyParsedParameters, parametersText]);

	const handleProviderParamsBlur = useCallback(() => {
		const trimmed = providerParamsText.trim();
		const textToParse = trimmed || "{}";
		try {
			const parsed = JSON.parse(textToParse);
			if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
				throw new Error("Provider params must be a JSON object");
			}
			updateStructuredParameters((current) => ({
				...current,
				provider: {
					...current.provider,
					params: parsed as Record<string, unknown>,
				},
			}));
		} catch (error) {
			setProviderParamsError(
				error instanceof Error
					? error.message
					: "Provider params must be valid JSON",
			);
		}
	}, [providerParamsText, updateStructuredParameters]);

	const setModelHostingMode = useCallback((nextMode: "local" | "hosted") => {
		setDraft((current) => {
			if (!current) return current;
			const nextParameters = normalizeModelParameters(current.parameters);
			nextParameters.provider = {
				...nextParameters.provider,
				provider_name: nextMode === "hosted" ? "Hosted" : "Local",
				params:
					nextMode === "hosted"
						? getProviderParams(nextParameters.provider)
						: getProviderParams(nextParameters.provider),
			};
			setParametersText(JSON.stringify(nextParameters, null, 2));
			setProviderParamsText(
				JSON.stringify(getProviderParams(nextParameters.provider), null, 2),
			);
			return {
				...current,
				parameters: nextParameters,
				download_link: nextMode === "hosted" ? "" : current.download_link,
				file_name: nextMode === "hosted" ? "" : current.file_name,
				size: nextMode === "hosted" ? 0 : current.size,
			};
		});
		setParametersError(null);
		setProviderParamsError(null);
	}, []);

	const handleRefresh = useCallback(() => {
		queryClient.invalidateQueries({ queryKey: ["bit-search"] });
		if (selectedId) {
			queryClient.invalidateQueries({ queryKey: ["bit", selectedId] });
		}
	}, [queryClient, selectedId]);

	const handleSave = useCallback(async () => {
		if (!profile.data || !draft) {
			toast.error("Bit not ready to save");
			return;
		}

		let parsedParameters: unknown;
		try {
			parsedParameters = JSON.parse(parametersText);
		} catch {
			toast.error("Parameters must be valid JSON");
			return;
		}

		const nextDraft: IBit = {
			...draft,
			authors: parseDelimitedList(authorsText),
			dependencies: parseDelimitedList(dependenciesText),
			parameters: parsedParameters,
			meta: {
				...draft.meta,
				en: {
					...getEnglishMeta(draft),
					tags: parseDelimitedList(tagsText),
				},
			},
		};

		setIsSaving(true);
		try {
			let savedBit = nextDraft;
			let receivedSavedBit = false;
			await backend.apiState.stream(
				profile.data,
				`admin/bit/${nextDraft.id}`,
				{
					method: "PUT",
					body: JSON.stringify(nextDraft),
				},
				(data: Record<string, unknown>) => {
					if (typeof data?.id === "string") {
						savedBit = data as unknown as IBit;
						receivedSavedBit = true;
					}
				},
			);
			if (!receivedSavedBit) {
				throw new Error("Bit update did not complete");
			}
			await backend.apiState.put(
				profile.data,
				`admin/bit/${savedBit.id}/en`,
				nextDraft.meta.en,
			);
			toast.success("Bit updated");
			queryClient.invalidateQueries({ queryKey: ["bit-search"] });
			queryClient.invalidateQueries({ queryKey: ["bit", savedBit.id] });
		} catch (error) {
			const message = error instanceof Error ? error.message : "Unknown error";
			toast.error(`Failed to update bit: ${message}`);
		} finally {
			setIsSaving(false);
		}
	}, [
		authorsText,
		backend.apiState,
		dependenciesText,
		draft,
		parametersText,
		profile.data,
		queryClient,
		tagsText,
	]);

	const handleDelete = useCallback(async () => {
		if (!profile.data || !draft) {
			toast.error("Bit not ready to delete");
			return;
		}

		const confirmed = window.confirm(`Delete bit \"${draft.id}\"?`);
		if (!confirmed) return;

		setIsDeleting(true);
		try {
			await backend.apiState.del(profile.data, `admin/bit/${draft.id}`);
			toast.success("Bit deleted");
			setDraft(null);
			setSelectedId(null);
			queryClient.invalidateQueries({ queryKey: ["bit-search"] });
		} catch (error) {
			const message = error instanceof Error ? error.message : "Unknown error";
			toast.error(`Failed to delete bit: ${message}`);
		} finally {
			setIsDeleting(false);
		}
	}, [backend.apiState, draft, profile.data, queryClient]);

	const handleRepairTtsAssets = useCallback(async () => {
		if (!draft) {
			toast.error("Bit not ready to repair");
			return;
		}

		setIsRepairingTts(true);
		try {
			const pack = await backend.bitState.repairTtsBitAssets(draft);
			const replacementBit = pack.bits[0];
			toast.success(
				replacementBit?.id && replacementBit.id !== draft.id
					? `Created replacement TTS bit ${replacementBit.id}`
					: "TTS bit repair completed",
			);
			if (replacementBit?.id && replacementBit.id !== draft.id) {
				setSelectedId(replacementBit.id);
			}
			queryClient.invalidateQueries({ queryKey: ["bit-search"] });
			queryClient.invalidateQueries({ queryKey: ["bit", draft.id] });
			if (replacementBit?.id) {
				queryClient.invalidateQueries({ queryKey: ["bit", replacementBit.id] });
			}
		} catch (error) {
			const message = error instanceof Error ? error.message : "Unknown error";
			toast.error(`Failed to repair TTS assets: ${message}`);
		} finally {
			setIsRepairingTts(false);
		}
	}, [backend.bitState, draft, queryClient]);

	const modelParameters = useMemo(() => {
		if (
			!draft ||
			!MODEL_BIT_TYPES.includes(draft.type as (typeof MODEL_BIT_TYPES)[number])
		) {
			return null;
		}
		return normalizeModelParameters(draft.parameters);
	}, [draft]);

	const draftIsHosted = isHostedBit(draft);
	const canRepairTtsAssets = draft?.type === IBitTypes.Tts;

	return (
		<main className="flex h-full min-h-0 w-full grow flex-col overflow-hidden bg-background">
			<div className="flex-1 overflow-y-auto p-6">
				<div className="space-y-6">
					<div className="flex items-center justify-between gap-4">
						<div>
							<h1 className="text-3xl font-bold">Bit Management</h1>
							<p className="text-muted-foreground">
								Search, inspect, edit, and remove published bits from one
								screen.
							</p>
						</div>
						<div className="flex items-center gap-2">
							<Button variant="outline" size="sm" onClick={handleRefresh}>
								<RefreshCw className="mr-2 h-4 w-4" />
								Refresh
							</Button>
							<Button size="sm" onClick={() => router.push("/admin/bits/add")}>
								<Plus className="mr-2 h-4 w-4" />
								Add Bit
							</Button>
						</div>
					</div>

					<Card>
						<CardHeader>
							<CardTitle>Search</CardTitle>
							<CardDescription>
								Filter by name, description, type, or hosted-model category.
							</CardDescription>
						</CardHeader>
						<CardContent className="grid gap-4 md:grid-cols-[1fr,240px,160px]">
							<div className="relative">
								<Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
								<Input
									className="pl-10"
									placeholder="Search bits..."
									value={searchTerm}
									onChange={(event) => setSearchTerm(event.target.value)}
								/>
							</div>
							<Select
								value={selectedType}
								onValueChange={(v) => setSelectedType(v as BitFilterValue)}
							>
								<SelectTrigger>
									<SelectValue placeholder="All categories" />
								</SelectTrigger>
								<SelectContent>
									<SelectItem value="all">All categories</SelectItem>
									<SelectItem value={HOSTED_FILTER}>Hosted models</SelectItem>
									{ALL_BIT_TYPES.map((bitType) => (
										<SelectItem key={bitType} value={bitType}>
											{bitType}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
							<Select
								value={itemsPerPage.toString()}
								onValueChange={(value) => setItemsPerPage(Number(value))}
							>
								<SelectTrigger>
									<SelectValue placeholder="Page size" />
								</SelectTrigger>
								<SelectContent>
									{ITEMS_PER_PAGE_OPTIONS.map((option) => (
										<SelectItem key={option} value={option.toString()}>
											{option} per page
										</SelectItem>
									))}
								</SelectContent>
							</Select>
						</CardContent>
					</Card>

					<div className="grid gap-6 lg:grid-cols-[360px,minmax(0,1fr)]">
						<Card className="min-h-180">
							<CardHeader>
								<CardTitle>Matching Bits</CardTitle>
								<CardDescription>
									{bits.isLoading
										? "Loading bits..."
										: `${visibleBits.length} visible on this page`}
								</CardDescription>
							</CardHeader>
							<CardContent className="space-y-3">
								{bits.isLoading ? (
									<div className="space-y-2">
										{Array.from({ length: 8 }).map((_, index) => (
											<Skeleton
												key={`bit-skeleton-${index}`}
												className="h-20 w-full"
											/>
										))}
									</div>
								) : visibleBits.length === 0 ? (
									<div className="rounded-lg border border-dashed p-6 text-sm text-muted-foreground">
										No bits matched the current filters.
									</div>
								) : (
									visibleBits.map((bit) => {
										const meta =
											bit.meta?.en ?? Object.values(bit.meta ?? {})[0];
										const isSelected = bit.id === selectedId;
										return (
											<button
												key={bit.id}
												type="button"
												onClick={() => setSelectedId(bit.id)}
												className={`w-full rounded-lg border p-4 text-left transition-colors ${
													isSelected
														? "border-primary bg-primary/5"
														: "hover:border-primary/40"
												}`}
											>
												<div className="flex items-center justify-between gap-3">
													<div>
														<p className="font-medium">
															{meta?.name || bit.id}
														</p>
														<p className="text-xs text-muted-foreground">
															{bit.id}
														</p>
													</div>
													<div className="flex items-center gap-2">
														{isHostedBit(bit) ? (
															<Badge variant="outline">Hosted</Badge>
														) : null}
														<Badge variant="secondary">{bit.type}</Badge>
													</div>
												</div>
												{meta?.description && (
													<p className="mt-2 line-clamp-2 text-sm text-muted-foreground">
														{meta.description}
													</p>
												)}
												<div className="mt-3 flex flex-wrap gap-2 text-xs text-muted-foreground">
													<span>Version {bit.version ?? "-"}</span>
													<span>{bit.repository || "No repository"}</span>
												</div>
											</button>
										);
									})
								)}

								<div className="flex items-center justify-between pt-2">
									<Button
										variant="outline"
										size="sm"
										disabled={currentPage === 1}
										onClick={() =>
											setCurrentPage((page) => Math.max(1, page - 1))
										}
									>
										Previous
									</Button>
									<span className="text-xs text-muted-foreground">
										Page {currentPage}
									</span>
									<Button
										variant="outline"
										size="sm"
										disabled={!hasMorePages}
										onClick={() => setCurrentPage((page) => page + 1)}
									>
										Next
									</Button>
								</div>
							</CardContent>
						</Card>

						<Card className="min-h-180">
							<CardHeader>
								<CardTitle>Editor</CardTitle>
								<CardDescription>
									Update core fields, metadata, and model runtime settings from
									one screen.
								</CardDescription>
							</CardHeader>
							<CardContent className="space-y-6">
								{selectedBit.isLoading || (selectedId && !draft) ? (
									<div className="space-y-3">
										{Array.from({ length: 10 }).map((_, index) => (
											<Skeleton
												key={`editor-skeleton-${index}`}
												className="h-12 w-full"
											/>
										))}
									</div>
								) : !draft ? (
									<div className="rounded-lg border border-dashed p-6 text-sm text-muted-foreground">
										Select a bit from the left to inspect and edit it.
									</div>
								) : (
									<>
										<div className="grid gap-4 md:grid-cols-2">
											<div className="space-y-2">
												<Label htmlFor="bit-id">Bit ID</Label>
												<Input id="bit-id" value={draft.id} disabled />
											</div>
											<div className="space-y-2">
												<Label htmlFor="bit-slug">Model Slug</Label>
												<Input
													id="bit-slug"
													value={draft.name ?? ""}
													onChange={(event) =>
														updateDraft("name", event.target.value)
													}
													placeholder="e.g. step-3-5-flash"
												/>
												<p className="text-xs text-muted-foreground">
													Used to auto-compute capability scores for hosted
													models.
												</p>
											</div>
											<div className="space-y-2">
												<Label htmlFor="bit-type">Type</Label>
												<Select
													value={draft.type}
													onValueChange={(value) =>
														updateDraft("type", value as IBitTypes)
													}
												>
													<SelectTrigger id="bit-type">
														<SelectValue />
													</SelectTrigger>
													<SelectContent>
														{ALL_BIT_TYPES.map((bitType) => (
															<SelectItem key={bitType} value={bitType}>
																{bitType}
															</SelectItem>
														))}
													</SelectContent>
												</Select>
											</div>
											{modelParameters ? (
												<div className="space-y-2">
													<Label htmlFor="bit-model-category">
														Model Category
													</Label>
													<Select
														value={draftIsHosted ? HOSTED_FILTER : "local"}
														onValueChange={(value) =>
															setModelHostingMode(
																value === HOSTED_FILTER ? "hosted" : "local",
															)
														}
													>
														<SelectTrigger id="bit-model-category">
															<SelectValue />
														</SelectTrigger>
														<SelectContent>
															<SelectItem value="local">
																Local weights
															</SelectItem>
															<SelectItem value={HOSTED_FILTER}>
																Hosted model
															</SelectItem>
														</SelectContent>
													</Select>
												</div>
											) : null}
											<div className="space-y-2 md:col-span-2">
												<Label htmlFor="bit-name">Display Name</Label>
												<Input
													id="bit-name"
													value={draft.meta.en?.name ?? ""}
													onChange={(event) =>
														updateMeta("name", event.target.value)
													}
												/>
											</div>
											<div className="space-y-2 md:col-span-2">
												<Label htmlFor="bit-description">Description</Label>
												<Textarea
													id="bit-description"
													rows={3}
													value={draft.meta.en?.description ?? ""}
													onChange={(event) =>
														updateMeta("description", event.target.value)
													}
												/>
											</div>
											<div className="space-y-2 md:col-span-2">
												<Label htmlFor="bit-long-description">
													Long Description
												</Label>
												<Textarea
													id="bit-long-description"
													rows={6}
													value={draft.meta.en?.long_description ?? ""}
													onChange={(event) =>
														updateMeta("long_description", event.target.value)
													}
												/>
											</div>
											<div className="space-y-2">
												<Label htmlFor="bit-version">Version</Label>
												<Input
													id="bit-version"
													value={draft.version ?? ""}
													onChange={(event) =>
														updateDraft("version", event.target.value)
													}
												/>
											</div>
											<div className="space-y-2">
												<Label htmlFor="bit-license">License</Label>
												<Input
													id="bit-license"
													value={draft.license ?? ""}
													onChange={(event) =>
														updateDraft("license", event.target.value)
													}
												/>
											</div>
											<div className="space-y-2 md:col-span-2">
												<Label htmlFor="bit-repository">Repository</Label>
												<Input
													id="bit-repository"
													value={draft.repository ?? ""}
													onChange={(event) =>
														updateDraft("repository", event.target.value)
													}
												/>
											</div>
											{modelParameters ? (
												<>
													<div className="space-y-2">
														<Label htmlFor="bit-context-length">
															Context Length
														</Label>
														<Input
															id="bit-context-length"
															type="number"
															value={modelParameters.context_length}
															onChange={(event) =>
																updateStructuredParameters((current) => ({
																	...current,
																	context_length:
																		Number(event.target.value) || 2048,
																}))
															}
														/>
													</div>
													<div className="space-y-2">
														<Label htmlFor="bit-provider-name">Provider</Label>
														<Select
															value={modelParameters.provider.provider_name}
															onValueChange={(value) =>
																updateStructuredParameters((current) => ({
																	...current,
																	provider: {
																		...current.provider,
																		provider_name: value,
																	},
																}))
															}
														>
															<SelectTrigger id="bit-provider-name">
																<SelectValue />
															</SelectTrigger>
															<SelectContent>
																<SelectItem value="Local">Local</SelectItem>
																<SelectItem value="Premium">Premium</SelectItem>
																{HOSTED_PROVIDER_OPTIONS.map((providerName) => (
																	<SelectItem
																		key={providerName}
																		value={providerName}
																	>
																		{providerName}
																	</SelectItem>
																))}
															</SelectContent>
														</Select>
													</div>
													<div className="space-y-2">
														<Label htmlFor="bit-model-id">Model ID</Label>
														<Input
															id="bit-model-id"
															value={modelParameters.provider.model_id ?? ""}
															onChange={(event) =>
																updateStructuredParameters((current) => ({
																	...current,
																	provider: {
																		...current.provider,
																		model_id: event.target.value || null,
																	},
																}))
															}
														/>
													</div>
													<div className="space-y-2">
														<Label htmlFor="bit-provider-version">
															Provider Version
														</Label>
														<Input
															id="bit-provider-version"
															value={modelParameters.provider.version ?? ""}
															onChange={(event) =>
																updateStructuredParameters((current) => ({
																	...current,
																	provider: {
																		...current.provider,
																		version: event.target.value || null,
																	},
																}))
															}
														/>
													</div>
													{draftIsHosted ? (
														<>
															<div className="space-y-2 md:col-span-2">
																<Label htmlFor="bit-provider-endpoint">
																	Endpoint
																</Label>
																<Input
																	id="bit-provider-endpoint"
																	value={
																		typeof getProviderParams(
																			modelParameters.provider,
																		).endpoint === "string"
																			? (getProviderParams(
																					modelParameters.provider,
																				).endpoint as string)
																			: ""
																	}
																	onChange={(event) =>
																		updateStructuredParameters((current) => ({
																			...current,
																			provider: {
																				...current.provider,
																				params: {
																					...getProviderParams(
																						current.provider,
																					),
																					endpoint: event.target.value,
																				},
																			},
																		}))
																	}
																/>
															</div>
															<div className="space-y-2 md:col-span-2">
																<Label htmlFor="bit-provider-params">
																	Provider Params JSON
																</Label>
																<Textarea
																	id="bit-provider-params"
																	rows={8}
																	value={providerParamsText}
																	onChange={(event) => {
																		setProviderParamsText(event.target.value);
																		setProviderParamsError(null);
																	}}
																	onBlur={handleProviderParamsBlur}
																/>
																{providerParamsError ? (
																	<p className="text-xs text-destructive">
																		{providerParamsError}
																	</p>
																) : (
																	<p className="text-xs text-muted-foreground">
																		Hosted models use provider params for
																		endpoint overrides and provider-specific
																		metadata.
																	</p>
																)}
															</div>
														</>
													) : null}
												</>
											) : null}
											{!draftIsHosted ? (
												<>
													<div className="space-y-2 md:col-span-2">
														<Label htmlFor="bit-download-link">
															Download Link
														</Label>
														<Input
															id="bit-download-link"
															value={draft.download_link ?? ""}
															onChange={(event) =>
																updateDraft("download_link", event.target.value)
															}
														/>
													</div>
													<div className="space-y-2">
														<Label htmlFor="bit-file-name">File Name</Label>
														<Input
															id="bit-file-name"
															value={draft.file_name ?? ""}
															onChange={(event) =>
																updateDraft("file_name", event.target.value)
															}
														/>
													</div>
													<div className="space-y-2">
														<Label htmlFor="bit-size">Size (bytes)</Label>
														<Input
															id="bit-size"
															type="number"
															value={draft.size ?? 0}
															onChange={(event) =>
																updateDraft("size", Number(event.target.value))
															}
														/>
													</div>
												</>
											) : (
												<div className="rounded-lg border border-dashed p-4 text-sm text-muted-foreground md:col-span-2">
													Hosted models are routed through provider metadata and
													do not need a downloadable artifact, stored file name,
													or local size.
												</div>
											)}
											<div className="space-y-2 md:col-span-2">
												<Label htmlFor="bit-authors">Authors</Label>
												<Input
													id="bit-authors"
													placeholder="Comma or newline separated"
													value={authorsText}
													onChange={(event) =>
														setAuthorsText(event.target.value)
													}
												/>
											</div>
											<div className="space-y-2 md:col-span-2">
												<Label htmlFor="bit-dependencies">Dependencies</Label>
												<Textarea
													id="bit-dependencies"
													rows={4}
													placeholder="One dependency per line or comma separated"
													value={dependenciesText}
													onChange={(event) =>
														setDependenciesText(event.target.value)
													}
												/>
											</div>
											<div className="space-y-2 md:col-span-2">
												<Label htmlFor="bit-tags">Tags</Label>
												<Input
													id="bit-tags"
													placeholder="Comma or newline separated"
													value={tagsText}
													onChange={(event) => setTagsText(event.target.value)}
												/>
											</div>
											<div className="space-y-2 md:col-span-2">
												<Label htmlFor="bit-parameters">Parameters JSON</Label>
												<Textarea
													id="bit-parameters"
													rows={16}
													value={parametersText}
													onChange={(event) => {
														setParametersText(event.target.value);
														setParametersError(null);
													}}
													onBlur={handleParametersBlur}
												/>
												{parametersError ? (
													<p className="text-xs text-destructive">
														{parametersError}
													</p>
												) : (
													<p className="text-xs text-muted-foreground">
														Advanced overrides remain available here. Structured
														model fields above keep this JSON in sync.
													</p>
												)}
											</div>
										</div>

										<div className="flex items-center justify-between rounded-lg border p-4">
											<div>
												<p className="font-medium">Danger Zone</p>
												<p className="text-sm text-muted-foreground">
													Deleting a bit removes it from the registry and
													deletes the stored artifact.
												</p>
											</div>
											<Button
												variant="destructive"
												onClick={handleDelete}
												disabled={isDeleting}
											>
												<Trash2 className="mr-2 h-4 w-4" />
												Delete Bit
											</Button>
										</div>

										<div className="flex justify-end gap-2">
											<Button variant="outline" onClick={handleRefresh}>
												Refresh
											</Button>
											{canRepairTtsAssets ? (
												<Button
													variant="outline"
													onClick={handleRepairTtsAssets}
													disabled={isRepairingTts}
												>
													<Wrench className="mr-2 h-4 w-4" />
													Repair TTS Assets
												</Button>
											) : null}
											<Button onClick={handleSave} disabled={isSaving}>
												<Save className="mr-2 h-4 w-4" />
												Save Changes
											</Button>
										</div>
									</>
								)}
							</CardContent>
						</Card>
					</div>
				</div>
			</div>
		</main>
	);
}
