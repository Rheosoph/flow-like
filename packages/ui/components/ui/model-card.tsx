"use client";
import type { UseQueryResult } from "@tanstack/react-query";
import {
	ArrowRightIcon,
	BrainIcon,
	CameraIcon,
	CheckIcon,
	ClockIcon,
	DownloadCloudIcon,
	ExternalLinkIcon,
	FileSearch,
	ImageIcon,
	MoreVerticalIcon,
	PlusIcon,
	ScanEyeIcon,
	SparklesIcon,
	TrashIcon,
	TypeIcon,
	XIcon,
} from "lucide-react";
import type { JSX, ReactNode } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useHub } from "../../hooks/use-hub";
import { useInvoke } from "../../hooks/use-invoke";
import { type IBit, IBitTypes } from "../../lib/schema/bit/bit";
import type { IEmbeddingModelParameters } from "../../lib/schema/bit/bit/embedding-model-parameters";
import type { ILlmParameters } from "../../lib/schema/bit/bit/llm-parameters";
import { humanFileSize } from "../../lib/utils";
import { useBackend } from "../../state/backend-state";
import { useDownloadManager } from "../../state/download-manager";
import type { ISettingsProfile } from "../../types";
import { Avatar, AvatarFallback, AvatarImage } from "./avatar";
import { Badge } from "./badge";
import { Button } from "./button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "./dropdown-menu";
import { IntelligenceIndexBadge } from "./model-benchmarks";
import type { IModelEvaluation } from "./model-benchmarks";
import { Progress } from "./progress";

export type ModelCardVariant = "grid" | "list";

export interface ModelCardProps {
	bit: IBit;
	variant?: ModelCardVariant;
	onClick?: (bit: IBit) => void;
}

