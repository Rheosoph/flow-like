"use client";
import { i18n as i18next, useTranslation } from "@flow-like/locales";
import type { UseQueryResult } from "@tanstack/react-query";
import {
	ArrowRightIcon,
	AudioLinesIcon,
	BrainIcon,
	CameraIcon,
	CheckIcon,
	ClockIcon,
	DownloadCloudIcon,
	ExternalLinkIcon,
	FileSearch,
	ImageIcon,
	LockIcon,
	MicIcon,
	MoreVerticalIcon,
	PencilIcon,
	PlusIcon,
	ScanEyeIcon,
	SparklesIcon,
	TrashIcon,
	TypeIcon,
	XIcon,
} from "lucide-react";
import type { JSX, ReactNode } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { useHub } from "../../hooks/use-hub";
import { useInvoke } from "../../hooks/use-invoke";
import { isMlxModelBit } from "../../lib/bit/mlx-model-pack";
import { type IBit, IBitTypes } from "../../lib/schema/bit/bit";
import type { IEmbeddingModelParameters } from "../../lib/schema/bit/bit/embedding-model-parameters";
import type { ILlmParameters } from "../../lib/schema/bit/bit/llm-parameters";
import { humanFileSize } from "../../lib/utils";
import { useBackend } from "../../state/backend-state";
import { useDownloadManager } from "../../state/download-manager";
import {
	handleUpgradeRequiredError,
	openUpgradeDialogIfEnabled,
} from "../../state/upgrade-dialog-state";
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
import {
	DeploymentLabel,
	ModalityFlow,
	ProviderGlyph,
	providerLabel,
} from "./model-kit";
import { Progress } from "./progress";

export type ModelCardVariant = "grid" | "list";

export interface ModelCardProps {
	bit: IBit;
	variant?: ModelCardVariant;
	onClick?: (bit: IBit) => void;
	/** Marks the model as user-owned, enabling the private badge + edit/delete. */
	isCustom?: boolean;
	onEdit?: (bit: IBit) => void;
	onDelete?: (bit: IBit) => void;
}

