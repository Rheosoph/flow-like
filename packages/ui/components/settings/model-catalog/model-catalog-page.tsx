"use client";

import {
	Brain,
	Code2,
	Cpu,
	DollarSign,
	FileSearchIcon,
	Filter,
	Globe,
	Grid3X3,
	ImageIcon,
	LayoutList,
	Lightbulb,
	type LucideIcon,
	MessageSquare,
	PackageCheck,
	Search,
	Shield,
	Sparkles,
	Type,
	Wand2,
	X,
	Zap,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useMiniSearch } from "react-minisearch";
import { useInvoke } from "../../../hooks/index";
import { useIsMobile } from "../../../hooks/use-mobile";
import { Bit } from "../../../lib/bit/bit";
import type { IBit } from "../../../lib/schema/bit/bit";
import { IBitTypes } from "../../../lib/schema/bit/bit";
import type { ILlmParameters } from "../../../lib/schema/bit/bit/llm-parameters";
import { useBackend } from "../../../state/backend-state";
import {
	Button,
	Input,
	ModelCard,
	ModelDetailSheet,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Slider,
	formatContextLength,
} from "../../ui";
import { Checkbox } from "../../ui/checkbox";
import {
	Sheet,
	SheetContent,
	SheetDescription,
	SheetHeader,
	SheetTitle,
} from "../../ui/sheet";
import { Skeleton } from "../../ui/skeleton";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../ui/tooltip";

type SortOption =
	| "name"
	| "updated"
	| "context"
	| "speed"
	| "cost"
	| "reasoning"
	| "coding";
type ViewMode = "grid" | "list";
type InputModality = "text" | "image";
type OutputModality = "text" | "embedding";

function getBitModality(type: IBitTypes): {
	input: InputModality;
	output: OutputModality;
} {
	switch (type) {
		case IBitTypes.Llm:
			return { input: "text", output: "text" };
		case IBitTypes.Vlm:
			return { input: "image", output: "text" };
		case IBitTypes.Embedding:
			return { input: "text", output: "embedding" };
		case IBitTypes.ImageEmbedding:
			return { input: "image", output: "embedding" };
		default:
			return { input: "text", output: "text" };
	}
}

interface CapabilityInfo {
	icon: LucideIcon;
	label: string;
	color: string;
}

const capabilityIcons: Record<string, CapabilityInfo> = {
	coding: { icon: Code2, label: "Coding", color: "text-blue-500" },
	cost: { icon: DollarSign, label: "Cost Efficiency", color: "text-green-500" },
	creativity: {
		icon: Lightbulb,
		label: "Creativity",
		color: "text-yellow-500",
	},
	factuality: { icon: Shield, label: "Factuality", color: "text-emerald-500" },
	function_calling: {
		icon: Wand2,
		label: "Function Calling",
		color: "text-purple-500",
	},
	multilinguality: {
		icon: Globe,
		label: "Multilingual",
		color: "text-cyan-500",
	},
	reasoning: { icon: Brain, label: "Reasoning", color: "text-orange-500" },
	speed: { icon: Zap, label: "Speed", color: "text-amber-500" },
};

const LLM_LIKE_TYPES = new Set([IBitTypes.Llm, IBitTypes.Vlm]);

function isHostedModel(bit: IBit): boolean {
	return (bit.size ?? 0) === 0 || !bit.download_link;
}

interface AIModelPageProps {
	webMode?: boolean;
}