export function ModelCard({
	bit,
	variant = "grid",
	onClick,
}: Readonly<ModelCardProps>) {
	const backend = useBackend();
	const { hub } = useHub();
	const download = useDownloadManager((s) => s.download);
	const onProgress = useDownloadManager((s) => s.onProgress);
	const isQueued = useDownloadManager((s) => s.isQueued);
	const getLatestPct = useDownloadManager((s) => s.getLatestPct);

	const [progress, setProgress] = useState<number | undefined>();
	const isQueuedState = useMemo(() => progress === 0, [progress]);

	const mountedRef = useRef(true);
	const lastPctRef = useRef(0);
	const lastUpdateRef = useRef(0);
	const unsubscribeRef = useRef<(() => void) | null>(null);

	useEffect(() => {
		mountedRef.current = true;
		const initial = getLatestPct(bit.hash);
		if (typeof initial === "number") {
			setProgress(initial);
			lastPctRef.current = initial;
		} else if (isQueued(bit.hash)) {
			setProgress(0);
			lastPctRef.current = 0;
		}

		unsubscribeRef.current = onProgress(bit.hash, (dl) => {
			const rawProgress = dl.progress();
			const pct = Math.round(rawProgress * 100);
			const now = Date.now();
			const changed = Math.abs(pct - lastPctRef.current) >= 1;
			const due = now - lastUpdateRef.current >= 250;
			const completed =
				rawProgress >= 0.999 || dl.total().downloaded >= dl.total().max;
			if (!mountedRef.current) return;
			if (completed) {
				setProgress(undefined);
				lastPctRef.current = 0;
				lastUpdateRef.current = now;
				return;
			}
			if (changed || due) {
				setProgress(pct);
				lastPctRef.current = pct;
				lastUpdateRef.current = now;
			}
		});

		return () => {
			mountedRef.current = false;
			if (unsubscribeRef.current) {
				unsubscribeRef.current();
				unsubscribeRef.current = null;
			}
			lastPctRef.current = 0;
			lastUpdateRef.current = 0;
			setProgress(undefined);
		};
	}, [bit.hash, getLatestPct, isQueued, onProgress]);

	const isInstalled: UseQueryResult<boolean> = useInvoke(
		backend.bitState.isBitInstalled,
		backend.bitState,
		[bit],
	);
	const bitSize: UseQueryResult<number> = useInvoke(
		backend.bitState.getBitSize,
		backend.bitState,
		[bit],
	);
	const currentProfile: UseQueryResult<ISettingsProfile> = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);

	const userInfo = useInvoke(backend.userState.getInfo, backend.userState, []);

	const isVirtualBit = useMemo(
		() => !bit.download_link || (bitSize.data === 0 && bitSize.isSuccess),
		[bit.download_link, bitSize.data, bitSize.isSuccess],
	);

	const tierInfo = useMemo(() => {
		const params = bit.parameters as {
			provider?: { params?: { tier?: string } };
		};
		const modelTier = params?.provider?.params?.tier;
		if (!modelTier || !hub?.tiers) {
			return { isRestricted: false, requiredTier: null };
		}
		const userTierKey = (userInfo.data?.tier ?? "FREE").toUpperCase();
		const userTierConfig = hub.tiers[userTierKey];
		if (!userTierConfig) {
			return { isRestricted: true, requiredTier: modelTier };
		}
		const allowedModelTiers = userTierConfig.llm_tiers ?? [];
		const isRestricted = !allowedModelTiers.includes(modelTier);
		return { isRestricted, requiredTier: isRestricted ? modelTier : null };
	}, [bit.parameters, hub?.tiers, userInfo.data?.tier]);

	const downloadBit = useCallback(
		async (b: IBit) => {
			if (!b.download_link || isVirtualBit) {
				await isInstalled.refetch();
				return;
			}
			setProgress(0);
			try {
				await download(b);
				await isInstalled.refetch();
			} finally {
				if (mountedRef.current) {
					setProgress(undefined);
					lastPctRef.current = 0;
					lastUpdateRef.current = 0;
				}
			}
		},
		[download, isInstalled, isVirtualBit],
	);

	const refetchIsInstalled = isInstalled.refetch;
	const toggleDownload = useCallback(async () => {
		if (isInstalled.data) {
			await backend.bitState.deleteBit(bit);
			await refetchIsInstalled();
			return;
		}
		await downloadBit(bit);
	}, [
		isInstalled.data,
		backend.bitState,
		bit,
		downloadBit,
		refetchIsInstalled,
	]);

	const refetchCurrentProfile = currentProfile.refetch;
	const toggleProfile = useCallback(async () => {
		const profile = currentProfile.data;
		if (!profile) return;
		const bitIndex = profile.hub_profile.bits.findIndex(
			(id) => id.split(":").pop() === bit.id,
		);
		if (bitIndex === -1) {
			await downloadBit(bit);
			await backend.bitState.addBit(bit, profile);
		} else {
			await backend.bitState.removeBit(bit, profile);
		}
		await refetchCurrentProfile();
	}, [
		currentProfile.data,
		bit,
		downloadBit,
		backend.bitState,
		refetchCurrentProfile,
	]);

	const openRepository = useCallback(() => {
		if (bit.repository) window.open(bit.repository, "_blank");
	}, [bit.repository]);

	if (bit.meta.en === undefined) return null;

	const isInProfile =
		(currentProfile.data?.hub_profile.bits || []).findIndex(
			(id) => id.split(":").pop() === bit.id,
		) > -1;

	const modality = getModelModality(bit);
	const params = bit.parameters as ILlmParameters | IEmbeddingModelParameters;
	const contextLength = (params as ILlmParameters)?.context_length;
	const isHosted = bitSize.data === 0 || isVirtualBit;
	const canRunRemotely = supportsRemoteEmbeddingExecution(bit);
	const isEmbeddingModel = isEmbeddingBit(bit);

	if (variant === "list") {
		return (
			<ModelCardListVariant
				bit={bit}
				modality={modality}
				contextLength={contextLength}
				canRunRemotely={canRunRemotely}
				isEmbeddingModel={isEmbeddingModel}
				isInstalled={!!isInstalled.data}
				isInProfile={isInProfile}
				isHosted={isHosted}
				isRestricted={tierInfo.isRestricted}
				requiredTier={tierInfo.requiredTier}
				bitSize={bitSize.data ?? 0}
				progress={progress}
				isQueuedState={isQueuedState}
				isVirtualBit={isVirtualBit}
				onCardClick={() => onClick?.(bit)}
				onToggleDownload={toggleDownload}
				onToggleProfile={toggleProfile}
				onOpenRepository={openRepository}
			/>
		);
	}

	return (
		<ModelCardGridVariant
			bit={bit}
			modality={modality}
			contextLength={contextLength}
			canRunRemotely={canRunRemotely}
			isEmbeddingModel={isEmbeddingModel}
			isInstalled={!!isInstalled.data}
			isInProfile={isInProfile}
			isHosted={isHosted}
			isRestricted={tierInfo.isRestricted}
			requiredTier={tierInfo.requiredTier}
			bitSize={bitSize.data ?? 0}
			progress={progress}
			isQueuedState={isQueuedState}
			isVirtualBit={isVirtualBit}
			onCardClick={() => onClick?.(bit)}
			onToggleDownload={toggleDownload}
			onToggleProfile={toggleProfile}
			onOpenRepository={openRepository}
		/>
	);
}

