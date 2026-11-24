"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AudioLines,
	Boxes,
	Brain,
	ChartNoAxesCombined,
	ChevronRight,
	CloudIcon,
	Code2,
	Cpu,
	FileSearchIcon,
	Globe,
	ImageIcon,
	KeyRound,
	LaptopMinimal,
	LayoutGrid,
	LayoutList,
	Lightbulb,
	ListChecks,
	type LucideIcon,
	MapPin,
	MessageSquare,
	Mic,
	PackageCheck,
	Plus,
	RefreshCw,
	Search,
	Shield,
	SlidersHorizontal,
	Sparkles,
	Type,
	Video,
	Wand2,
	X,
	Zap,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";
import { useInvalidateInvoke, useInvoke } from "../../../hooks/index";
import { useIsMobile } from "../../../hooks/use-mobile";
import { useSearch } from "../../../hooks/use-search-index";
import { Bit } from "../../../lib/bit/bit";
import { isEuHosted } from "../../../lib/bit/hosting-region";
import { filterHostableLlmModels } from "../../../lib/bit/local-model-filter";
import { isMlxModelBit } from "../../../lib/bit/mlx-model-pack";
import {
	compareModelCapability,
	meetsModelCapabilityMinimum,
} from "../../../lib/bit/model-capabilities";
import {
	MODEL_BIT_TYPES,
	listableModels,
	searchAllBitsOfType,
} from "../../../lib/bit/model-listing";
import {
	type ModelPrice,
	type PriceDeployment,
	bitPrice,
	priceTiers,
	priceValue,
} from "../../../lib/bit/model-pricing";
import { bitRating, compareRating } from "../../../lib/bit/model-rating";
import { asArray } from "../../../lib/response-shape";
import type { IBit } from "../../../lib/schema/bit/bit";
import { IBitTypes } from "../../../lib/schema/bit/bit";
import type { ILlmParameters } from "../../../lib/schema/bit/bit/llm-parameters";
import { cn } from "../../../lib/utils";
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
	type Modality,
	ModelCard,
	ModelDetailDialog,
	RATING_TONE,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Slider,
	bitModalities,
	formatContextLength,
	supportsRemoteEmbeddingExecution,
} from "../../ui";
import { Popover, PopoverContent, PopoverTrigger } from "../../ui/popover";
import { Skeleton } from "../../ui/skeleton";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../ui/tooltip";
import { AddCustomModelDialog } from "./add-custom-model-dialog";

type SortOption =
	| "updated"
	| "name"
	| "rating"
	| "price"
	| "context"
	| "speed"
	| "reasoning"
	| "coding";
type ViewMode = "grid" | "list";
type RunFilter = "hosted" | "local" | "ownKey";
type PriceFilter = "any" | "free" | "low";
type RatingFilter = "any" | "good" | "top";
type InputFilter = Extract<Modality, "text" | "image" | "audio" | "series">;

const ALL = "all";
const CAPABILITIES: { key: string; icon: LucideIcon; color: string }[] = [
	{ key: "reasoning", icon: Brain, color: "text-orange-500" },
	{ key: "coding", icon: Code2, color: "text-blue-500" },
	{ key: "speed", icon: Zap, color: "text-amber-500" },
	{ key: "cost", icon: Sparkles, color: "text-green-500" },
	{ key: "creativity", icon: Lightbulb, color: "text-yellow-500" },
	{ key: "factuality", icon: Shield, color: "text-emerald-500" },
	{ key: "function_calling", icon: Wand2, color: "text-purple-500" },
	{ key: "multilinguality", icon: Globe, color: "text-cyan-500" },
];
const NO_CAPABILITY_FILTERS: Record<string, number> = Object.fromEntries(
	CAPABILITIES.map((capability) => [capability.key, 0]),
);

const LLM_LIKE_TYPES = new Set([IBitTypes.Llm, IBitTypes.Vlm]);

function providerName(bit: IBit): string {
	return (
		(bit.parameters as ILlmParameters | undefined)?.provider?.provider_name ??
		""
	);
}

function isHostedModel(bit: IBit): boolean {
	if (providerName(bit) === "custom:claude-code") return false;
	// An MLX root looks artifact-free but downloads its inline manifest locally,
	// so it is never hosted — and never runnable in the browser.
	if (isMlxModelBit(bit)) return false;
	return (
		(bit.dependencies?.length ?? 0) === 0 &&
		((bit.size ?? 0) === 0 || !bit.download_link)
	);
}

/** The catalog's own view of where a model runs, before a card resolves its pack size. */
function catalogDeployment(bit: IBit): PriceDeployment {
	if (bit.type === IBitTypes.Forecasting) return "native";
	if (isHostedModel(bit) || supportsRemoteEmbeddingExecution(bit))
		return "hosted";
	return "local";
}

function runKind(bit: IBit, price: ModelPrice): RunFilter {
	if (price.kind === "ownKey") return "ownKey";
	return catalogDeployment(bit) === "hosted" ? "hosted" : "local";
}

interface AIModelPageProps {
	webMode?: boolean;
}

interface Rail {
	id: string;
	tab: string;
	title: string;
	icon: LucideIcon;
	color: string;
	preview: number;
	match: (bit: IBit) => boolean;
}