export function AIModelPage({ webMode = false }: AIModelPageProps) {
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);
	const isMobile = useIsMobile();
	const [searchTerm, setSearchTerm] = useState("");
	const [blacklist, setBlacklist] = useState(new Set<string>());
	const [viewMode, setViewMode] = useState<ViewMode>("grid");
	const [sortBy, setSortBy] = useState<SortOption>("updated");
	const [filtersExpanded, setFiltersExpanded] = useState(false);
	const [mobileFiltersOpen, setMobileFiltersOpen] = useState(false);
	const [providerFilter, setProviderFilter] = useState("all");
	const [contextLengthFilter, setContextLengthFilter] = useState<
		[number, number]
	>([0, 2000000]);
	const [showInProfileOnly, setShowInProfileOnly] = useState(false);
	const [showDownloadedOnly, setShowDownloadedOnly] = useState(false);
	const [selectedModel, setSelectedModel] = useState<IBit | null>(null);
	const [inputModalities, setInputModalities] = useState<Set<InputModality>>(
		new Set(["text", "image"]),
	);
	const [outputModalities, setOutputModalities] = useState<Set<OutputModality>>(
		new Set(["text", "embedding"]),
	);
	const [capabilityFilters, setCapabilityFilters] = useState<
		Record<string, number>
	>({
		reasoning: 0,
		coding: 0,
		speed: 0,
		cost: 0,
		creativity: 0,
		factuality: 0,
	});

	const checkInstalled = useCallback(
		async (bit: IBit) => {
			try {
				const result = await backend.bitState.isBitInstalled(bit);
				return result;
			} catch {
				return false;
			}
		},
		[backend.bitState],
	);

	const foundBits = useInvoke(
		backend.bitState.searchBits,
		backend.bitState,
		[
			{
				bit_types: [
					IBitTypes.Llm,
					IBitTypes.Vlm,
					IBitTypes.Embedding,
					IBitTypes.ImageEmbedding,
				],
			},
		],
		typeof profile.data !== "undefined",
		[profile.data?.id ?? ""],
	);

	const imageBlacklist = useCallback(async () => {
		if (!foundBits.data) return;
		const dependencies = await Promise.all(
			foundBits.data
				.filter((bit) => bit.type === IBitTypes.ImageEmbedding)
				.map((bit) =>
					Bit.fromObject(bit).setBackend(backend).fetchDependencies(),
				),
		);
		const bl = new Set<string>(
			dependencies.flatMap((dep) =>
				dep.bits
					.filter((bit) => bit.type !== IBitTypes.ImageEmbedding)
					.map((bit) => bit.id),
			),
		);
		setBlacklist(bl);
	}, [backend, foundBits.data]);

	const [installedBits, setInstalledBits] = useState<Set<string>>(new Set());

	const { search, searchResults, addAllAsync, removeAll } = useMiniSearch<IBit>(
		[],
		{
			fields: [
				"authors",
				"file_name",
				"hub",
				"id",
				"name",
				"long_description",
				"description",
				"type",
			],
			storeFields: ["id"],
			searchOptions: {
				fuzzy: true,
				boost: { name: 2, type: 1.5, description: 1, long_description: 0.5 },
			},
		},
	);

	useEffect(() => {
		if (!foundBits.data) return;
		imageBlacklist();
	}, [foundBits.data, imageBlacklist]);

	useEffect(() => {
		if (!foundBits.data || !profile.data || !checkInstalled) return;
		const checkInstalledAll = async () => {
			const installedSet = new Set<string>();
			for (const bit of foundBits.data) {
				const isInstalled = await checkInstalled(bit);
				if (isInstalled) installedSet.add(bit.id);
			}
			setInstalledBits(installedSet);
		};
		checkInstalledAll();
	}, [foundBits.data, profile.data, checkInstalled]);

	useEffect(() => {
		if (!foundBits.data) return;
		removeAll();
		addAllAsync(
			foundBits.data.map((item) => ({
				...item,
				name: item.meta?.en?.name,
				long_description: item.meta?.en?.long_description,
				description: item.meta?.en?.description,
			})),
		);
	}, [foundBits.data, addAllAsync, removeAll]);

	const providers = useMemo(() => {
		if (!foundBits.data) return [];
		const providerSet = new Set<string>();
		for (const model of foundBits.data) {
			const params = model.parameters as ILlmParameters | undefined;
			if (params?.provider?.provider_name) {
				providerSet.add(params.provider.provider_name);
			}
		}
		return Array.from(providerSet).sort();
	}, [foundBits.data]);

	const maxContextLength = useMemo(() => {
		if (!foundBits.data) return 2000000;
		return Math.max(
			...foundBits.data.map(
				(m) =>
					(m.parameters as ILlmParameters | undefined)?.context_length ?? 0,
			),
			128000,
		);
	}, [foundBits.data]);

	const profileBitIds = useMemo(() => {
		return new Set(profile.data?.bits?.map((id) => id.split(":").pop()) ?? []);
	}, [profile.data]);

	const filteredModels = useMemo(() => {
		let models = searchTerm.trim()
			? ((searchResults as IBit[]) ?? [])
			: (foundBits.data ?? []);
		models = models.filter((bit) => !blacklist.has(bit.id));
		models = models.filter((bit) => bit.meta?.en !== undefined);

		// In web mode, filter LLM/VLM to only show hosted models
		if (webMode) {
			models = models.filter((bit) => {
				if (LLM_LIKE_TYPES.has(bit.type)) {
					return isHostedModel(bit);
				}
				return true;
			});
		}

		if (inputModalities.size < 2 || outputModalities.size < 2) {
			models = models.filter((m) => {
				const bitModality = getBitModality(m.type);
				const inputMatch =
					inputModalities.size === 0 || inputModalities.has(bitModality.input);
				const outputMatch =
					outputModalities.size === 0 ||
					outputModalities.has(bitModality.output);
				return inputMatch && outputMatch;
			});
		}

		if (showInProfileOnly)
			models = models.filter((m) => profileBitIds.has(m.id));
		if (showDownloadedOnly)
			models = models.filter((m) => installedBits.has(m.id));

		if (providerFilter !== "all") {
			models = models.filter((m) => {
				const params = m.parameters as ILlmParameters | undefined;
				return params?.provider?.provider_name === providerFilter;
			});
		}

		models = models.filter((m) => {
			const params = m.parameters as ILlmParameters | undefined;
			const contextLength = params?.context_length ?? 0;
			return (
				contextLength >= contextLengthFilter[0] &&
				contextLength <= contextLengthFilter[1]
			);
		});

		models = models.filter((m) => {
			const params = m.parameters as ILlmParameters | undefined;
			const classification = params?.model_classification;
			if (!classification) return true;
			for (const [key, minValue] of Object.entries(capabilityFilters)) {
				if (minValue > 0) {
					const modelValue =
						classification[key as keyof typeof classification] ?? 0;
					if (modelValue < minValue) return false;
				}
			}
			return true;
		});

		models.sort((a, b) => {
			const aParams = a.parameters as ILlmParameters | undefined;
			const bParams = b.parameters as ILlmParameters | undefined;
			switch (sortBy) {
				case "name":
					return (a.meta?.en?.name || a.id).localeCompare(
						b.meta?.en?.name || b.id,
					);
				case "updated":
					return Date.parse(b.updated) - Date.parse(a.updated);
				case "context":
					return (
						(bParams?.context_length ?? 0) - (aParams?.context_length ?? 0)
					);
				case "speed":
					return (
						(bParams?.model_classification?.speed ?? 0) -
						(aParams?.model_classification?.speed ?? 0)
					);
				case "cost":
					return (
						(bParams?.model_classification?.cost ?? 0) -
						(aParams?.model_classification?.cost ?? 0)
					);
				case "reasoning":
					return (
						(bParams?.model_classification?.reasoning ?? 0) -
						(aParams?.model_classification?.reasoning ?? 0)
					);
				case "coding":
					return (
						(bParams?.model_classification?.coding ?? 0) -
						(aParams?.model_classification?.coding ?? 0)
					);
				default:
					return 0;
			}
		});

		return models;
	}, [
		foundBits.data,
		searchResults,
		searchTerm,
		inputModalities,
		outputModalities,
		providerFilter,
		contextLengthFilter,
		showInProfileOnly,
		showDownloadedOnly,
		profileBitIds,
		installedBits,
		blacklist,
		sortBy,
		capabilityFilters,
		webMode,
	]);

	const modalityCounts = useMemo(() => {
		const counts = { text: 0, image: 0, embedding: 0, total: 0 };
		if (!foundBits.data) return counts;
		const validBits = foundBits.data.filter((bit) => !blacklist.has(bit.id));
		counts.total = validBits.length;
		for (const bit of validBits) {
			const modality = getBitModality(bit.type);
			if (modality.input === "text") counts.text++;
			if (modality.input === "image") counts.image++;
			if (modality.output === "embedding") counts.embedding++;
		}
		return counts;
	}, [foundBits.data, blacklist]);

	const activeFilterCount = useMemo(() => {
		let count = 0;
		if (providerFilter !== "all") count++;
		if (showInProfileOnly) count++;
		if (showDownloadedOnly) count++;
		if (contextLengthFilter[0] > 0 || contextLengthFilter[1] < maxContextLength)
			count++;
		if (inputModalities.size < 2) count++;
		if (outputModalities.size < 2) count++;
		if (Object.values(capabilityFilters).some((v) => v > 0)) count++;
		return count;
	}, [
		providerFilter,
		showInProfileOnly,
		showDownloadedOnly,
		contextLengthFilter,
		maxContextLength,
		inputModalities,
		outputModalities,
		capabilityFilters,
	]);

	const toggleInputModality = useCallback((modality: InputModality) => {
		setInputModalities((prev) => {
			const next = new Set(prev);
			if (next.has(modality)) next.delete(modality);
			else next.add(modality);
			return next;
		});
	}, []);

	const toggleOutputModality = useCallback((modality: OutputModality) => {
		setOutputModalities((prev) => {
			const next = new Set(prev);
			if (next.has(modality)) next.delete(modality);
			else next.add(modality);
			return next;
		});
	}, []);

	const resetFilters = useCallback(() => {
		setProviderFilter("all");
		setShowInProfileOnly(false);
		setShowDownloadedOnly(false);
		setContextLengthFilter([0, maxContextLength]);
		setInputModalities(new Set(["text", "image"]));
		setOutputModalities(new Set(["text", "embedding"]));
		setCapabilityFilters({
			reasoning: 0,
			coding: 0,
			speed: 0,
			cost: 0,
			creativity: 0,
			factuality: 0,
		});
	}, [maxContextLength]);

	const filterContent = (
		<div className="space-y-5">
			<div className="space-y-2">
				<p className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
					Status
				</p>
				<div className="space-y-1.5">
					<FilterCheckbox
						checked={showInProfileOnly}
						onCheckedChange={(c) => setShowInProfileOnly(!!c)}
						icon={Sparkles}
						iconColor="text-primary"
						label="In Profile"
					/>
					{!webMode && (
						<FilterCheckbox
							checked={showDownloadedOnly}
							onCheckedChange={(c) => setShowDownloadedOnly(!!c)}
							icon={PackageCheck}
							iconColor="text-emerald-500"
							label="Downloaded"
						/>
					)}
				</div>
			</div>

			{providers.length > 0 && (
				<div className="space-y-2">
					<p className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
						Provider
					</p>
					<Select value={providerFilter} onValueChange={setProviderFilter}>
						<SelectTrigger className="h-8 text-xs">
							<SelectValue placeholder="All providers" />
						</SelectTrigger>
						<SelectContent>
							<SelectItem value="all">All providers</SelectItem>
							{providers.map((provider) => (
								<SelectItem key={provider} value={provider}>
									{provider}
								</SelectItem>
							))}
						</SelectContent>
					</Select>
				</div>
			)}

			<div className="space-y-2">
				<div className="flex items-center justify-between">
					<p className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60 flex items-center gap-2">
						<Cpu className="h-3 w-3" />
						Context
					</p>
					<span className="text-[10px] text-muted-foreground/40">
						{formatContextLength(contextLengthFilter[0])} –{" "}
						{formatContextLength(contextLengthFilter[1])}
					</span>
				</div>
				<Slider
					value={contextLengthFilter}
					onValueChange={(v) => setContextLengthFilter(v as [number, number])}
					min={0}
					max={maxContextLength}
					step={1000}
				/>
			</div>

			<div className="space-y-3">
				<p className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60 flex items-center gap-2">
					<Brain className="h-3 w-3" />
					Capabilities
				</p>
				<div className="space-y-4">
					{Object.entries(capabilityIcons)
						.slice(0, 6)
						.map(([key, info]) => {
							const Icon = info.icon;
							const value = capabilityFilters[key] ?? 0;
							return (
								<div key={key} className="space-y-1.5">
									<div className="flex items-center justify-between text-xs">
										<div className={`flex items-center gap-1.5 ${info.color}`}>
											<Icon className="h-3 w-3" />
											<span>{info.label}</span>
										</div>
										<span className="text-muted-foreground/40">
											{value > 0 ? `\u2265${Math.round(value * 100)}%` : "Any"}
										</span>
									</div>
									<Slider
										value={[value]}
										onValueChange={([v]) =>
											setCapabilityFilters((prev) => ({
												...prev,
												[key]: v,
											}))
										}
										min={0}
										max={1}
										step={0.1}
										className="h-1"
									/>
								</div>
							);
						})}
				</div>
			</div>

			{activeFilterCount > 0 && (
				<button
					type="button"
					onClick={resetFilters}
					className="text-xs text-muted-foreground/40 hover:text-foreground transition-colors"
				>
					Clear {activeFilterCount} filter{activeFilterCount !== 1 ? "s" : ""}
				</button>
			)}
		</div>
	);

	return (
		<main className="flex flex-col w-full flex-1 min-h-0">
			<div
				className={`pt-5 pb-3 space-y-3 ${isMobile ? "px-4" : "px-4 sm:px-8"}`}
			>
				<div className="flex items-center gap-2">
					<div className="relative flex-1 max-w-lg">
						<Search className="absolute left-4 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground/40 pointer-events-none" />
						<Input
							placeholder="Search…"
							value={searchTerm}
							onChange={(e) => {
								setSearchTerm(e.target.value);
								search(e.target.value);
							}}
							className="pl-11 h-10 rounded-full bg-muted/30 border-transparent focus:border-border/40 focus:bg-muted/50 transition-all text-sm"
						/>
						{searchTerm && (
							<button
								type="button"
								onClick={() => {
									setSearchTerm("");
									search("");
								}}
								className="absolute right-4 top-1/2 -translate-y-1/2 text-muted-foreground/40 hover:text-foreground transition-colors"
							>
								<X className="h-4 w-4" />
							</button>
						)}
					</div>

					<div className="flex items-center gap-1">
						<Select
							value={sortBy}
							onValueChange={(v) => setSortBy(v as SortOption)}
						>
							<SelectTrigger className="h-8 w-auto gap-1.5 rounded-full border-transparent bg-transparent text-xs text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30 px-3 focus:ring-0">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="updated">Recent</SelectItem>
								<SelectItem value="name">Name</SelectItem>
								<SelectItem value="context">Context</SelectItem>
								<SelectItem value="speed">Speed</SelectItem>
								<SelectItem value="cost">Cost</SelectItem>
								<SelectItem value="reasoning">Reasoning</SelectItem>
								<SelectItem value="coding">Coding</SelectItem>
							</SelectContent>
						</Select>

						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant="ghost"
									size="icon"
									className={`h-8 w-8 rounded-full ${
										viewMode === "list"
											? "text-foreground/80 bg-muted/40"
											: "text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30"
									}`}
									onClick={() =>
										setViewMode((v) => (v === "grid" ? "list" : "grid"))
									}
								>
									{viewMode === "grid" ? (
										<Grid3X3 className="h-4 w-4" />
									) : (
										<LayoutList className="h-4 w-4" />
									)}
								</Button>
							</TooltipTrigger>
							<TooltipContent>
								{viewMode === "grid" ? "Switch to list" : "Switch to grid"}
							</TooltipContent>
						</Tooltip>

						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant={filtersExpanded ? "secondary" : "ghost"}
									size="icon"
									className={`h-8 w-8 rounded-full relative ${
										filtersExpanded
											? "text-primary bg-primary/10"
											: "text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30"
									}`}
									onClick={() => {
										if (isMobile) {
											setMobileFiltersOpen(true);
										} else {
											setFiltersExpanded((v) => !v);
										}
									}}
								>
									<Filter className="h-4 w-4" />
									{activeFilterCount > 0 && !filtersExpanded && (
										<span className="absolute -top-0.5 -right-0.5 h-3.5 w-3.5 rounded-full bg-primary text-[9px] text-primary-foreground flex items-center justify-center">
											{activeFilterCount}
										</span>
									)}
								</Button>
							</TooltipTrigger>
							<TooltipContent>
								{filtersExpanded ? "Hide filters" : "Show filters"}
							</TooltipContent>
						</Tooltip>
					</div>
				</div>

				{/* Quick modality chips */}
				<div className="flex items-center gap-1.5 flex-wrap">
					<ModalityChip
						active={inputModalities.has("text")}
						onClick={() => toggleInputModality("text")}
						icon={Type}
						label="Text"
					/>
					<ModalityChip
						active={inputModalities.has("image")}
						onClick={() => toggleInputModality("image")}
						icon={ImageIcon}
						label="Image"
					/>
					<span className="w-px h-4 bg-border/20 mx-0.5" />
					<ModalityChip
						active={outputModalities.has("text")}
						onClick={() => toggleOutputModality("text")}
						icon={MessageSquare}
						label="Chat"
					/>
					<ModalityChip
						active={outputModalities.has("embedding")}
						onClick={() => toggleOutputModality("embedding")}
						icon={FileSearchIcon}
						label="Embedding"
					/>

					<span className="text-xs text-muted-foreground/30 ml-auto">
						{filteredModels.length} model
						{filteredModels.length !== 1 ? "s" : ""}
					</span>
				</div>

				{/* Expanded filter panel (desktop) */}
				{filtersExpanded && (
					<div className="pt-3 border-t border-border/10">{filterContent}</div>
				)}
			</div>

			{/* Mobile Filter Sheet */}
			<Sheet open={mobileFiltersOpen} onOpenChange={setMobileFiltersOpen}>
				<SheetContent side="left" className="w-72 p-0">
					<SheetHeader className="p-4 border-b border-border/10">
						<SheetTitle className="text-sm font-medium">Filters</SheetTitle>
						<SheetDescription className="text-xs text-muted-foreground/50">
							{modalityCounts.total} models available
						</SheetDescription>
					</SheetHeader>
					<div className="p-4 overflow-y-auto">{filterContent}</div>
				</SheetContent>
			</Sheet>

			{/* Model grid */}
			<div
				className={`flex-1 overflow-auto pb-8 ${isMobile ? "px-4" : "px-4 sm:px-8"}`}
			>
				{foundBits.isLoading ? (
					<ModelCatalogSkeleton />
				) : filteredModels.length === 0 ? (
					<div className="flex flex-col items-center justify-center py-32 text-center">
						<div className="rounded-full bg-muted/30 p-5 mb-5">
							<Search className="h-7 w-7 text-muted-foreground/40" />
						</div>
						<p className="text-sm text-foreground/60 mb-1">
							{searchTerm
								? `Nothing found for \u201C${searchTerm}\u201D`
								: "No models found"}
						</p>
						<p className="text-xs text-muted-foreground/60">
							Try adjusting your filters
						</p>
						{(searchTerm || activeFilterCount > 0) && (
							<button
								type="button"
								onClick={() => {
									setSearchTerm("");
									search("");
									resetFilters();
								}}
								className="mt-4 text-xs text-muted-foreground/40 hover:text-foreground transition-colors px-4 py-1.5 rounded-full border border-border/30 hover:border-border/50 hover:bg-muted/30"
							>
								Clear all filters
							</button>
						)}
					</div>
				) : (
					<div
						className={viewMode === "grid" ? "grid gap-3" : "space-y-2"}
						style={
							viewMode === "grid"
								? {
										gridTemplateColumns:
											"repeat(auto-fill, minmax(280px, 1fr))",
									}
								: undefined
						}
					>
						{filteredModels.map((bit) => (
							<ModelCard
								key={bit.id}
								bit={bit}
								variant={viewMode}
								onClick={() => setSelectedModel(bit)}
							/>
						))}
					</div>
				)}
			</div>

			<ModelDetailSheet
				bit={selectedModel}
				open={selectedModel !== null}
				onOpenChange={(open) => !open && setSelectedModel(null)}
				webMode={webMode}
			/>
		</main>
	);
}