export function ModelCard({
	bit,
	variant = "grid",
	onClick,
	isCustom = false,
	onEdit,
	onDelete,
}: Readonly<ModelCardProps>) {
	const { t } = useTranslation("common");
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

	// The backend-resolved pack size is authoritative: it accounts for artifacts
	// the bit itself never names (inline MLX manifests, llama.cpp projectors).
	// Only fall back to the bit's own fields while that size is still loading.
	const isVirtualBit = useMemo(() => {
		if (bitSize.isSuccess) return (bitSize.data ?? 0) === 0;
		return (
			(bit.dependencies?.length ?? 0) === 0 &&
			!bit.download_link &&
			!isMlxModelBit(bit)
		);
	}, [bit, bitSize.data, bitSize.isSuccess]);

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
			if (isVirtualBit) {
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
		try {
			if (bitIndex === -1) {
				if (tierInfo.isRestricted) {
					if (
						!openUpgradeDialogIfEnabled({
							reason: "model-tier",
							requiredTier: tierInfo.requiredTier ?? undefined,
						})
					) {
						toast.error(
							t('thisModelRequiresTheRequiredtierPlan', 'This model requires the {{requiredTier}} plan.', { requiredTier: tierInfo.requiredTier }),
						);
					}
					return;
				}
				await downloadBit(bit);
				await backend.bitState.addBit(bit, profile);
			} else {
				await backend.bitState.removeBit(bit, profile);
			}
			await refetchCurrentProfile();
		} catch (error) {
			console.error("Failed to update profile models:", error);
			if (handleUpgradeRequiredError(error, "model-tier")) return;
			toast.error(
				error instanceof Error ? error.message : t('failedToUpdateProfile', 'Failed to update profile'),
			);
		}
	}, [
		currentProfile.data,
		bit,
		downloadBit,
		backend.bitState,
		refetchCurrentProfile,
		tierInfo,
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
	const isEmbeddingModel =
		isEmbeddingBit(bit) ||
		bit.type === IBitTypes.Tts ||
		bit.type === IBitTypes.Stt;

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
			isCustom={isCustom}
			onCardClick={() => onClick?.(bit)}
			onToggleDownload={toggleDownload}
			onToggleProfile={toggleProfile}
			onOpenRepository={openRepository}
			onEdit={onEdit ? () => onEdit(bit) : undefined}
			onDelete={onDelete ? () => onDelete(bit) : undefined}
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
	isCustom?: boolean;
	onCardClick: () => void;
	onToggleDownload: () => void;
	onToggleProfile: () => void;
	onOpenRepository: () => void;
	onEdit?: () => void;
	onDelete?: () => void;
}

function ModelCardGridVariant({
	bit,
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
	isCustom,
	onCardClick,
	onToggleDownload,
	onToggleProfile,
	onOpenRepository,
	onEdit,
	onDelete,
}: Readonly<ModelCardVariantProps>) {
	const { t } = useTranslation("common");
	const meta = bit.meta.en;
	if (!meta) return null;

	const description = meta.description?.trim();

	return (
		<article
			onClick={onCardClick}
			onKeyDown={(e) => e.key === "Enter" && onCardClick()}
			className={`group relative flex h-full cursor-pointer flex-col gap-3 overflow-hidden rounded-xl border bg-card p-3.5 transition-colors hover:border-foreground/25 hover:bg-muted/30 dark:border-white/10 dark:hover:border-white/20 ${
				isInProfile ? "border-primary/40 dark:border-primary/40" : ""
			}`}
		>
			{/* Download Overlay */}
			{progress !== undefined && !isVirtualBit && (
				<div className="absolute inset-0 z-30 flex items-center justify-center rounded-xl bg-background/90 backdrop-blur-sm">
					{isQueuedState ? (
						<div className="flex items-center gap-2">
							<ClockIcon className="h-4 w-4 animate-pulse text-primary" />
							<span className="text-sm text-muted-foreground">{t('queued', 'Queued')}</span>
						</div>
					) : (
						<div className="flex items-center gap-3">
							<Progress value={progress} className="h-1.5 w-24" />
							<span className="text-sm tabular-nums text-muted-foreground">{`${progress}%`}</span>
						</div>
					)}
				</div>
			)}

			<div className="flex items-start gap-2.5">
				<ProviderGlyph bit={bit} size={32} className="shrink-0" />
				<div className="min-w-0 flex-1">
					<div
						className="truncate text-[14px] font-semibold tracking-tight"
						title={meta.name}
					>
						{meta.name}
					</div>
					<div className="mt-0.5 flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
						<span className="truncate">{providerLabel(bit)}</span>
						{isCustom && (
							<LockIcon
								className="h-3 w-3 shrink-0"
								aria-label={t('privateToYou', 'Private to you')}
							/>
						)}
					</div>
				</div>
				<ModelCardDropdown
					isInstalled={isInstalled}
					isInProfile={isInProfile}
					hasRepository={!!bit.repository}
					bitSize={bitSize}
					onToggleDownload={onToggleDownload}
					onToggleProfile={onToggleProfile}
					onOpenRepository={onOpenRepository}
					onEdit={onEdit}
					onDelete={onDelete}
				/>
			</div>

			{description && (
				<p className="line-clamp-2 text-[12.5px] leading-relaxed text-muted-foreground">
					{description}
				</p>
			)}

			{/* One quiet spec line: what it takes in, what it returns, how big, where it runs */}
			<div className="mt-auto flex flex-wrap items-center gap-x-2 gap-y-1 text-[11.5px] text-muted-foreground">
				<ModalityFlow type={bit.type} plain />
				<span className="h-1 w-1 rounded-full bg-muted-foreground/40" />
				{contextLength ? (
					<>
						<span className="font-mono tabular-nums">
							{formatContextLength(contextLength)}
						</span>
						<span className="h-1 w-1 rounded-full bg-muted-foreground/40" />
					</>
				) : null}
				<DeploymentLabel
					kind={isHosted ? "hosted" : canRunRemotely ? "remote" : "local"}
				/>
				{!isHosted && isInstalled && (
					<>
						<span className="h-1 w-1 rounded-full bg-muted-foreground/40" />
						<span className="font-mono tabular-nums text-emerald-600 dark:text-emerald-400">
							{humanFileSize(bitSize)} {t('onDisk', 'on disk')}
						</span>
					</>
				)}
				{isRestricted && requiredTier && (
					<>
						<span className="h-1 w-1 rounded-full bg-muted-foreground/40" />
						<span className="font-semibold text-amber-600 dark:text-amber-400">{t('requiredtierPlan', '{{requiredTier}} plan', { requiredTier })}</span>
					</>
				)}
			</div>

			{bit.model_evaluation ? (
				<IntelligenceIndexBadge
					evaluation={bit.model_evaluation as IModelEvaluation | undefined}
				/>
			) : null}

			<Button
				variant="outline"
				size="sm"
				onClick={(e) => {
					e.stopPropagation();
					onToggleProfile();
				}}
				className={`h-8 w-full gap-1.5 rounded-lg text-xs font-semibold ${
					isInProfile
						? "border-primary/40 bg-primary/10 text-primary hover:bg-primary/15 hover:text-primary"
						: ""
				}`}
			>
				{isInProfile ? (
					<CheckIcon className="h-3.5 w-3.5" />
				) : (
					<PlusIcon className="h-3.5 w-3.5" />
				)}
				{isInProfile ? "In profile" : t('addToProfile2', 'Add to profile')}
			</Button>
		</article>
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
	const { t } = useTranslation("common");
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
							<span className="text-sm text-muted-foreground">{t('queued', 'Queued')}</span>
						</div>
					) : (
						<div className="flex items-center gap-3">
							<Progress value={progress} className="w-24 h-1.5" />
							<span className="text-sm text-muted-foreground tabular-nums">{`${progress}%`}</span>
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
						{t('remote', 'Remote')}
					</Badge>
				)}
				{isEmbeddingModel && !canRunRemotely && (
					<Badge
						variant="outline"
						className="text-[10px] px-1.5 py-0 h-5 bg-zinc-500/10 text-zinc-600 border-zinc-500/30"
					>
						{t('localOnly', 'Local only')}
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
	/** Present only for user-owned models. */
	onEdit?: () => void;
	onDelete?: () => void;
}

function ModelCardDropdown({
	isInstalled,
	isInProfile,
	hasRepository,
	bitSize,
	onToggleDownload,
	onToggleProfile,
	onOpenRepository,
	onEdit,
	onDelete,
}: Readonly<ModelCardDropdownProps>) {
	const { t } = useTranslation("common");
	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<Button
					size="sm"
					variant="ghost"
					aria-label={t('modelActions', 'Model actions')}
					className="h-7 w-7 shrink-0 p-0 text-muted-foreground/60 transition-colors hover:text-foreground"
					onClick={(e) => e.stopPropagation()}
				>
					<MoreVerticalIcon className="h-4 w-4" />
				</Button>
			</DropdownMenuTrigger>
			<DropdownMenuContent align="end" className="w-48">
				<DropdownMenuItem
					onClick={(e) => {
						e.stopPropagation();
						onToggleDownload();
					}}
				>
					{isInstalled ? (
						<>
							<TrashIcon className="h-4 w-4 mr-2" />
							{t('remove', 'Remove')}
						</>
					) : (
						<>
							<DownloadCloudIcon className="h-4 w-4 mr-2" />
							{t('download', 'Download (')}{humanFileSize(bitSize)})
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
							{`Remove from Profile`}
						</>
					) : (
						<>
							<PlusIcon className="h-4 w-4 mr-2" />
							{t('addToProfile', 'Add to Profile')}
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
							{t('viewRepository', 'View Repository')}
						</DropdownMenuItem>
					</>
				)}
				{(onEdit || onDelete) && <DropdownMenuSeparator />}
				{onEdit && (
					<DropdownMenuItem
						onClick={(e) => {
							e.stopPropagation();
							onEdit();
						}}
					>
						<PencilIcon className="h-4 w-4 mr-2" />
						{t('editModel', 'Edit model')}
					</DropdownMenuItem>
				)}
				{onDelete && (
					<DropdownMenuItem
						variant="destructive"
						onClick={(e) => {
							e.stopPropagation();
							onDelete();
						}}
					>
						<TrashIcon className="h-4 w-4 mr-2" />
						{t('deleteModel', 'Delete model')}
					</DropdownMenuItem>
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
	const { t } = useTranslation("common");
	if (isHosted) {
		return (
			<Badge
				variant="outline"
				className="text-[10px] px-1.5 py-0 h-5 bg-sky-500/10 text-sky-600 border-sky-500/30"
			>
				{t('hosted', 'Hosted')}
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
	const { t } = useTranslation("common");
	const cn = `h-4 w-4 ${className}`;
	switch (type) {
		case IBitTypes.Llm:
			return <BrainIcon className={cn} />;
		case IBitTypes.Vlm:
			return <CameraIcon className={cn} />;
		case IBitTypes.Tts:
			return <AudioLinesIcon className={cn} />;
		case IBitTypes.Stt:
			return <MicIcon className={cn} />;
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
	const { t } = useTranslation("common");
	const iconClass = "h-3 w-3";
	const arrowClass = `h-2.5 w-2.5 text-foreground`;

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
		case IBitTypes.Tts:
			return (
				<div className="flex items-center gap-1 text-muted-foreground">
					<TypeIcon className={`${iconClass} text-blue-500`} />
					<ArrowRightIcon className={arrowClass} />
					<AudioLinesIcon className={`${iconClass} text-rose-500`} />
				</div>
			);
		case IBitTypes.Stt:
			return (
				<div className="flex items-center gap-1 text-muted-foreground">
					<AudioLinesIcon className={`${iconClass} text-rose-500`} />
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
			return i18next.t('textText', 'Text → Text');
		case IBitTypes.Vlm:
			return i18next.t('imageText', 'Image → Text');
		case IBitTypes.Tts:
			return i18next.t('textSpeech', 'Text → Speech');
		case IBitTypes.Stt:
			return i18next.t('speechText', 'Speech → Text');
		case IBitTypes.Embedding:
			return i18next.t('textEmbedding', 'Text → Embedding');
		case IBitTypes.ImageEmbedding:
			return i18next.t('imageEmbedding', 'Image → Embedding');
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
	if (length >= 1_000_000) return i18next.t('valmCtx', '{{val}}M ctx', { val: (length / 1_000_000).toFixed(1) });
	if (length >= 1000) return i18next.t('valkCtx', '{{val}}K ctx', { val: Math.round(length / 1000) });
	return i18next.t('lengthCtx', '{{length}} ctx', { length });
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
		coding: { icon: "💻", label: i18next.t('coding', 'Coding'), color: "text-blue-500" },
		cost: { icon: "💰", label: i18next.t('costEfficiency', 'Cost Efficiency'), color: "text-green-500" },
		creativity: { icon: "🎨", label: i18next.t('creativity', 'Creativity'), color: "text-purple-500" },
		factuality: { icon: "📚", label: i18next.t('factuality', 'Factuality'), color: "text-amber-500" },
		function_calling: {
			icon: "🔧",
			label: i18next.t('functionCalling', 'Function Calling'),
			color: "text-cyan-500",
		},
		multilinguality: {
			icon: "🌍",
			label: i18next.t('multilingual', 'Multilingual'),
			color: "text-teal-500",
		},
		openness: { icon: "🔓", label: i18next.t('openness', 'Openness'), color: "text-orange-500" },
		reasoning: { icon: "🧠", label: i18next.t('reasoning', 'Reasoning'), color: "text-pink-500" },
		safety: { icon: "🛡️", label: i18next.t('safety', 'Safety'), color: "text-red-500" },
		speed: { icon: "⚡", label: i18next.t('speed', 'Speed'), color: "text-yellow-500" },
	};
	return icons[key] || { icon: "❓", label: key, color: "text-gray-500" };
}