export function AIModelPage({ webMode = false }: AIModelPageProps) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const isMobile = useIsMobile();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);
	const [customDialogOpen, setCustomDialogOpen] = useState(false);
	const [editingCustomBit, setEditingCustomBit] = useState<IBit | null>(null);
	const [deleteCustomTarget, setDeleteCustomTarget] = useState<IBit | null>(
		null,
	);
	const [selectedModel, setSelectedModel] = useState<IBit | null>(null);
	const [searchTerm, setSearchTerm] = useState("");
	const [blacklist, setBlacklist] = useState(new Set<string>());
	const [installedBits, setInstalledBits] = useState<Set<string>>(new Set());
	const [viewMode, setViewMode] = useState<ViewMode>("grid");
	const [sortBy, setSortBy] = useState<SortOption>("updated");
	const [activeTab, setActiveTab] = useState<string>(ALL);
	const [showInProfileOnly, setShowInProfileOnly] = useState(false);
	const [showDownloadedOnly, setShowDownloadedOnly] = useState(false);
	const [showEuHostedOnly, setShowEuHostedOnly] = useState(false);
	const [runFilters, setRunFilters] = useState<Set<RunFilter>>(new Set());
	const [priceFilter, setPriceFilter] = useState<PriceFilter>("any");
	const [ratingFilter, setRatingFilter] = useState<RatingFilter>("any");
	const [inputFilters, setInputFilters] = useState<Set<InputFilter>>(new Set());
	const [providerFilter, setProviderFilter] = useState(ALL);
	const [contextLengthFilter, setContextLengthFilter] = useState<
		[number, number]
	>([0, 2000000]);
	const [capabilityFilters, setCapabilityFilters] = useState(
		NO_CAPABILITY_FILTERS,
	);

	const foundBits = useInvoke(
		searchAllBitsOfType,
		backend.bitState,
		[MODEL_BIT_TYPES],
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
		() => new Set(asArray(customBits.data).map((bit) => bit.id)),
		[customBits.data],
	);

	const allBits = useMemo(() => {
		const merged = new Map<string, IBit>();
		for (const bit of listableModels(asArray(foundBits.data)))
			merged.set(bit.id, bit);
		for (const bit of asArray(customBits.data)) merged.set(bit.id, bit);
		return Array.from(merged.values());
	}, [foundBits.data, customBits.data]);

	const {
		canHostLlamaCPP,
		canHostMLX,
		canUseNativeAgentProviders,
		deviceModels,
	} = backend.capabilities();
	const hostableBits = useMemo(
		() =>
			filterHostableLlmModels(allBits, {
				canHostLlamaCPP,
				canHostMLX,
				canUseNativeAgentProviders: !webMode && canUseNativeAgentProviders,
				deviceModels,
			}),
		[
			allBits,
			canHostLlamaCPP,
			canHostMLX,
			canUseNativeAgentProviders,
			deviceModels,
			webMode,
		],
	);

	useEffect(() => {
		if (!foundBits.data) return;
		let cancelled = false;
		// Best effort: a model whose dependencies cannot be resolved must not
		// take down the whole catalog page with an unhandled rejection.
		void Promise.allSettled(
			asArray(foundBits.data)
				.filter((bit) => bit.type === IBitTypes.ImageEmbedding)
				.map((bit) =>
					Bit.fromObject(bit).setBackend(backend).fetchDependencies(),
				),
		).then((dependencies) => {
			if (cancelled) return;
			setBlacklist(
				new Set(
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
				),
			);
		});
		return () => {
			cancelled = true;
		};
	}, [backend, foundBits.data]);

	useEffect(() => {
		if (hostableBits.length === 0 || !profile.data) return;
		let cancelled = false;
		const checkInstalledAll = async () => {
			const installed = new Set<string>();
			for (const bit of hostableBits) {
				const isInstalled = await backend.bitState
					.isBitInstalled(bit)
					.catch(() => false);
				if (isInstalled) installed.add(bit.id);
			}
			if (!cancelled) setInstalledBits(installed);
		};
		void checkInstalledAll();
		return () => {
			cancelled = true;
		};
	}, [hostableBits, profile.data, backend.bitState]);

	const catalogBits = useMemo(
		() =>
			hostableBits.filter((bit) => {
				if (blacklist.has(bit.id) || bit.meta?.en === undefined) return false;
				// The web app only lists chat and decision models it can host.
				if (
					webMode &&
					(LLM_LIKE_TYPES.has(bit.type) || bit.type === IBitTypes.SystemOne)
				)
					return isHostedModel(bit);
				return true;
			}),
		[hostableBits, blacklist, webMode],
	);

	const prices = useMemo(
		() =>
			new Map(
				catalogBits.map((bit) => [
					bit.id,
					bitPrice(bit, catalogDeployment(bit)),
				]),
			),
		[catalogBits],
	);
	const tiers = useMemo(
		() =>
			priceTiers(
				catalogBits.map((bit) => ({
					id: bit.id,
					type: bit.type,
					price: prices.get(bit.id) ?? { kind: "unlisted" },
				})),
			),
		[catalogBits, prices],
	);

	const searchedBits = useSearch(catalogBits, searchTerm, {
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

	const providers = useMemo(
		() =>
			Array.from(new Set(catalogBits.map(providerName).filter(Boolean))).sort(),
		[catalogBits],
	);

	const maxContextLength = useMemo(
		() =>
			Math.max(
				128000,
				...catalogBits.map(
					(bit) =>
						(bit.parameters as ILlmParameters | undefined)?.context_length ?? 0,
				),
			),
		[catalogBits],
	);

	const profileBitIds = useMemo(
		() => new Set(profile.data?.bits?.map((id) => id.split(":").pop()) ?? []),
		[profile.data],
	);

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
		const models = searchedBits.filter((bit) => {
			if (showInProfileOnly && !profileBitIds.has(bit.id)) return false;
			if (showDownloadedOnly && !installedBits.has(bit.id)) return false;
			if (showEuHostedOnly && !isEuHosted(bit)) return false;
			const price = prices.get(bit.id) ?? { kind: "unlisted" };
			if (runFilters.size > 0 && !runFilters.has(runKind(bit, price)))
				return false;
			if (priceFilter === "free" && price.kind !== "free") return false;
			if (
				priceFilter === "low" &&
				price.kind !== "free" &&
				tiers.get(bit.id) !== 1
			)
				return false;
			if (ratingFilter !== "any") {
				const tier = bitRating(bit)?.tier;
				if (!tier || tier > (ratingFilter === "top" ? 1 : 2)) return false;
			}
			if (inputFilters.size > 0) {
				const inputs = bitModalities(bit.type).inputs;
				for (const input of inputFilters)
					if (!inputs.includes(input)) return false;
			}
			if (providerFilter !== ALL && providerName(bit) !== providerFilter)
				return false;
			const contextLength =
				(bit.parameters as ILlmParameters | undefined)?.context_length ?? 0;
			if (
				contextLength < contextLengthFilter[0] ||
				contextLength > contextLengthFilter[1]
			)
				return false;
			return Object.entries(capabilityFilters).every(([key, minValue]) =>
				meetsModelCapabilityMinimum(bit.parameters, key, minValue),
			);
		});

		// A live query already orders by relevance; only an explicitly picked
		// sort ("updated" is the untouched default) overrides it.
		if (searchTerm.trim() && sortBy === "updated") return models;

		const priceOrder = (bit: IBit) => {
			const value = priceValue(prices.get(bit.id) ?? { kind: "unlisted" });
			return value ?? Number.POSITIVE_INFINITY;
		};
		return [...models].sort((a, b) => {
			const aParams = a.parameters as ILlmParameters | undefined;
			const bParams = b.parameters as ILlmParameters | undefined;
			switch (sortBy) {
				case "name":
					return (a.meta?.en?.name || a.id).localeCompare(
						b.meta?.en?.name || b.id,
					);
				case "updated":
					return Date.parse(b.updated) - Date.parse(a.updated);
				case "rating":
					return compareRating(a, b);
				case "price":
					return priceOrder(a) - priceOrder(b);
				case "context":
					return (
						(bParams?.context_length ?? 0) - (aParams?.context_length ?? 0)
					);
				default:
					return compareModelCapability(aParams, bParams, sortBy);
			}
		});
	}, [
		searchedBits,
		searchTerm,
		showInProfileOnly,
		showDownloadedOnly,
		showEuHostedOnly,
		profileBitIds,
		installedBits,
		prices,
		tiers,
		runFilters,
		priceFilter,
		ratingFilter,
		inputFilters,
		providerFilter,
		contextLengthFilter,
		capabilityFilters,
		sortBy,
	]);

	const railDefs = useMemo<Rail[]>(
		() => [
			{
				id: "chat",
				tab: t("chatReasoning", "Chat & reasoning"),
				title: t("chatReasoning", "Chat & reasoning"),
				icon: MessageSquare,
				color: "var(--m-chat)",
				preview: 8,
				match: (b) => LLM_LIKE_TYPES.has(b.type),
			},
			{
				id: "image",
				tab: t("catalogTabImages", "Images"),
				title: t("imageGeneration", "Image generation"),
				icon: ImageIcon,
				color: "var(--m-image)",
				preview: 4,
				match: (b) => b.type === IBitTypes.ImageGeneration,
			},
			{
				id: "tts",
				tab: t("texttospeech", "Text-to-speech"),
				title: t("texttospeech", "Text-to-speech"),
				icon: AudioLines,
				color: "var(--m-speech)",
				preview: 4,
				match: (b) => b.type === IBitTypes.Tts,
			},
			{
				id: "stt",
				tab: t("speechtotext", "Speech-to-text"),
				title: t("speechtotext", "Speech-to-text"),
				icon: Mic,
				color: "var(--m-audio)",
				preview: 4,
				match: (b) => b.type === IBitTypes.Stt,
			},
			{
				id: "embed",
				tab: t("embeddings", "Embeddings"),
				title: t("embeddings", "Embeddings"),
				icon: FileSearchIcon,
				color: "var(--m-embed)",
				preview: 4,
				match: (b) =>
					b.type === IBitTypes.Embedding || b.type === IBitTypes.ImageEmbedding,
			},
			{
				id: "video",
				tab: t("catalogTabVideo", "Video"),
				title: t("videoGeneration", "Video generation"),
				icon: Video,
				color: "var(--m-video)",
				preview: 4,
				match: (b) => b.type === IBitTypes.VideoGeneration,
			},
			{
				id: "decisions",
				tab: t("catalogTabDecisions", "Decisions"),
				title: t("catalogTabDecisions", "Decisions"),
				icon: ListChecks,
				color: "var(--m-embed)",
				preview: 4,
				match: (b) => b.type === IBitTypes.SystemOne,
			},
			{
				id: "forecasting",
				tab: t("catalogTabForecasting", "Forecasting"),
				title: t("catalogTabForecasting", "Forecasting"),
				icon: ChartNoAxesCombined,
				color: "var(--m-embed)",
				preview: 4,
				match: (b) => b.type === IBitTypes.Forecasting,
			},
			{
				id: "other",
				tab: t("otherModels", "Other models"),
				title: t("otherModels", "Other models"),
				icon: Sparkles,
				color: "var(--m-video)",
				preview: 4,
				match: () => true,
			},
		],
		[t],
	);

	const railOf = useCallback(
		(bit: IBit) => railDefs.find((rail) => rail.match(bit))?.id ?? "other",
		[railDefs],
	);

	const rails = useMemo(() => {
		const catalogCounts = new Map<string, number>();
		for (const bit of catalogBits) {
			const id = railOf(bit);
			catalogCounts.set(id, (catalogCounts.get(id) ?? 0) + 1);
		}
		return railDefs
			.map((rail) => ({
				...rail,
				items: filteredModels.filter((bit) => railOf(bit) === rail.id),
			}))
			.filter((rail) => (catalogCounts.get(rail.id) ?? 0) > 0);
	}, [railDefs, railOf, catalogBits, filteredModels]);

	useEffect(() => {
		if (activeTab !== ALL && !rails.some((rail) => rail.id === activeTab))
			setActiveTab(ALL);
	}, [activeTab, rails]);

	const visibleRails = useMemo(
		() =>
			rails.filter(
				(rail) =>
					rail.items.length > 0 && (activeTab === ALL || rail.id === activeTab),
			),
		[rails, activeTab],
	);

	const profileCount = useMemo(
		() => catalogBits.filter(isMine).length,
		[catalogBits, isMine],
	);

	const resetFilters = useCallback(() => {
		setShowDownloadedOnly(false);
		setShowEuHostedOnly(false);
		setRunFilters(new Set());
		setPriceFilter("any");
		setRatingFilter("any");
		setInputFilters(new Set());
		setProviderFilter(ALL);
		setContextLengthFilter([0, maxContextLength]);
		setCapabilityFilters(NO_CAPABILITY_FILTERS);
	}, [maxContextLength]);

	const activeFilters = useMemo(() => {
		const chips: { id: string; label: string; clear: () => void }[] = [];
		const runLabels: Record<RunFilter, string> = {
			hosted: t("runHosted", "Hosted"),
			local: t("runOnDevice", "On this device"),
			ownKey: t("runOwnKey", "Your API key"),
		};
		for (const run of runFilters)
			chips.push({
				id: `run-${run}`,
				label: runLabels[run],
				clear: () =>
					setRunFilters((prev) => {
						const next = new Set(prev);
						next.delete(run);
						return next;
					}),
			});
		if (showDownloadedOnly)
			chips.push({
				id: "downloaded",
				label: t("downloaded", "Downloaded"),
				clear: () => setShowDownloadedOnly(false),
			});
		if (showEuHostedOnly)
			chips.push({
				id: "eu-hosted",
				label: t("runEuHosted", "EU hosted"),
				clear: () => setShowEuHostedOnly(false),
			});
		if (priceFilter !== "any")
			chips.push({
				id: "price",
				label:
					priceFilter === "free"
						? t("priceFreeOnly", "Free")
						: t("priceLowCost", "Low cost"),
				clear: () => setPriceFilter("any"),
			});
		if (ratingFilter !== "any")
			chips.push({
				id: "rating",
				label:
					ratingFilter === "top"
						? t("ratingTopOnly", "Top tier only")
						: t("ratingStrongAndUp", "Rated strong and up"),
				clear: () => setRatingFilter("any"),
			});
		for (const input of inputFilters)
			chips.push({
				id: `input-${input}`,
				label: t("acceptsModality", "Accepts {{modality}}", {
					modality: inputLabel(input, t),
				}),
				clear: () =>
					setInputFilters((prev) => {
						const next = new Set(prev);
						next.delete(input);
						return next;
					}),
			});
		if (providerFilter !== ALL)
			chips.push({
				id: "provider",
				label: providerFilter,
				clear: () => setProviderFilter(ALL),
			});
		if (contextLengthFilter[0] > 0 || contextLengthFilter[1] < maxContextLength)
			chips.push({
				id: "context",
				label: `${formatContextLength(contextLengthFilter[0])} – ${formatContextLength(contextLengthFilter[1])}`,
				clear: () => setContextLengthFilter([0, maxContextLength]),
			});
		if (Object.values(capabilityFilters).some((value) => value > 0))
			chips.push({
				id: "capabilities",
				label: t("capabilities", "Capabilities"),
				clear: () => setCapabilityFilters(NO_CAPABILITY_FILTERS),
			});
		return chips;
	}, [
		t,
		runFilters,
		showDownloadedOnly,
		showEuHostedOnly,
		priceFilter,
		ratingFilter,
		inputFilters,
		providerFilter,
		contextLengthFilter,
		maxContextLength,
		capabilityFilters,
	]);

	const pickTab = useCallback((id: string) => {
		setActiveTab(id);
		document
			.getElementById("model-catalog-top")
			?.scrollIntoView({ block: "start" });
	}, []);

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
			toast.success(t("customModelDeleted", "Custom model deleted"));
		} catch (error) {
			toast.error(
				t("failedToDeleteModelVal", "Failed to delete model: {{val}}", {
					val: error instanceof Error ? error.message : String(error),
				}),
			);
		}
	}, [deleteCustomTarget, backend.bitState, invalidate, t]);

	const renderCard = useCallback(
		(bit: IBit) => {
			const custom = customBitIds.has(bit.id);
			return (
				<ModelCard
					key={bit.id}
					bit={bit}
					variant={viewMode}
					isCustom={custom}
					priceTier={tiers.get(bit.id)}
					onClick={() => setSelectedModel(bit)}
					onEdit={custom ? openEditCustomModel : undefined}
					onDelete={custom ? setDeleteCustomTarget : undefined}
				/>
			);
		},
		[customBitIds, openEditCustomModel, viewMode, tiers],
	);

	const narrowed =
		!!searchTerm.trim() || activeFilters.length > 0 || showInProfileOnly;
	const shownCount = visibleRails.reduce(
		(count, rail) =>
			count +
			(activeTab === ALL
				? Math.min(rail.preview, rail.items.length)
				: rail.items.length),
		0,
	);

	return (
		// -m-4 cancels the settings shell padding so the sticky header spans the full
		// width; the rows below re-add their own gutters.
		<main className="-m-4 flex min-h-0 flex-1 flex-col overflow-y-auto">
			<span id="model-catalog-top" className="block h-0 scroll-mt-0" />
			{/* Opaque: WebKit disables backdrop-filter, so a blur-only header
			    would let the cards scroll straight through it */}
			<header className="sticky top-0 z-30 border-b border-border/60 bg-background">
				<div
					className={cn(
						"mx-auto flex w-full max-w-310 flex-col gap-3 pt-4",
						isMobile ? "px-4" : "px-4 sm:px-8",
					)}
				>
					<div className="flex flex-wrap items-center gap-2.5">
						<div className="mr-auto flex min-w-0 items-center gap-3">
							<span className="grid h-9 w-9 shrink-0 place-items-center rounded-[10px] bg-muted text-foreground">
								<Boxes className="h-4.5 w-4.5" />
							</span>
							<div className="min-w-0">
								<h1 className="m-0 text-lg font-semibold leading-tight tracking-tight">
									{t("models", "Models")}
								</h1>
								<p className="m-0 text-[12.5px] text-muted-foreground">
									{t("catalogModelCount", "{{count}} models", {
										count: catalogBits.length,
									})}
									{" · "}
									{t("catalogInProfileCount", "{{count}} in your profile", {
										count: profileCount,
									})}
								</p>
							</div>
						</div>
						<fieldset
							aria-label={t("whichModelsToShow", "Which models to show")}
							className="m-0 flex rounded-[9px] border bg-background p-0.75"
						>
							<SegmentButton
								active={!showInProfileOnly}
								onClick={() => setShowInProfileOnly(false)}
							>
								{t("allModels", "All models")}
							</SegmentButton>
							<SegmentButton
								active={showInProfileOnly}
								onClick={() => setShowInProfileOnly(true)}
							>
								<Sparkles className="h-3.5 w-3.5" />
								{t("inProfile", "In profile")}
								<span className="font-mono text-[11px] tabular-nums">
									{profileCount}
								</span>
							</SegmentButton>
						</fieldset>
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant="outline"
									size="icon"
									className="h-8.5 w-8.5"
									aria-label={t("refreshModels", "Refresh models")}
									disabled={foundBits.isFetching}
									onClick={() => void foundBits.refetch()}
								>
									<RefreshCw
										className={cn(
											"h-4 w-4",
											foundBits.isFetching && "animate-spin",
										)}
									/>
								</Button>
							</TooltipTrigger>
							<TooltipContent>
								{t("refreshModels", "Refresh models")}
							</TooltipContent>
						</Tooltip>
						<Button
							onClick={openAddCustomModel}
							className="h-8.5 gap-1.5 px-3 text-[13px]"
						>
							<Plus className="h-4 w-4" />
							<span className="hidden sm:inline">
								{t("addCustomModel", "Add custom model")}
							</span>
							<span className="sm:hidden">{t("add", "Add")}</span>
						</Button>
					</div>

					<div className="flex flex-wrap items-center gap-2">
						<div className="relative min-w-0 flex-[1_1_18rem]">
							<Search className="pointer-events-none absolute top-1/2 left-3 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
							<Input
								aria-label={t("searchModels", "Search models")}
								placeholder={t(
									"searchModelsProvidersCapabilities",
									"Search models, providers, capabilities…",
								)}
								value={searchTerm}
								onChange={(e) => setSearchTerm(e.target.value)}
								className="h-9 border-border/60 bg-muted/40 pr-9 pl-9 text-sm focus:bg-background"
							/>
							{searchTerm && (
								<button
									type="button"
									aria-label={t("clearSearch", "Clear search")}
									onClick={() => setSearchTerm("")}
									className="absolute top-1/2 right-2 grid h-6 w-6 -translate-y-1/2 place-items-center rounded text-muted-foreground transition-colors hover:text-foreground"
								>
									<X className="h-4 w-4" />
								</button>
							)}
						</div>
						<Select
							value={sortBy}
							onValueChange={(v) => setSortBy(v as SortOption)}
						>
							<SelectTrigger
								aria-label={t("sortModels", "Sort models")}
								className="h-9 w-auto gap-1.5 border-border/60 px-3 text-[13px]"
							>
								<span className="text-muted-foreground">
									{t("sortLabel", "Sort")}
								</span>
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="updated">{t("recent", "Recent")}</SelectItem>
								<SelectItem value="rating">
									{t("sortRating", "Rating")}
								</SelectItem>
								<SelectItem value="price">
									{t("sortLowestPrice", "Lowest price")}
								</SelectItem>
								<SelectItem value="name">{t("sortName", "Name")}</SelectItem>
								<SelectItem value="context">
									{t("context", "Context")}
								</SelectItem>
								<SelectItem value="speed">{t("speed", "Speed")}</SelectItem>
								<SelectItem value="reasoning">
									{t("reasoning", "Reasoning")}
								</SelectItem>
								<SelectItem value="coding">{t("coding", "Coding")}</SelectItem>
							</SelectContent>
						</Select>
						<Popover>
							<PopoverTrigger asChild>
								<Button
									variant="outline"
									className={cn(
										"h-9 gap-1.5 border-border/60 px-3 text-[13px]",
										activeFilters.length > 0 &&
											"border-primary/40 bg-primary/10 text-primary hover:bg-primary/15 hover:text-primary",
									)}
								>
									<SlidersHorizontal className="h-4 w-4" />
									{t("filters", "Filters")}
									{activeFilters.length > 0 && (
										<span className="grid h-4.5 min-w-4.5 place-items-center rounded-full bg-primary px-1 font-mono text-[10.5px] text-primary-foreground">
											{activeFilters.length}
										</span>
									)}
								</Button>
							</PopoverTrigger>
							<PopoverContent
								align="end"
								className="max-h-[min(70vh,600px)] w-[min(380px,calc(100vw-2rem))] overflow-y-auto p-4"
							>
								<FilterPanel
									webMode={webMode}
									runFilters={runFilters}
									onToggleRun={(run) =>
										setRunFilters((prev) => toggled(prev, run))
									}
									showDownloadedOnly={showDownloadedOnly}
									onToggleDownloaded={() => setShowDownloadedOnly((v) => !v)}
									showEuHostedOnly={showEuHostedOnly}
									onToggleEuHosted={() => setShowEuHostedOnly((v) => !v)}
									priceFilter={priceFilter}
									onPriceFilter={setPriceFilter}
									ratingFilter={ratingFilter}
									onRatingFilter={setRatingFilter}
									inputFilters={inputFilters}
									onToggleInput={(input) =>
										setInputFilters((prev) => toggled(prev, input))
									}
									providers={providers}
									providerFilter={providerFilter}
									onProviderFilter={setProviderFilter}
									contextLengthFilter={contextLengthFilter}
									maxContextLength={maxContextLength}
									onContextLengthFilter={setContextLengthFilter}
									capabilityFilters={capabilityFilters}
									onCapabilityFilter={(key, value) =>
										setCapabilityFilters((prev) => ({ ...prev, [key]: value }))
									}
									onReset={resetFilters}
									resultCount={filteredModels.length}
								/>
							</PopoverContent>
						</Popover>
						<fieldset
							aria-label={t("layout", "Layout")}
							className="m-0 flex rounded-[9px] border bg-background p-0.75"
						>
							<SegmentButton
								active={viewMode === "grid"}
								onClick={() => setViewMode("grid")}
								label={t("switchToGrid", "Switch to grid")}
							>
								<LayoutGrid className="h-4 w-4" />
							</SegmentButton>
							<SegmentButton
								active={viewMode === "list"}
								onClick={() => setViewMode("list")}
								label={t("switchToList", "Switch to list")}
							>
								<LayoutList className="h-4 w-4" />
							</SegmentButton>
						</fieldset>
					</div>

					<nav
						aria-label={t("modelTypes", "Model types")}
						className="-mx-2.5 flex overflow-x-auto [mask-image:linear-gradient(90deg,#000_calc(100%-28px),transparent)] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
					>
						<div className="flex gap-0.5 pr-7">
							<TypeTab
								active={activeTab === ALL}
								onClick={() => pickTab(ALL)}
								label={t("allTab", "All")}
								count={filteredModels.length}
							/>
							{rails.map((rail) => (
								<TypeTab
									key={rail.id}
									active={activeTab === rail.id}
									onClick={() => pickTab(rail.id)}
									label={rail.tab}
									count={rail.items.length}
								/>
							))}
						</div>
					</nav>
				</div>

				{activeFilters.length > 0 && (
					<div className="border-t border-border/60">
						<div
							className={cn(
								"mx-auto flex w-full max-w-310 flex-wrap items-center gap-1.5 py-2.5",
								isMobile ? "px-4" : "px-4 sm:px-8",
							)}
						>
							<span className="mr-0.5 text-xs text-muted-foreground">
								{t("filteredBy", "Filtered by")}
							</span>
							{activeFilters.map((chip) => (
								<button
									key={chip.id}
									type="button"
									onClick={chip.clear}
									aria-label={t(
										"removeFilterLabel",
										"Remove filter: {{label}}",
										{
											label: chip.label,
										},
									)}
									className="inline-flex h-6.5 items-center gap-1.5 rounded-full border border-primary/40 bg-primary/10 px-2.5 text-xs text-primary transition-colors hover:bg-primary/15"
								>
									{chip.label}
									<X className="h-3 w-3" />
								</button>
							))}
							<button
								type="button"
								onClick={resetFilters}
								className="ml-1 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground"
							>
								{t("clearAll", "Clear all")}
							</button>
						</div>
					</div>
				)}
			</header>

			<div
				className={cn(
					"mx-auto flex w-full max-w-310 flex-1 flex-col gap-9 pt-6 pb-16",
					isMobile ? "px-4" : "px-4 sm:px-8",
				)}
			>
				{foundBits.error && (
					<p
						role="alert"
						className="rounded-lg border border-destructive/30 p-3 text-sm text-destructive"
					>
						{t("catalogRefreshFailed", "Could not refresh the model catalog.")}{" "}
						{foundBits.error.message}
					</p>
				)}
				{foundBits.isLoading ? (
					<ModelCatalogSkeleton />
				) : visibleRails.length === 0 ? (
					<div className="flex flex-col items-center justify-center rounded-xl border border-dashed border-border py-24 text-center">
						<Search className="mb-3 h-6 w-6 text-muted-foreground/50" />
						<p className="text-sm font-medium">
							{searchTerm
								? t(
										"nothingMatchesSearchterm",
										"Nothing matches “{{searchTerm}}”",
										{ searchTerm },
									)
								: t(
										"noModelsMatchTheseFilters",
										"No models match these filters",
									)}
						</p>
						<p className="mt-1 text-xs text-muted-foreground">
							{t(
								"catalogEmptyHint",
								"Try another name, or clear the filters to see the whole catalog.",
							)}
						</p>
						{narrowed && (
							<Button
								variant="outline"
								size="sm"
								className="mt-4 h-8 text-xs"
								onClick={() => {
									setSearchTerm("");
									setShowInProfileOnly(false);
									resetFilters();
								}}
							>
								{t("clearSearchAndFilters", "Clear search and filters")}
							</Button>
						)}
					</div>
				) : (
					<>
						{visibleRails.map((rail) => (
							<ModelSection
								key={rail.id}
								rail={rail}
								items={
									activeTab === ALL
										? rail.items.slice(0, rail.preview)
										: rail.items
								}
								total={rail.items.length}
								owned={showInProfileOnly ? 0 : rail.items.filter(isMine).length}
								view={viewMode}
								onSeeAll={
									activeTab === ALL ? () => pickTab(rail.id) : undefined
								}
								renderCard={renderCard}
							/>
						))}
						<footer className="border-t border-border/60 pt-4 text-xs text-muted-foreground">
							{activeTab === ALL && shownCount < filteredModels.length
								? t(
										"catalogShowingPreview",
										"Showing the first models of each type. Pick a type or “See all” for the full list.",
									)
								: t("catalogShowingCount", "Showing {{count}} models.", {
										count: shownCount,
									})}
						</footer>
					</>
				)}
			</div>

			<ModelDetailDialog
				bit={selectedModel}
				priceTier={selectedModel ? tiers.get(selectedModel.id) : undefined}
				onEdit={
					selectedModel && customBitIds.has(selectedModel.id)
						? (bit) => {
								setSelectedModel(null);
								openEditCustomModel(bit);
							}
						: undefined
				}
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
						<AlertDialogTitle>
							{t("deleteCustomModel", "Delete custom model?")}
						</AlertDialogTitle>
						<AlertDialogDescription>
							{t(
								"valAndItsStoredCredentialsWillBeRemovedFlowsUsingItWillNoLongerResolveThisModel",
								'"{{val}}" and its stored credentials will be removed. Flows using it will no longer resolve this model.',
								{ val: deleteCustomTarget?.meta?.en?.name ?? "This model" },
							)}
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel>{t("cancel", "Cancel")}</AlertDialogCancel>
						<AlertDialogAction onClick={confirmDeleteCustomModel}>
							{t("delete", "Delete")}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</main>
	);
}