interface ModelCardVariantProps {
	bit: IBit;
	modality: string;
	contextLength?: number;
	canRunRemotely: boolean;
	isEmbeddingModel: boolean;
	isInstalled: boolean;
	isInProfile: boolean;
	isHosted: boolean;
	isRestricted: boolean;
	requiredTier: string | null;
	bitSize: number;
	progress?: number;
	isQueuedState: boolean;
	isVirtualBit: boolean;
	onCardClick: () => void;
	onToggleDownload: () => void;
	onToggleProfile: () => void;
	onOpenRepository: () => void;
}

function ModelCardGridVariant({
	bit,
	modality,
	contextLength,
	canRunRemotely,
	isEmbeddingModel,
	isInstalled,
	isInProfile,
	isHosted,
	isRestricted,
	requiredTier,
	bitSize,
	progress,
	isQueuedState,
	isVirtualBit,
	onCardClick,
	onToggleDownload,
	onToggleProfile,
	onOpenRepository,
}: Readonly<ModelCardVariantProps>) {
	const meta = bit.meta.en;
	if (!meta) return null;

	return (
		<div
			onClick={onCardClick}
			onKeyDown={(e) => e.key === "Enter" && onCardClick()}
			className="group relative flex flex-col rounded-lg border bg-card p-3 cursor-pointer transition-all hover:bg-accent/50 hover:border-primary/30 h-35"
		>
			{/* Download Overlay */}
			{progress !== undefined && !isVirtualBit && (
				<div className="absolute inset-0 bg-background/90 backdrop-blur-sm z-30 flex items-center justify-center rounded-lg">
					{isQueuedState ? (
						<div className="flex items-center gap-2">
							<ClockIcon className="h-4 w-4 text-primary animate-pulse" />
							<span className="text-sm text-muted-foreground">Queued</span>
						</div>
					) : (
						<div className="flex items-center gap-3">
							<Progress value={progress} className="w-24 h-1.5" />
							<span className="text-sm text-muted-foreground tabular-nums">
								{progress}%
							</span>
						</div>
					)}
				</div>
			)}

			{/* Header: Icon + Name + Menu */}
			<div className="flex items-start gap-2.5">
				<Avatar className="h-9 w-9 shrink-0 border border-border/50">
					<AvatarImage src={meta.icon ?? "/app-logo.webp"} />
					<AvatarFallback className="bg-muted text-xs">
						<ModelTypeIcon type={bit.type} className="h-4 w-4" />
					</AvatarFallback>
				</Avatar>
				<div className="flex-1 min-w-0">
					<div className="flex items-center gap-1.5">
						<span className="font-medium text-sm truncate">{meta.name}</span>
						{isInProfile && (
							<SparklesIcon className="h-3.5 w-3.5 text-primary shrink-0" />
						)}
					</div>
					<ModalityIcons type={bit.type} />
				</div>
				<ModelCardDropdown
					isInstalled={isInstalled}
					isInProfile={isInProfile}
					hasRepository={!!bit.repository}
					bitSize={bitSize}
					onToggleDownload={onToggleDownload}
					onToggleProfile={onToggleProfile}
					onOpenRepository={onOpenRepository}
				/>
			</div>

			{/* Description */}
			<p className="text-xs text-muted-foreground line-clamp-2 mt-2 h-8 overflow-hidden">
				{meta.description}
			</p>

			{/* Footer: Badges */}
			<div className="flex items-center gap-1.5 flex-wrap mt-auto pt-2">
				<ModelStatusBadge
					isInstalled={isInstalled}
					isHosted={isHosted}
					bitSize={bitSize}
				/>
				<IntelligenceIndexBadge
					evaluation={bit.model_evaluation as IModelEvaluation | undefined}
				/>
				{contextLength && (
					<Badge variant="outline" className="text-[10px] px-1.5 py-0 h-5">
						{formatContextLength(contextLength)}
					</Badge>
				)}
				{canRunRemotely && (
					<Badge
						variant="outline"
						className="text-[10px] px-1.5 py-0 h-5 bg-cyan-500/10 text-cyan-700 border-cyan-500/30"
					>
						Remote
					</Badge>
				)}
				{isEmbeddingModel && !canRunRemotely && (
					<Badge
						variant="outline"
						className="text-[10px] px-1.5 py-0 h-5 bg-zinc-500/10 text-zinc-600 border-zinc-500/30"
					>
						Local only
					</Badge>
				)}
				{isRestricted && requiredTier && (
					<Badge
						variant="outline"
						className="text-[10px] px-1.5 py-0 h-5 bg-amber-500/10 text-amber-600 border-amber-500/30"
					>
						{requiredTier}
					</Badge>
				)}
			</div>
		</div>
	);
}

