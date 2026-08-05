"use client";

import {
	AudioLines,
	Boxes,
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
	Mic,
	PackageCheck,
	Plus,
	Search,
	Shield,
	Sparkles,
	Type,
	Wand2,
	X,
	Zap,
} from "lucide-react";
import { useCallback, useEffect, useId, useMemo, useState } from "react";
import { toast } from "sonner";
import { useInvalidateInvoke, useInvoke } from "../../../hooks/index";
import { useIsMobile } from "../../../hooks/use-mobile";
import { useSearch } from "../../../hooks/use-search-index";
import { Bit } from "../../../lib/bit/bit";
import { filterHostableLlmModels } from "../../../lib/bit/local-model-filter";
import { isMlxModelBit } from "../../../lib/bit/mlx-model-pack";
import type { IBit } from "../../../lib/schema/bit/bit";
import { IBitTypes } from "../../../lib/schema/bit/bit";
import type { ILlmParameters } from "../../../lib/schema/bit/bit/llm-parameters";
import { useBackend } from "../../../state/backend-state";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
	Button,
	Input,
	ModelCard,
	ModelDetailSheet,
	ProviderGlyph,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Slider,
	formatContextLength,
	providerLabel,
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
import { AddCustomModelDialog } from "./add-custom-model-dialog";

type SortOption =
	| "name"
	| "updated"
	| "context"
	| "speed"
	| "cost"
	| "reasoning"
	| "coding";
type ViewMode = "grid" | "list";
type InputModality = "text" | "image" | "speech";
type OutputModality = "text" | "embedding" | "speech";