function ModalityChip({
	active,
	onClick,
	icon: Icon,
	label,
}: {
	active: boolean;
	onClick: () => void;
	icon: LucideIcon;
	label: string;
}) {
	return (
		<button
			type="button"
			onClick={onClick}
			className={`flex items-center gap-1.5 px-3 py-1 rounded-full text-xs transition-all ${
				active
					? "bg-foreground/10 text-foreground"
					: "text-muted-foreground/40 hover:text-muted-foreground/70 hover:bg-muted/20"
			}`}
		>
			<Icon className="h-3 w-3" />
			{label}
		</button>
	);
}

function FilterCheckbox({
	checked,
	onCheckedChange,
	icon: Icon,
	iconColor,
	label,
}: {
	checked: boolean;
	onCheckedChange: (checked: boolean | "indeterminate") => void;
	icon: LucideIcon;
	iconColor: string;
	label: string;
}) {
	return (
		<label className="flex items-center gap-2.5 text-sm cursor-pointer text-muted-foreground/70 hover:text-foreground transition-colors">
			<Checkbox checked={checked} onCheckedChange={onCheckedChange} />
			<Icon className={`h-3.5 w-3.5 ${iconColor}`} />
			<span>{label}</span>
		</label>
	);
}

function ModelCatalogSkeleton() {
	return (
		<div className="space-y-8 pt-2">
			<div
				className="grid gap-3"
				style={{ gridTemplateColumns: "repeat(auto-fill, minmax(280px, 1fr))" }}
			>
				{Array.from({ length: 8 }).map((_, i) => (
					<Skeleton
						key={`skel-model-${i.toString()}`}
						className="h-48 rounded-xl"
					/>
				))}
			</div>
		</div>
	);
}