function ModelCardListVariant({
	bit,
	modality,
	contextLength,
	canRunRemotely,
	isEmbeddingModel,
	isInstalled,
	isInProfile,
	isHosted,
	isRestricted,
	requiredTier,
	bitSize,
	progress,
	isQueuedState,
	isVirtualBit,
	onCardClick,
	onToggleDownload,
	onToggleProfile,
	onOpenRepository,
}: Readonly<ModelCardVariantProps>) {
	const meta = bit.meta.en;
	if (!meta) return null;

	return (
		<div
			onClick={onCardClick}
			onKeyDown={(e) => e.key === "Enter" && onCardClick()}
			className="group relative flex items-center gap-3 rounded-lg border bg-card px-3 py-2 cursor-pointer transition-all hover:bg-accent/50 hover:border-primary/30"
		>
			{/* Download Overlay */}
			{progress !== undefined && !isVirtualBit && (
				<div className="absolute inset-0 bg-background/90 backdrop-blur-sm z-30 flex items-center justify-center rounded-lg">
					{isQueuedState ? (
						<div className="flex items-center gap-2">
							<ClockIcon className="h-4 w-4 text-primary animate-pulse" />
							<span className="text-sm text-muted-foreground">Queued</span>
						</div>
					) : (
						<div className="flex items-center gap-3">
							<Progress value={progress} className="w-24 h-1.5" />
							<span className="text-sm text-muted-foreground tabular-nums">
								{progress}%
							</span>
						</div>
					)}
				</div>
			)}

			{/* Icon */}
			<Avatar className="h-8 w-8 shrink-0 border border-border/50">
				<AvatarImage src={meta.icon ?? "/app-logo.webp"} />
				<AvatarFallback className="bg-muted text-xs">
					<ModelTypeIcon type={bit.type} className="h-4 w-4" />
				</AvatarFallback>
			</Avatar>

			{/* Name + Modality */}
			<div className="flex-1 min-w-0">
				<div className="flex items-center gap-1.5">
					<span className="font-medium text-sm truncate">{meta.name}</span>
					{isInProfile && (
						<SparklesIcon className="h-3.5 w-3.5 text-primary shrink-0" />
					)}
				</div>
				<ModalityIcons type={bit.type} />
			</div>

			{/* Badges */}
			<div className="flex items-center gap-1.5 shrink-0">
				<ModelStatusBadge
					isInstalled={isInstalled}
					isHosted={isHosted}
					bitSize={bitSize}
				/>
				<IntelligenceIndexBadge
					evaluation={bit.model_evaluation as IModelEvaluation | undefined}
				/>
				{contextLength && (
					<Badge variant="outline" className="text-[10px] px-1.5 py-0 h-5">
						{formatContextLength(contextLength)}
					</Badge>
				)}
				{canRunRemotely && (
					<Badge
						variant="outline"
						className="text-[10px] px-1.5 py-0 h-5 bg-cyan-500/10 text-cyan-700 border-cyan-500/30"
					>
						Remote
					</Badge>
				)}
				{isEmbeddingModel && !canRunRemotely && (
					<Badge
						variant="outline"
						className="text-[10px] px-1.5 py-0 h-5 bg-zinc-500/10 text-zinc-600 border-zinc-500/30"
					>
						Local only
					</Badge>
				)}
				{isRestricted && requiredTier && (
					<Badge
						variant="outline"
						className="text-[10px] px-1.5 py-0 h-5 bg-amber-500/10 text-amber-600 border-amber-500/30"
					>
						{requiredTier}
					</Badge>
				)}
			</div>

			{/* Menu */}
			<ModelCardDropdown
				isInstalled={isInstalled}
				isInProfile={isInProfile}
				hasRepository={!!bit.repository}
				bitSize={bitSize}
				onToggleDownload={onToggleDownload}
				onToggleProfile={onToggleProfile}
				onOpenRepository={onOpenRepository}
			/>
		</div>
	);
}