function toggled<T>(set: Set<T>, value: T): Set<T> {
	const next = new Set(set);
	if (next.has(value)) next.delete(value);
	else next.add(value);
	return next;
}

function inputLabel(
	input: InputFilter,
	t: ReturnType<typeof useTranslation>["t"],
): string {
	switch (input) {
		case "text":
			return t("settings:inputText", "text");
		case "image":
			return t("settings:inputImages", "images");
		case "audio":
			return t("settings:inputAudio", "audio");
		default:
			return t("settings:inputTimeSeries", "time series");
	}
}

function SegmentButton({
	active,
	onClick,
	label,
	children,
}: Readonly<{
	active: boolean;
	onClick: () => void;
	label?: string;
	children: React.ReactNode;
}>) {
	return (
		<button
			type="button"
			aria-pressed={active}
			aria-label={label}
			title={label}
			onClick={onClick}
			className={cn(
				"inline-flex h-6.5 items-center gap-1.5 whitespace-nowrap rounded-md px-2.5 text-[12.5px] font-medium transition-colors",
				active
					? "bg-muted text-foreground"
					: "text-muted-foreground hover:text-foreground",
			)}
		>
			{children}
		</button>
	);
}

function TypeTab({
	active,
	onClick,
	label,
	count,
}: Readonly<{
	active: boolean;
	onClick: () => void;
	label: string;
	count: number;
}>) {
	return (
		<button
			type="button"
			aria-pressed={active}
			onClick={onClick}
			className={cn(
				"relative inline-flex h-10.5 items-center gap-1.5 whitespace-nowrap px-2.5 text-[13px] font-medium transition-colors",
				active
					? "text-foreground after:absolute after:inset-x-2.5 after:-bottom-px after:h-0.5 after:rounded-full after:bg-primary after:content-['']"
					: "text-muted-foreground hover:text-foreground",
			)}
		>
			{label}
			<span
				className={cn(
					"font-mono text-[11px] tabular-nums",
					count === 0 ? "text-muted-foreground/50" : "text-muted-foreground",
				)}
			>
				{count}
			</span>
		</button>
	);
}