function getBitModality(type: IBitTypes): {
	input: InputModality;
	output: OutputModality;
} {
	switch (type) {
		case IBitTypes.Llm:
			return { input: "text", output: "text" };
		case IBitTypes.Vlm:
			return { input: "image", output: "text" };
		case IBitTypes.Tts:
			return { input: "text", output: "speech" };
		case IBitTypes.Stt:
			return { input: "speech", output: "text" };
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
	// An MLX root looks artifact-free but downloads its inline manifest locally,
	// so it is never hosted — and never runnable in the browser.
	if (isMlxModelBit(bit)) return false;
	return (
		(bit.dependencies?.length ?? 0) === 0 &&
		((bit.size ?? 0) === 0 || !bit.download_link)
	);
}

interface AIModelPageProps {
	webMode?: boolean;
}

export function AIModelPage({ webMode = false }: AIModelPageProps) {
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);
	const isMobile = useIsMobile();
	const [customDialogOpen, setCustomDialogOpen] = useState(false);
	const [editingCustomBit, setEditingCustomBit] = useState<IBit | null>(null);
	const [deleteCustomTarget, setDeleteCustomTarget] = useState<IBit | null>(
		null,
	);
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
		new Set(["text", "image", "speech"]),
	);
	const [outputModalities, setOutputModalities] = useState<Set<OutputModality>>(
		new Set(["text", "embedding", "speech"]),
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
					IBitTypes.Tts,
					IBitTypes.Stt,
					IBitTypes.Embedding,
					IBitTypes.ImageEmbedding,
				],
			},
		],
		typeof profile.data !== "undefined",
		[profile.data?.id ?? ""],
	);

	const customBits = useInvoke(
		backend.bitState.listCustomBits,
		backend.bitState,
		[],
		typeof profile.data !== "undefined",
		[profile.data?.id ?? ""],
	);

	const customBitIds = useMemo(
		() => new Set((customBits.data ?? []).map((bit) => bit.id)),
		[customBits.data],
	);

	const allBits = useMemo(() => {
		const merged = new Map<string, IBit>();
		for (const bit of foundBits.data ?? []) merged.set(bit.id, bit);
		for (const bit of customBits.data ?? []) merged.set(bit.id, bit);
		return Array.from(merged.values());
	}, [foundBits.data, customBits.data]);
	const { canHostLlamaCPP, canHostMLX } = backend.capabilities();
	const hostableBits = useMemo(
		() =>
			filterHostableLlmModels(allBits, {
				canHostLlamaCPP,
				canHostMLX,
			}),
		[allBits, canHostLlamaCPP, canHostMLX],
	);

	const imageBlacklist = useCallback(async () => {
		if (!foundBits.data) return;
		// Best effort: a model whose dependencies cannot be resolved must not
		// take down the whole catalog page with an unhandled rejection.
		const dependencies = await Promise.allSettled(
			foundBits.data
				.filter((bit) => bit.type === IBitTypes.ImageEmbedding)
				.map((bit) =>
					Bit.fromObject(bit).setBackend(backend).fetchDependencies(),
				),
		);
		const bl = new Set<string>(
			dependencies.flatMap((result) => {
				if (result.status === "rejected") {
					console.warn(
						"Failed to resolve image embedding dependencies",
						result.reason,
					);
					return [];
				}
				return (result.value.bits ?? [])
					.filter((bit) => bit.type !== IBitTypes.ImageEmbedding)
					.map((bit) => bit.id);
			}),
		);
		setBlacklist(bl);
	}, [backend, foundBits.data]);

	const [installedBits, setInstalledBits] = useState<Set<string>>(new Set());

	const searchedBits = useSearch(hostableBits, searchTerm, {
		fields: [
			"meta.en.name",
			"meta.en.description",
			"meta.en.long_description",
			"meta.en.tags",
			"id",
			"type",
			"authors",
			"file_name",
			"hub",
			"parameters.provider.provider_name",
			"parameters.provider.model_id",
		],
		boost: {
			"meta.en.name": 4,
			id: 2,
			"parameters.provider.provider_name": 2,
			type: 1.5,
			"meta.en.description": 1,
			"meta.en.long_description": 0.5,
		},
	});

	useEffect(() => {
		if (!foundBits.data) return;
		imageBlacklist();
	}, [foundBits.data, imageBlacklist]);

	useEffect(() => {
		if (hostableBits.length === 0 || !profile.data || !checkInstalled) return;
		const checkInstalledAll = async () => {
			const installedSet = new Set<string>();
			for (const bit of hostableBits) {
				const isInstalled = await checkInstalled(bit);
				if (isInstalled) installedSet.add(bit.id);
			}
			setInstalledBits(installedSet);
		};
		checkInstalledAll();
	}, [hostableBits, profile.data, checkInstalled]);

	const providers = useMemo(() => {
		const providerSet = new Set<string>();
		for (const model of hostableBits) {
			const params = model.parameters as ILlmParameters | undefined;
			if (params?.provider?.provider_name) {
				providerSet.add(params.provider.provider_name);
			}
		}
		return Array.from(providerSet).sort();
	}, [hostableBits]);

	const maxContextLength = useMemo(() => {
		if (hostableBits.length === 0) return 2000000;
		return Math.max(
			...hostableBits.map(
				(m) =>
					(m.parameters as ILlmParameters | undefined)?.context_length ?? 0,
			),
			128000,
		);
	}, [hostableBits]);

	const profileBitIds = useMemo(() => {
		return new Set(profile.data?.bits?.map((id) => id.split(":").pop()) ?? []);
	}, [profile.data]);

	/**
	 * Custom bits are a user-owned library — the whole catalog lists them so
	 * credentials are entered once — but a model only counts as "mine" once it
	 * is added to the active profile, exactly like a public bit.
	 */
	const isMine = useCallback(
		(bit: IBit) => profileBitIds.has(bit.id),
		[profileBitIds],
	);

	const filteredModels = useMemo(() => {
		let models = searchedBits;
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

		if (inputModalities.size < 3 || outputModalities.size < 3) {
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

		// A live query already orders by relevance; only an explicitly picked
		// sort ("updated" is the untouched default) overrides it.
		if (searchTerm.trim() && sortBy === "updated") return models;

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
		searchedBits,
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

	const rails = useMemo(() => {
		const defs: {
			id: string;
			label: string;
			icon: LucideIcon;
			color: string;
			match: (bit: IBit) => boolean;
		}[] = [
			{
				id: "rail-chat",
				label: "Chat & reasoning",
				icon: MessageSquare,
				color: "var(--m-chat)",
				match: (b) => LLM_LIKE_TYPES.has(b.type),
			},
			{
				id: "rail-stt",
				label: "Speech-to-text",
				icon: Mic,
				color: "var(--m-audio)",
				match: (b) => b.type === IBitTypes.Stt,
			},
			{
				id: "rail-tts",
				label: "Text-to-speech",
				icon: AudioLines,
				color: "var(--m-speech)",
				match: (b) => b.type === IBitTypes.Tts,
			},
			{
				id: "rail-embed",
				label: "Embeddings",
				icon: FileSearchIcon,
				color: "var(--m-embed)",
				match: (b) =>
					b.type === IBitTypes.Embedding || b.type === IBitTypes.ImageEmbedding,
			},
		];
		const claimed = new Set<string>();
		const built = defs.map((def) => {
			const items = filteredModels.filter((bit) => {
				if (!def.match(bit)) return false;
				claimed.add(bit.id);
				return true;
			});
			return { ...def, items };
		});
		const rest = filteredModels.filter((bit) => !claimed.has(bit.id));
		if (rest.length > 0) {
			built.push({
				id: "rail-other",
				label: "Other models",
				icon: Sparkles,
				color: "var(--m-video)",
				match: () => true,
				items: rest,
			});
		}
		return built;
	}, [filteredModels]);

	const populatedRails = useMemo(
		() => rails.filter((rail) => rail.items.length > 0),
		[rails],
	);

	const profileModels = useMemo(
		() => filteredModels.filter(isMine),
		[filteredModels, isMine],
	);

	const profileGlyphModels = useMemo(
		() => hostableBits.filter(isMine),
		[hostableBits, isMine],
	);

	const [activeRail, setActiveRail] = useState<string>("rail-profile");
	const railIds = useMemo(
		() => [
			"rail-profile",
			...rails.filter((r) => r.items.length).map((r) => r.id),
		],
		[rails],
	);

	useEffect(() => {
		const sections = railIds
			.map((id) => document.getElementById(id))
			.filter((el): el is HTMLElement => el !== null);
		if (sections.length === 0) return;
		const observer = new IntersectionObserver(
			(entries) => {
				for (const entry of entries) {
					if (entry.isIntersecting) setActiveRail(entry.target.id);
				}
			},
			{ rootMargin: "-140px 0px -58% 0px", threshold: 0 },
		);
		for (const section of sections) observer.observe(section);
		return () => observer.disconnect();
	}, [railIds]);

	const jumpToRail = useCallback((id: string) => {
		setActiveRail(id);
		document.getElementById(id)?.scrollIntoView({
			behavior: "smooth",
			block: "start",
		});
	}, []);

	const modalityCounts = useMemo(() => {
		const counts = { text: 0, image: 0, embedding: 0, speech: 0, total: 0 };
		const validBits = hostableBits.filter((bit) => !blacklist.has(bit.id));
		counts.total = validBits.length;
		for (const bit of validBits) {
			const modality = getBitModality(bit.type);
			if (modality.input === "text") counts.text++;
			if (modality.input === "image") counts.image++;
			if (modality.input === "speech") counts.speech++;
			if (modality.output === "embedding") counts.embedding++;
			if (modality.output === "speech") counts.speech++;
		}
		return counts;
	}, [hostableBits, blacklist]);

	const activeFilterCount = useMemo(() => {
		let count = 0;
		if (providerFilter !== "all") count++;
		if (showInProfileOnly) count++;
		if (showDownloadedOnly) count++;
		if (contextLengthFilter[0] > 0 || contextLengthFilter[1] < maxContextLength)
			count++;
		if (inputModalities.size < 2) count++;
		if (outputModalities.size < 3) count++;
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
		setInputModalities(new Set(["text", "image", "speech"]));
		setOutputModalities(new Set(["text", "embedding", "speech"]));
		setCapabilityFilters({
			reasoning: 0,
			coding: 0,
			speed: 0,
			cost: 0,
			creativity: 0,
			factuality: 0,
		});
	}, [maxContextLength]);

	const openAddCustomModel = useCallback(() => {
		setEditingCustomBit(null);
		setCustomDialogOpen(true);
	}, []);

	const openEditCustomModel = useCallback((bit: IBit) => {
		setEditingCustomBit(bit);
		setCustomDialogOpen(true);
	}, []);

	const confirmDeleteCustomModel = useCallback(async () => {
		const target = deleteCustomTarget;
		if (!target) return;
		setDeleteCustomTarget(null);
		try {
			await backend.bitState.deleteCustomBit(target.id);
			await Promise.all([
				invalidate(backend.bitState.listCustomBits, []),
				invalidate(backend.bitState.getProfileBits, []),
			]);
			toast.success("Custom model deleted");
		} catch (error) {
			toast.error(
				`Failed to delete model: ${
					error instanceof Error ? error.message : String(error)
				}`,
			);
		}
	}, [deleteCustomTarget, backend.bitState, invalidate]);

	const renderCard = useCallback(
		(bit: IBit) => {
			const custom = customBitIds.has(bit.id);
			return (
				<ModelCard
					key={bit.id}
					bit={bit}
					variant={viewMode}
					isCustom={custom}
					onClick={() => setSelectedModel(bit)}
					onEdit={custom ? openEditCustomModel : undefined}
					onDelete={custom ? setDeleteCustomTarget : undefined}
				/>
			);
		},
		[customBitIds, openEditCustomModel, viewMode],
	);

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

	// -m-4 cancels the settings shell padding so the sticky header spans the full
	// width; the rows below re-add their own gutters.
	return (
		<main className="-m-4 flex min-h-0 flex-1 flex-col overflow-y-auto">
			{/* Opaque: WebKit disables backdrop-filter, so a blur-only header
			    would let the cards scroll straight through it */}
			<div className="sticky top-0 z-30 border-b border-border/60 bg-background">
				<div
					className={`mx-auto flex w-full max-w-[1240px] flex-wrap items-center gap-x-3 gap-y-2 pt-4 pb-2.5 ${isMobile ? "px-4" : "px-4 sm:px-8"}`}
				>
					<div className="mr-auto flex min-w-0 items-center gap-2.5">
						<span className="grid h-8 w-8 shrink-0 place-items-center rounded-lg border border-border bg-muted text-foreground/70">
							<Boxes className="h-4.25 w-4.25" />
						</span>
						<span className="flex min-w-0 flex-col leading-tight">
							<span className="text-[15px] font-semibold tracking-tight">
								Models
							</span>
							<span className="text-[11.5px] text-muted-foreground">
								{hostableBits.length} available &middot;{" "}
								{profileGlyphModels.length} in your profile
							</span>
						</span>
					</div>

					<ProfileToggle
						models={profileGlyphModels}
						active={showInProfileOnly}
						onToggle={() => setShowInProfileOnly((v) => !v)}
					/>

					<Button
						onClick={openAddCustomModel}
						size="sm"
						className="h-8 shrink-0 gap-1.5 px-3 text-xs"
					>
						<Plus className="h-3.5 w-3.5" />
						<span className="hidden sm:inline">Add custom model</span>
						<span className="sm:hidden">Add</span>
					</Button>
				</div>

				<div
					className={`mx-auto flex w-full max-w-[1240px] items-center gap-2 pb-2.5 ${isMobile ? "px-4" : "px-4 sm:px-8"}`}
				>
					<div className="relative min-w-0 flex-1">
						<Search className="absolute left-3.5 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground/60 pointer-events-none" />
						<Input
							placeholder="Search models, providers, capabilities…"
							value={searchTerm}
							onChange={(e) => setSearchTerm(e.target.value)}
							className="h-9 pl-10 text-sm bg-muted/40 border-border/60 focus:bg-background"
						/>
						{searchTerm && (
							<button
								type="button"
								aria-label="Clear search"
								onClick={() => setSearchTerm("")}
								className="absolute right-3 top-1/2 -translate-y-1/2 text-muted-foreground/60 hover:text-foreground transition-colors"
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
							<SelectTrigger className="h-9 w-auto gap-1.5 rounded-md border-border/60 bg-transparent px-3 text-xs text-muted-foreground hover:text-foreground focus:ring-0">
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
									variant="outline"
									size="icon"
									className={`h-9 w-9 rounded-md border-border/60 ${
										viewMode === "list"
											? "bg-muted text-foreground"
											: "text-muted-foreground hover:text-foreground"
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
									variant="outline"
									size="icon"
									className={`relative h-9 w-9 rounded-md border-border/60 ${
										filtersExpanded || activeFilterCount > 0
											? "bg-muted text-foreground"
											: "text-muted-foreground hover:text-foreground"
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

				{/* Jump links earn their row only once the page is long enough to scroll */}
				{populatedRails.length > 2 && (
					<nav
						aria-label="Jump to a capability"
						className={`mx-auto flex w-full max-w-[1240px] gap-1.5 overflow-x-auto pb-2.5 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden ${isMobile ? "px-4" : "px-4 sm:px-8"}`}
					>
						<RailChip
							icon={Sparkles}
							label="Your profile"
							count={profileModels.length}
							owned={profileModels.length}
							active={activeRail === "rail-profile"}
							onClick={() => jumpToRail("rail-profile")}
						/>
						{populatedRails.map((rail) => (
							<RailChip
								key={rail.id}
								icon={rail.icon}
								label={rail.label}
								count={rail.items.length}
								owned={rail.items.filter(isMine).length}
								active={activeRail === rail.id}
								onClick={() => jumpToRail(rail.id)}
							/>
						))}
					</nav>
				)}
			</div>

			<div
				className={`mx-auto w-full max-w-[1240px] pt-4 pb-3 ${isMobile ? "px-4" : "px-4 sm:px-8"}`}
			>
				{/* Modality filters, labelled so the two rows of chips can't be confused */}
				<div className="flex flex-wrap items-center gap-x-3 gap-y-2">
					<div className="flex items-center gap-1.5">
						<FilterGroupLabel>Accepts</FilterGroupLabel>
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
						<ModalityChip
							active={inputModalities.has("speech")}
							onClick={() => toggleInputModality("speech")}
							icon={Mic}
							label="Audio"
						/>
					</div>

					<span className="hidden h-4 w-px bg-border sm:block" />

					<div className="flex items-center gap-1.5">
						<FilterGroupLabel>Produces</FilterGroupLabel>
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
						<ModalityChip
							active={outputModalities.has("speech")}
							onClick={() => toggleOutputModality("speech")}
							icon={AudioLines}
							label="Speech"
						/>
					</div>

					<span className="ml-auto font-mono text-[11px] tabular-nums text-muted-foreground">
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

			{/* Catalog */}
			<div
				className={`mx-auto flex w-full max-w-[1240px] flex-1 flex-col gap-7 pb-12 ${isMobile ? "px-4" : "px-4 sm:px-8"}`}
			>
				{foundBits.isLoading ? (
					<ModelCatalogSkeleton />
				) : filteredModels.length === 0 ? (
					<div className="flex flex-col items-center justify-center rounded-xl border border-dashed border-border py-24 text-center">
						<Search className="mb-3 h-6 w-6 text-muted-foreground/50" />
						<p className="text-sm font-medium">
							{searchTerm
								? `Nothing matches “${searchTerm}”`
								: "No models match these filters"}
						</p>
						<p className="mt-1 text-xs text-muted-foreground">
							Widen the modality or capability filters to see more.
						</p>
						{(searchTerm || activeFilterCount > 0) && (
							<Button
								variant="outline"
								size="sm"
								className="mt-4 h-8 text-xs"
								onClick={() => {
									setSearchTerm("");
									resetFilters();
								}}
							>
								Clear search and filters
							</Button>
						)}
					</div>
				) : (
					<>
						{/* State summary first, catalog detail below */}
						<ProfileSummary
							id="rail-profile"
							models={profileGlyphModels}
							onSelect={setSelectedModel}
							onAdd={openAddCustomModel}
						/>

						{populatedRails.map((rail) => (
							<ModelSection
								key={rail.id}
								id={rail.id}
								label={rail.label}
								icon={rail.icon}
								color={rail.color}
								items={rail.items}
								owned={rail.items.filter(isMine).length}
								view={viewMode}
								renderCard={renderCard}
							/>
						))}

						<footer className="flex flex-wrap items-center gap-x-2 gap-y-1 border-t border-border/60 pt-4 text-xs text-muted-foreground">
							<span className="font-mono tabular-nums">
								{filteredModels.length} of {hostableBits.length}
							</span>
							<span>models shown</span>
						</footer>
					</>
				)}
			</div>

			<ModelDetailSheet
				bit={selectedModel}
				open={selectedModel !== null}
				onOpenChange={(open) => !open && setSelectedModel(null)}
				webMode={webMode}
			/>

			<AddCustomModelDialog
				open={customDialogOpen}
				onOpenChange={setCustomDialogOpen}
				existingBit={editingCustomBit}
				webMode={webMode}
			/>

			<AlertDialog
				open={deleteCustomTarget !== null}
				onOpenChange={(open) => !open && setDeleteCustomTarget(null)}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>Delete custom model?</AlertDialogTitle>
						<AlertDialogDescription>
							{`"${deleteCustomTarget?.meta?.en?.name ?? "This model"}" and its stored credentials will be removed. Flows using it will no longer resolve this model.`}
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel>Cancel</AlertDialogCancel>
						<AlertDialogAction onClick={confirmDeleteCustomModel}>
							Delete
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</main>
	);
}

/** One capability group of the catalog, as a grid that fills the row. */
function ModelSection({
	id,
	label,
	icon: Icon,
	color,
	items,
	owned,
	view,
	renderCard,
}: Readonly<{
	id: string;
	label: string;
	icon: LucideIcon;
	color: string;
	items: IBit[];
	owned: number;
	view: ViewMode;
	renderCard: (bit: IBit) => React.ReactNode;
}>) {
	return (
		<section id={id} aria-labelledby={`${id}-heading`} className="scroll-mt-32">
			<div className="mb-3 flex items-center gap-2.5 border-b border-border/60 pb-2">
				<span
					style={{ "--rc": color } as React.CSSProperties}
					className="grid h-6 w-6 shrink-0 place-items-center rounded-md bg-[color-mix(in_srgb,var(--rc)_14%,transparent)] text-(--rc)"
				>
					<Icon className="h-3.5 w-3.5" />
				</span>
				<h2
					id={`${id}-heading`}
					className="m-0 text-[12px] font-semibold uppercase tracking-[0.12em] text-muted-foreground"
				>
					{label}
				</h2>
				<span className="font-mono text-[11px] tabular-nums text-muted-foreground/70">
					{items.length}
				</span>
				{owned > 0 && (
					<span className="ml-auto text-[11px] text-muted-foreground">
						<span className="font-mono tabular-nums">{owned}</span> in your
						profile
					</span>
				)}
			</div>
			<ul
				className={`m-0 grid list-none p-0 ${view === "list" ? "gap-1.5" : "gap-3"}`}
				style={{
					gridTemplateColumns:
						view === "list"
							? "minmax(0, 1fr)"
							: "repeat(auto-fill, minmax(258px, 1fr))",
				}}
			>
				{items.map((bit) => (
					<li key={bit.id} className="min-w-0">
						{renderCard(bit)}
					</li>
				))}
			</ul>
		</section>
	);
}

/**
 * What the profile currently runs on, as a compact strip — the same models
 * still appear as full cards in their capability group below, so this stays a
 * summary and never repeats the detail.
 */
function ProfileSummary({
	id,
	models,
	onSelect,
	onAdd,
}: Readonly<{
	id: string;
	models: IBit[];
	onSelect: (bit: IBit) => void;
	onAdd: () => void;
}>) {
	return (
		<section
			id={id}
			aria-labelledby={`${id}-heading`}
			className="scroll-mt-32 rounded-xl border border-border bg-muted/40 p-3.5 dark:border-white/10"
		>
			<div className="mb-2.5 flex items-center gap-2.5">
				<Sparkles className="h-3.5 w-3.5 shrink-0 text-primary" />
				<h2
					id={`${id}-heading`}
					className="m-0 text-[12px] font-semibold uppercase tracking-[0.12em] text-muted-foreground"
				>
					In your profile
				</h2>
				<span className="ml-auto text-[11px] text-muted-foreground">
					Available to every flow in this workspace
				</span>
			</div>

			{models.length === 0 ? (
				<div className="flex flex-wrap items-center gap-2 text-[13px] text-muted-foreground">
					<span>
						No models yet — add one below, or connect your own provider.
					</span>
					<Button
						variant="outline"
						size="sm"
						onClick={onAdd}
						className="h-7 gap-1.5 bg-background text-xs"
					>
						<Plus className="h-3.5 w-3.5" />
						Add custom model
					</Button>
				</div>
			) : (
				<ul className="m-0 flex list-none flex-wrap gap-1.5 p-0">
					{models.map((bit) => (
						<li key={bit.id}>
							<button
								type="button"
								onClick={() => onSelect(bit)}
								title={`${bit.meta?.en?.name ?? bit.id} — ${providerLabel(bit)}`}
								className="flex h-8 max-w-60 items-center gap-2 rounded-lg border border-border bg-background pl-1.5 pr-2.5 text-left transition-colors hover:border-primary/40 hover:bg-primary/5 dark:border-white/10"
							>
								<ProviderGlyph bit={bit} size={20} className="rounded-[5px]" />
								<span className="truncate text-[12.5px] font-medium">
									{bit.meta?.en?.name ?? bit.id}
								</span>
								<span className="shrink-0 truncate text-[11px] text-muted-foreground">
									{providerLabel(bit)}
								</span>
							</button>
						</li>
					))}
				</ul>
			)}
		</section>
	);
}

/** Header control: who is in the profile, and a shortcut to filter down to them. */
function ProfileToggle({
	models,
	active,
	onToggle,
}: Readonly<{ models: IBit[]; active: boolean; onToggle: () => void }>) {
	const shown = models.slice(0, 4);
	const rest = models.length - shown.length;
	return (
		<button
			type="button"
			onClick={onToggle}
			aria-pressed={active}
			aria-label={
				active
					? "Show the whole catalog"
					: `Show only the ${models.length} models in your profile`
			}
			className={`flex h-8 shrink-0 items-center gap-2 rounded-md border px-2.5 text-xs font-medium transition-colors ${
				active
					? "border-primary/40 bg-primary/10 text-primary"
					: "border-border text-muted-foreground hover:bg-muted hover:text-foreground dark:border-white/10"
			}`}
		>
			<Sparkles className="h-3.5 w-3.5 shrink-0" />
			{shown.length > 0 && (
				<span className="flex items-center pl-1" aria-hidden="true">
					{shown.map((bit) => (
						<ProviderGlyph
							key={bit.id}
							bit={bit}
							size={18}
							className="-ml-1 rounded-[5px] ring-2 ring-background"
						/>
					))}
					{rest > 0 && (
						<span className="-ml-1 grid h-4.5 min-w-4.5 place-items-center rounded-[5px] bg-muted px-1 font-mono text-[9px] font-semibold tabular-nums text-muted-foreground ring-2 ring-background">
							+{rest}
						</span>
					)}
				</span>
			)}
			<span className="whitespace-nowrap">In profile</span>
		</button>
	);
}

/** Capability jump chip with a dot when you already own models in that group. */
function RailChip({
	icon: Icon,
	label,
	count,
	owned,
	active,
	onClick,
}: Readonly<{
	icon: LucideIcon;
	label: string;
	count: number;
	owned: number;
	active: boolean;
	onClick: () => void;
}>) {
	return (
		<button
			type="button"
			onClick={onClick}
			aria-pressed={active}
			className={`flex h-7 shrink-0 snap-start items-center gap-1.5 whitespace-nowrap rounded-md border px-2.5 text-[12px] font-medium transition-colors ${
				active
					? "border-foreground/20 bg-muted text-foreground"
					: "border-transparent text-muted-foreground hover:bg-muted/60 hover:text-foreground"
			}`}
		>
			<Icon className="h-3.5 w-3.5" />
			<span>{label}</span>
			{owned > 0 && (
				<span
					title={`${owned} in your profile`}
					className="h-1.5 w-1.5 rounded-full bg-primary"
				/>
			)}
			<span className="font-mono text-[10px] tabular-nums opacity-60">
				{count}
			</span>
		</button>
	);
}

function FilterGroupLabel({
	children,
}: Readonly<{ children: React.ReactNode }>) {
	return (
		<span className="text-[10px] font-semibold uppercase tracking-[0.12em] text-muted-foreground/70">
			{children}
		</span>
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
			aria-pressed={active}
			className={`flex h-7 items-center gap-1.5 rounded-md border px-2.5 text-xs transition-colors ${
				active
					? "border-border bg-muted text-foreground dark:border-white/10"
					: "border-transparent text-muted-foreground/60 hover:bg-muted/50 hover:text-foreground"
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
	const id = useId();
	return (
		<label
			htmlFor={id}
			className="flex items-center gap-2.5 text-sm cursor-pointer text-muted-foreground/70 hover:text-foreground transition-colors"
		>
			<Checkbox id={id} checked={checked} onCheckedChange={onCheckedChange} />
			<Icon className={`h-3.5 w-3.5 ${iconColor}`} />
			<span>{label}</span>
		</label>
	);
}

function ModelCatalogSkeleton() {
	return (
		<div className="flex flex-col gap-7">
			<Skeleton className="h-19 rounded-xl" />
			{[0, 1].map((section) => (
				<div key={`skel-section-${section}`} className="flex flex-col gap-3">
					<Skeleton className="h-4 w-40" />
					<div
						className="grid gap-3"
						style={{
							gridTemplateColumns: "repeat(auto-fill, minmax(258px, 1fr))",
						}}
					>
						{Array.from({ length: 4 }).map((_, i) => (
							<Skeleton
								key={`skel-model-${section}-${i.toString()}`}
								className="h-46 rounded-xl"
							/>
						))}
					</div>
				</div>
			))}
		</div>
	);
}