interface ModelCardDropdownProps {
	isInstalled: boolean;
	isInProfile: boolean;
	hasRepository: boolean;
	bitSize: number;
	onToggleDownload: () => void;
	onToggleProfile: () => void;
	onOpenRepository: () => void;
}

function ModelCardDropdown({
	isInstalled,
	isInProfile,
	hasRepository,
	bitSize,
	onToggleDownload,
	onToggleProfile,
	onOpenRepository,
}: Readonly<ModelCardDropdownProps>) {
	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<Button
					size="sm"
					variant="ghost"
					className="h-7 w-7 p-0 opacity-0 group-hover:opacity-100 transition-opacity shrink-0"
					onClick={(e) => e.stopPropagation()}
				>
					<MoreVerticalIcon className="h-4 w-4" />
				</Button>
			</DropdownMenuTrigger>
			<DropdownMenuContent align="end" className="w-44">
				<DropdownMenuItem
					onClick={(e) => {
						e.stopPropagation();
						onToggleDownload();
					}}
				>
					{isInstalled ? (
						<>
							<TrashIcon className="h-4 w-4 mr-2" />
							Remove
						</>
					) : (
						<>
							<DownloadCloudIcon className="h-4 w-4 mr-2" />
							Download ({humanFileSize(bitSize)})
						</>
					)}
				</DropdownMenuItem>
				<DropdownMenuItem
					onClick={(e) => {
						e.stopPropagation();
						onToggleProfile();
					}}
				>
					{isInProfile ? (
						<>
							<XIcon className="h-4 w-4 mr-2" />
							Remove from Profile
						</>
					) : (
						<>
							<PlusIcon className="h-4 w-4 mr-2" />
							Add to Profile
						</>
					)}
				</DropdownMenuItem>
				{hasRepository && (
					<>
						<DropdownMenuSeparator />
						<DropdownMenuItem
							onClick={(e) => {
								e.stopPropagation();
								onOpenRepository();
							}}
						>
							<ExternalLinkIcon className="h-4 w-4 mr-2" />
							View Repository
						</DropdownMenuItem>
					</>
				)}
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

function ModelStatusBadge({
	isInstalled,
	isHosted,
	bitSize,
}: Readonly<{ isInstalled: boolean; isHosted: boolean; bitSize: number }>) {
	if (isHosted) {
		return (
			<Badge
				variant="outline"
				className="text-[10px] px-1.5 py-0 h-5 bg-sky-500/10 text-sky-600 border-sky-500/30"
			>
				Hosted
			</Badge>
		);
	}
	if (isInstalled) {
		return (
			<Badge
				variant="outline"
				className="text-[10px] px-1.5 py-0 h-5 bg-emerald-500/10 text-emerald-600 border-emerald-500/30"
			>
				<CheckIcon className="h-3 w-3 mr-0.5" />
				{humanFileSize(bitSize)}
			</Badge>
		);
	}
	return (
		<Badge variant="outline" className="text-[10px] px-1.5 py-0 h-5">
			{humanFileSize(bitSize)}
		</Badge>
	);
}

export function ModelTypeIcon({
	type,
	className = "",
}: Readonly<{ type: IBitTypes; className?: string }>): JSX.Element {
	const cn = `h-4 w-4 ${className}`;
	switch (type) {
		case IBitTypes.Llm:
			return <BrainIcon className={cn} />;
		case IBitTypes.Vlm:
			return <CameraIcon className={cn} />;
		case IBitTypes.Embedding:
			return <FileSearch className={cn} />;
		case IBitTypes.ImageEmbedding:
			return <ScanEyeIcon className={cn} />;
		default:
			return <BrainIcon className={cn} />;
	}
}

export function ModalityIcons({
	type,
}: Readonly<{ type: IBitTypes }>): JSX.Element {
	const iconClass = "h-3 w-3";
	const arrowClass = "h-2.5 w-2.5 text-foreground";

	switch (type) {
		case IBitTypes.Llm:
			return (
				<div className="flex items-center gap-1 text-muted-foreground">
					<TypeIcon className={`${iconClass} text-blue-500`} />
					<ArrowRightIcon className={arrowClass} />
					<TypeIcon className={`${iconClass} text-emerald-500`} />
				</div>
			);
		case IBitTypes.Vlm:
			return (
				<div className="flex items-center gap-1 text-muted-foreground">
					<TypeIcon className={`${iconClass} text-blue-500`} />
					<ImageIcon className={`${iconClass} text-purple-500`} />
					<ArrowRightIcon className={arrowClass} />
					<TypeIcon className={`${iconClass} text-emerald-500`} />
				</div>
			);
		case IBitTypes.Embedding:
			return (
				<div className="flex items-center gap-1 text-muted-foreground">
					<TypeIcon className={`${iconClass} text-blue-500`} />
					<ArrowRightIcon className={arrowClass} />
					<FileSearch className={`${iconClass} text-amber-500`} />
				</div>
			);
		case IBitTypes.ImageEmbedding:
			return (
				<div className="flex items-center gap-1 text-muted-foreground">
					<ImageIcon className={`${iconClass} text-purple-500`} />
					<ArrowRightIcon className={arrowClass} />
					<FileSearch className={`${iconClass} text-amber-500`} />
				</div>
			);
		default:
			return (
				<div className="flex items-center gap-1 text-muted-foreground">
					<TypeIcon className={`${iconClass} text-muted-foreground`} />
					<ArrowRightIcon className={arrowClass} />
					<TypeIcon className={`${iconClass} text-muted-foreground`} />
				</div>
			);
	}
}

export function getModelModality(bit: IBit): string {
	switch (bit.type) {
		case IBitTypes.Llm:
			return "Text → Text";
		case IBitTypes.Vlm:
			return "Image → Text";
		case IBitTypes.Embedding:
			return "Text → Embedding";
		case IBitTypes.ImageEmbedding:
			return "Image → Embedding";
		default:
			return "Unknown";
	}
}

interface IRemoteExecutionConfig {
	endpoint?: string | null;
	implementation?: string | null;
	model_id?: string | null;
}

interface IEmbeddingProviderParamsWithRemote {
	remote?: IRemoteExecutionConfig;
}

type IEmbeddingModelParametersWithRemote =
	Partial<IEmbeddingModelParameters> & {
		remote?: IRemoteExecutionConfig;
		provider?:
			| (Partial<IEmbeddingModelParameters["provider"]> & {
					params?: IEmbeddingProviderParamsWithRemote;
			  })
			| undefined;
	};

export function isEmbeddingBit(bit: IBit): boolean {
	return (
		bit.type === IBitTypes.Embedding || bit.type === IBitTypes.ImageEmbedding
	);
}

export function supportsRemoteEmbeddingExecution(bit: IBit): boolean {
	if (!isEmbeddingBit(bit)) return false;
	const params = bit.parameters as
		| IEmbeddingModelParametersWithRemote
		| undefined;
	const remote = params?.remote ?? params?.provider?.params?.remote;
	const remoteModelId = remote?.model_id ?? params?.provider?.model_id;
	if (remote?.implementation && remoteModelId) return true;

	const providerName = params?.provider?.provider_name?.toLowerCase();
	if (
		providerName === "premium" ||
		providerName === "internal" ||
		providerName === "hosted" ||
		providerName?.startsWith("hosted:")
	) {
		return Boolean(params?.provider?.model_id);
	}

	return false;
}

export function formatContextLength(length: number): string {
	if (length >= 1_000_000) return `${(length / 1_000_000).toFixed(1)}M ctx`;
	if (length >= 1000) return `${Math.round(length / 1000)}K ctx`;
	return `${length} ctx`;
}

export function getCapabilityIcon(key: string): {
	icon: ReactNode;
	label: string;
	color: string;
} {
	const icons: Record<
		string,
		{ icon: ReactNode; label: string; color: string }
	> = {
		coding: { icon: "💻", label: "Coding", color: "text-blue-500" },
		cost: { icon: "💰", label: "Cost Efficiency", color: "text-green-500" },
		creativity: { icon: "🎨", label: "Creativity", color: "text-purple-500" },
		factuality: { icon: "📚", label: "Factuality", color: "text-amber-500" },
		function_calling: {
			icon: "🔧",
			label: "Function Calling",
			color: "text-cyan-500",
		},
		multilinguality: {
			icon: "🌍",
			label: "Multilingual",
			color: "text-teal-500",
		},
		openness: { icon: "🔓", label: "Openness", color: "text-orange-500" },
		reasoning: { icon: "🧠", label: "Reasoning", color: "text-pink-500" },
		safety: { icon: "🛡️", label: "Safety", color: "text-red-500" },
		speed: { icon: "⚡", label: "Speed", color: "text-yellow-500" },
	};
	return icons[key] || { icon: "❓", label: key, color: "text-gray-500" };
}