function FilterGroup({
	title,
	children,
}: Readonly<{ title: string; children: React.ReactNode }>) {
	return (
		<div className="space-y-2">
			<p className="text-[11px] font-semibold uppercase tracking-[0.07em] text-muted-foreground">
				{title}
			</p>
			<div className="flex flex-wrap gap-1.5">{children}</div>
		</div>
	);
}

function FilterChip({
	active,
	onClick,
	children,
}: Readonly<{
	active: boolean;
	onClick: () => void;
	children: React.ReactNode;
}>) {
	return (
		<button
			type="button"
			aria-pressed={active}
			onClick={onClick}
			className={cn(
				"inline-flex h-7.5 items-center gap-1.5 whitespace-nowrap rounded-full border px-3 text-[12.5px] transition-colors",
				active
					? "border-primary/40 bg-primary/10 text-primary"
					: "text-foreground/80 hover:border-foreground/30 hover:text-foreground",
			)}
		>
			{children}
		</button>
	);
}

function FilterPanel({
	webMode,
	runFilters,
	onToggleRun,
	showDownloadedOnly,
	onToggleDownloaded,
	showEuHostedOnly,
	onToggleEuHosted,
	priceFilter,
	onPriceFilter,
	ratingFilter,
	onRatingFilter,
	inputFilters,
	onToggleInput,
	providers,
	providerFilter,
	onProviderFilter,
	contextLengthFilter,
	maxContextLength,
	onContextLengthFilter,
	capabilityFilters,
	onCapabilityFilter,
	onReset,
	resultCount,
}: Readonly<{
	webMode: boolean;
	runFilters: Set<RunFilter>;
	onToggleRun: (run: RunFilter) => void;
	showDownloadedOnly: boolean;
	onToggleDownloaded: () => void;
	showEuHostedOnly: boolean;
	onToggleEuHosted: () => void;
	priceFilter: PriceFilter;
	onPriceFilter: (value: PriceFilter) => void;
	ratingFilter: RatingFilter;
	onRatingFilter: (value: RatingFilter) => void;
	inputFilters: Set<InputFilter>;
	onToggleInput: (input: InputFilter) => void;
	providers: string[];
	providerFilter: string;
	onProviderFilter: (value: string) => void;
	contextLengthFilter: [number, number];
	maxContextLength: number;
	onContextLengthFilter: (value: [number, number]) => void;
	capabilityFilters: Record<string, number>;
	onCapabilityFilter: (key: string, value: number) => void;
	onReset: () => void;
	resultCount: number;
}>) {
	const { t } = useTranslation("settings");
	const runs: { id: RunFilter; label: string; icon: LucideIcon }[] = [
		{ id: "hosted", label: t("runHosted", "Hosted"), icon: CloudIcon },
		{
			id: "local",
			label: t("runOnDevice", "On this device"),
			icon: LaptopMinimal,
		},
		{ id: "ownKey", label: t("runOwnKey", "Your API key"), icon: KeyRound },
	];
	const inputs: { id: InputFilter; icon: LucideIcon }[] = [
		{ id: "text", icon: Type },
		{ id: "image", icon: ImageIcon },
		{ id: "audio", icon: Mic },
		{ id: "series", icon: ChartNoAxesCombined },
	];
	const capabilityLabels: Record<string, string> = {
		reasoning: t("reasoning", "Reasoning"),
		coding: t("coding", "Coding"),
		speed: t("speed", "Speed"),
		cost: t("costEfficiency", "Cost efficiency"),
		creativity: t("creativity", "Creativity"),
		factuality: t("factuality", "Factuality"),
		function_calling: t("functionCalling", "Function calling"),
		multilinguality: t("multilingual", "Multilingual"),
	};

	return (
		<div className="space-y-5">
			<FilterGroup title={t("filterWhereItRuns", "Where it runs")}>
				{runs.map((run) => (
					<FilterChip
						key={run.id}
						active={runFilters.has(run.id)}
						onClick={() => onToggleRun(run.id)}
					>
						<run.icon className="h-3.5 w-3.5" />
						{run.label}
					</FilterChip>
				))}
				<FilterChip active={showEuHostedOnly} onClick={onToggleEuHosted}>
					<MapPin className="h-3.5 w-3.5" />
					{t("runEuHosted", "EU hosted")}
				</FilterChip>
				{!webMode && (
					<FilterChip active={showDownloadedOnly} onClick={onToggleDownloaded}>
						<PackageCheck className="h-3.5 w-3.5" />
						{t("downloaded", "Downloaded")}
					</FilterChip>
				)}
			</FilterGroup>

			<FilterGroup title={t("filterPrice", "Price")}>
				{(
					[
						["any", t("priceAny", "Any price")],
						["free", t("priceFreeOnly", "Free")],
						["low", t("priceLowCost", "Low cost")],
					] as const
				).map(([id, label]) => (
					<FilterChip
						key={id}
						active={priceFilter === id}
						onClick={() => onPriceFilter(id)}
					>
						{label}
					</FilterChip>
				))}
			</FilterGroup>

			<div className="space-y-2">
				<FilterGroup title={t("filterRating", "Rating")}>
					{(
						[
							["any", t("ratingAny", "Any rating"), null],
							["good", t("ratingStrongAndUp", "Rated strong and up"), 2],
							["top", t("ratingTopOnly", "Top tier only"), 1],
						] as const
					).map(([id, label, tier]) => (
						<FilterChip
							key={id}
							active={ratingFilter === id}
							onClick={() => onRatingFilter(id)}
						>
							{tier && (
								<span
									aria-hidden="true"
									className={cn("h-2 w-2 rounded-full", RATING_TONE[tier].fill)}
								/>
							)}
							{label}
						</FilterChip>
					))}
				</FilterGroup>
				<p className="text-[11.5px] leading-relaxed text-muted-foreground">
					{t(
						"ratingFilterHint",
						"Chat models are rated by the intelligence index. Every other type has an estimated 0–100 quality score within its type.",
					)}
				</p>
				<div className="flex flex-wrap gap-x-3 gap-y-1 text-[11.5px] text-muted-foreground">
					{(
						[
							[1, t("ratingLegendTop", "Top tier")],
							[2, t("ratingLegendStrong", "Strong")],
							[3, t("ratingLegendFair", "Fair")],
							[4, t("ratingLegendBasic", "Basic")],
						] as const
					).map(([tier, label]) => (
						<span key={tier} className="inline-flex items-center gap-1.5">
							<span
								aria-hidden="true"
								className={cn("h-2 w-2 rounded-full", RATING_TONE[tier].fill)}
							/>
							{label}
						</span>
					))}
				</div>
			</div>

			<FilterGroup title={t("accepts", "Accepts")}>
				{inputs.map((input) => (
					<FilterChip
						key={input.id}
						active={inputFilters.has(input.id)}
						onClick={() => onToggleInput(input.id)}
					>
						<input.icon className="h-3.5 w-3.5" />
						{inputLabel(input.id, t)}
					</FilterChip>
				))}
			</FilterGroup>

			{providers.length > 0 && (
				<div className="space-y-2">
					<p className="text-[11px] font-semibold uppercase tracking-[0.07em] text-muted-foreground">
						{t("provider", "Provider")}
					</p>
					<Select value={providerFilter} onValueChange={onProviderFilter}>
						<SelectTrigger className="h-8 text-xs">
							<SelectValue placeholder={t("allProviders", "All providers")} />
						</SelectTrigger>
						<SelectContent>
							<SelectItem value={ALL}>
								{t("allProviders", "All providers")}
							</SelectItem>
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
					<p className="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-[0.07em] text-muted-foreground">
						<Cpu className="h-3 w-3" />
						{t("context", "Context")}
					</p>
					<span className="text-[11px] text-muted-foreground">
						{formatContextLength(contextLengthFilter[0])} –{" "}
						{formatContextLength(contextLengthFilter[1])}
					</span>
				</div>
				<Slider
					value={contextLengthFilter}
					onValueChange={(v) => onContextLengthFilter(v as [number, number])}
					min={0}
					max={maxContextLength}
					step={1000}
				/>
			</div>

			<div className="space-y-3">
				<p className="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-[0.07em] text-muted-foreground">
					<Brain className="h-3 w-3" />
					{t("capabilities", "Capabilities")}
				</p>
				{CAPABILITIES.map((capability) => {
					const value = capabilityFilters[capability.key] ?? 0;
					return (
						<div key={capability.key} className="space-y-1.5">
							<div className="flex items-center justify-between text-xs">
								<span
									className={cn("flex items-center gap-1.5", capability.color)}
								>
									<capability.icon className="h-3 w-3" />
									{capabilityLabels[capability.key]}
								</span>
								<span className="text-muted-foreground">
									{value > 0
										? `≥${Math.round(value * 100)}%`
										: t("anyValue", "Any")}
								</span>
							</div>
							<Slider
								value={[value]}
								onValueChange={([v]) => onCapabilityFilter(capability.key, v)}
								min={0}
								max={1}
								step={0.1}
							/>
						</div>
					);
				})}
			</div>

			<div className="flex items-center justify-between border-t pt-3">
				<button
					type="button"
					onClick={onReset}
					className="text-xs font-medium text-muted-foreground transition-colors hover:text-foreground"
				>
					{t("resetFilters", "Reset")}
				</button>
				<span className="text-xs text-muted-foreground">
					{t("catalogMatchingCount", "{{count}} matching models", {
						count: resultCount,
					})}
				</span>
			</div>
		</div>
	);
}

/** One model type: a preview row on "All", the full grid on its own tab. */
function ModelSection({
	rail,
	items,
	total,
	owned,
	view,
	onSeeAll,
	renderCard,
}: Readonly<{
	rail: Rail;
	items: IBit[];
	total: number;
	owned: number;
	view: ViewMode;
	onSeeAll?: () => void;
	renderCard: (bit: IBit) => React.ReactNode;
}>) {
	const { t } = useTranslation("settings");
	const Icon = rail.icon;
	const headingId = `rail-${rail.id}-heading`;
	return (
		<section aria-labelledby={headingId} className="flex flex-col gap-3">
			<div className="flex min-h-7 items-center gap-2.5">
				<span
					style={{ "--rc": rail.color } as React.CSSProperties}
					className="grid h-6.5 w-6.5 shrink-0 place-items-center rounded-[7px] bg-[color-mix(in_srgb,var(--rc)_16%,transparent)] text-(--rc)"
				>
					<Icon className="h-3.5 w-3.5" />
				</span>
				<h2 id={headingId} className="m-0 text-[15px] font-semibold">
					{rail.title}
				</h2>
				<span className="font-mono text-xs tabular-nums text-muted-foreground">
					{total}
				</span>
				{owned > 0 && (
					<span className="text-xs text-muted-foreground">
						·{" "}
						{t("catalogInProfileCount", "{{count}} in your profile", {
							count: owned,
						})}
					</span>
				)}
				{onSeeAll && total > items.length && (
					<button
						type="button"
						onClick={onSeeAll}
						className="ml-auto inline-flex items-center gap-1 text-[12.5px] font-medium text-muted-foreground transition-colors hover:text-foreground"
					>
						{t("seeAllCount", "See all {{count}}", { count: total })}
						<ChevronRight className="h-3.5 w-3.5" />
					</button>
				)}
			</div>
			<ul
				className={cn(
					"m-0 grid list-none p-0",
					view === "list"
						? "grid-cols-1 gap-1.5"
						: "grid-cols-[repeat(auto-fill,minmax(272px,1fr))] gap-3",
				)}
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

function ModelCatalogSkeleton() {
	return (
		<div className="flex flex-col gap-9">
			{[0, 1].map((section) => (
				<div key={`skel-section-${section}`} className="flex flex-col gap-3">
					<Skeleton className="h-5 w-48" />
					<div className="grid grid-cols-[repeat(auto-fill,minmax(272px,1fr))] gap-3">
						{Array.from({ length: 4 }).map((_, i) => (
							<Skeleton
								key={`skel-model-${section}-${i.toString()}`}
								className="h-52 rounded-xl"
							/>
						))}
					</div>
				</div>
			))}
		</div>
	);
}
