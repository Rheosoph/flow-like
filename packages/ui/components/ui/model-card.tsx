"use client";
import { i18n as i18next, useTranslation } from "@flow-like/locales";
import type { UseQueryResult } from "@tanstack/react-query";
import {
	CheckIcon,
	ClockIcon,
	DownloadCloudIcon,
	ExternalLinkIcon,
	LockIcon,
	MoreHorizontalIcon,
	PencilIcon,
	PlusIcon,
	TrashIcon,
	XIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import {
	isBitInProfile,
	useAddModelToProfile,
} from "../../hooks/use-add-model-to-profile";
import { useInvoke } from "../../hooks/use-invoke";
import { isMlxModelBit } from "../../lib/bit/mlx-model-pack";
import { bitPreviewMedia } from "../../lib/bit/model-media";
import {
	type PriceDeployment,
	type PriceTier,
	bitPrice,
} from "../../lib/bit/model-pricing";
import { bitRating } from "../../lib/bit/model-rating";
import { type IBit, IBitTypes } from "../../lib/schema/bit/bit";
import type { IEmbeddingModelParameters } from "../../lib/schema/bit/bit/embedding-model-parameters";
import type { ILlmParameters } from "../../lib/schema/bit/bit/llm-parameters";
import { cn, humanFileSize } from "../../lib/utils";
import { useBackend } from "../../state/backend-state";
import { useDownloadManager } from "../../state/download-manager";
import type { ISettingsProfile } from "../../types";
import { DeviceBitBadge } from "../settings/devices/models/use/device-model-badge";
import { Button } from "./button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "./dropdown-menu";
import {
	type Deployment,
	DeploymentLabel,
	ModalityFlow,
	ProviderGlyph,
	providerLabel,
} from "./model-kit";
import { AudioSample, AudioSampleButton, MediaStrip } from "./model-media";
import { PriceTierMark, usePriceText } from "./model-price";
import { ModelRatingBadge } from "./model-rating";
import { Progress } from "./progress";

export type ModelCardVariant = "grid" | "list";

export interface ModelCardProps {
	bit: IBit;
	/** Isolates profile and hub queries for embedded collections. */
	queryScope?: string[];
	onProfileChange?: () => void | Promise<void>;
	variant?: ModelCardVariant;
	onClick?: (bit: IBit) => void;
	/** Marks the model as user-owned, enabling the private badge + edit/delete. */
	isCustom?: boolean;
	onEdit?: (bit: IBit) => void;
	onDelete?: (bit: IBit) => void;
	/** Where the price sits among models of the same type, from the surrounding catalog. */
	priceTier?: PriceTier;
}

const LOCAL_PROVIDER_LABEL = "On-device";

/**
 * Who makes the model. Local runtimes are labelled "On-device" as their provider,
 * which the deployment label already says, so name the authors instead.
 */
export function modelSourceLabel(bit: IBit): string {
	const provider = providerLabel(bit);
	if (provider !== LOCAL_PROVIDER_LABEL) return provider;
	return (
		bit.authors
			?.map((author) => author.trim())
			.filter(Boolean)
			.join(", ") || provider
	);
}

/** How a model runs, as the catalog shows it. */
export function modelDeployment(
	bit: IBit,
	isHosted: boolean,
): { kind: Deployment; price: PriceDeployment } {
	if (bit.type === IBitTypes.Forecasting)
		return { kind: "native", price: "native" };
	if (isHosted) return { kind: "hosted", price: "hosted" };
	if (supportsRemoteEmbeddingExecution(bit))
		return { kind: "remote", price: "hosted" };
	return { kind: "local", price: "local" };
}

export function ModelCard({
	bit,
	queryScope = [],
	onProfileChange,
	variant = "grid",
	onClick,
	isCustom = false,
	onEdit,
	onDelete,
	priceTier,
}: Readonly<ModelCardProps>) {
	const { t } = useTranslation("common");
	const backend = useBackend();
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
		true,
		queryScope,
	);
	const bitSize: UseQueryResult<number> = useInvoke(
		backend.bitState.getBitSize,
		backend.bitState,
		[bit],
		true,
		queryScope,
	);
	const currentProfile: UseQueryResult<ISettingsProfile> = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
		true,
		queryScope,
	);
	const { gate, add: addToProfile } = useAddModelToProfile(queryScope);

	// The backend-resolved pack size is authoritative: it accounts for artifacts
	// the bit itself never names (inline MLX manifests, llama.cpp projectors).
	// Only fall back to the bit's own fields while that size is still loading.
	const isVirtualBit = useMemo(() => {
		if (bit.type === IBitTypes.Forecasting) return false;
		if (bitSize.isSuccess) return (bitSize.data ?? 0) === 0;
		return (
			(bit.dependencies?.length ?? 0) === 0 &&
			!bit.download_link &&
			!isMlxModelBit(bit)
		);
	}, [bit, bitSize.data, bitSize.isSuccess]);

	const tierInfo = useMemo(() => gate(bit), [gate, bit]);

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
		try {
			await downloadBit(bit);
		} catch (error) {
			console.error("Failed to download bit:", error);
			toast.error(t("downloadFailed", "Download failed"), {
				description: error instanceof Error ? error.message : undefined,
			});
		}
	}, [
		isInstalled.data,
		backend.bitState,
		bit,
		downloadBit,
		refetchIsInstalled,
		t,
	]);

	const refetchCurrentProfile = currentProfile.refetch;
	const toggleProfile = useCallback(async () => {
		const profile = currentProfile.data;
		if (!profile) return;
		if (!isBitInProfile(profile, bit.id)) {
			if (await addToProfile(bit, downloadBit)) await onProfileChange?.();
			return;
		}
		try {
			await backend.bitState.removeBit(bit, profile);
			await refetchCurrentProfile();
			await onProfileChange?.();
		} catch (error) {
			console.error(`Removing model ${bit.id} from the profile failed:`, error);
			toast.error(
				error instanceof Error
					? error.message
					: t("failedToUpdateProfile", "Failed to update profile"),
			);
		}
	}, [
		currentProfile.data,
		bit,
		addToProfile,
		downloadBit,
		backend.bitState,
		refetchCurrentProfile,
		onProfileChange,
		t,
	]);

	const openRepository = useCallback(() => {
		if (bit.repository) window.open(bit.repository, "_blank");
	}, [bit.repository]);

	const rating = useMemo(() => bitRating(bit), [bit]);
	const media = useMemo(() => bitPreviewMedia(bit), [bit]);

	if (bit.meta.en === undefined) return null;

	const isHosted =
		bit.type !== IBitTypes.Forecasting && (bitSize.data === 0 || isVirtualBit);
	const deployment = modelDeployment(bit, isHosted);
	const size = !isVirtualBit && bitSize.data ? humanFileSize(bitSize.data) : "";

	const view: ModelCardView = {
		bit,
		isCustom,
		isInstalled: !!isInstalled.data,
		isInProfile: isBitInProfile(currentProfile.data, bit.id),
		deployment: deployment.kind,
		price: bitPrice(bit, deployment.price),
		priceTier,
		rating,
		media,
		size,
		contextLength: (bit.parameters as ILlmParameters | undefined)
			?.context_length,
		requiredTier: tierInfo.isRestricted ? tierInfo.requiredTier : null,
		description:
			bit.meta.en.description?.trim() ||
			(isCustom
				? t("customModelOnYourKey", "Custom model on your {{provider}} key.", {
						provider: providerLabel(bit),
					})
				: ""),
		overlay:
			progress !== undefined && !isVirtualBit ? (
				<DownloadOverlay progress={progress} queued={isQueuedState} />
			) : null,
		menu: (
			<ModelCardDropdown
				name={bit.meta.en.name}
				canDownload={!isVirtualBit}
				isInstalled={!!isInstalled.data}
				isInProfile={isBitInProfile(currentProfile.data, bit.id)}
				hasRepository={!!bit.repository}
				bitSize={bitSize.data ?? 0}
				onToggleDownload={toggleDownload}
				onToggleProfile={toggleProfile}
				onOpenRepository={openRepository}
				onEdit={onEdit ? () => onEdit(bit) : undefined}
				onDelete={onDelete ? () => onDelete(bit) : undefined}
			/>
		),
		onOpen: () => onClick?.(bit),
		onToggleProfile: toggleProfile,
	};

	return variant === "list" ? (
		<ModelCardRow {...view} />
	) : (
		<ModelCardTile {...view} />
	);
}

interface ModelCardView {
	bit: IBit;
	isCustom: boolean;
	isInstalled: boolean;
	isInProfile: boolean;
	deployment: Deployment;
	price: ReturnType<typeof bitPrice>;
	priceTier?: PriceTier;
	rating: ReturnType<typeof bitRating>;
	media: ReturnType<typeof bitPreviewMedia>;
	size: string;
	contextLength?: number;
	requiredTier: string | null;
	description: string;
	overlay: ReactNode;
	menu: ReactNode;
	onOpen: () => void;
	onToggleProfile: () => void;
}

function DownloadOverlay({
	progress,
	queued,
}: Readonly<{ progress: number; queued: boolean }>) {
	const { t } = useTranslation("common");
	return (
		<div className="absolute inset-0 z-30 flex items-center justify-center rounded-[inherit] bg-background/90 backdrop-blur-sm">
			{queued ? (
				<div className="flex items-center gap-2">
					<ClockIcon className="h-4 w-4 animate-pulse text-primary" />
					<span className="text-sm text-muted-foreground">
						{t("queued", "Queued")}
					</span>
				</div>
			) : (
				<div className="flex items-center gap-3">
					<Progress value={progress} className="h-1.5 w-24" />
					<span className="text-sm tabular-nums text-muted-foreground">{`${progress}%`}</span>
				</div>
			)}
		</div>
	);
}

/** The name doubles as the open-details button; its overlay makes the whole card clickable. */
function OpenDetailsButton({
	name,
	onOpen,
	className,
}: Readonly<{ name: string; onOpen: () => void; className?: string }>) {
	const { t } = useTranslation("common");
	return (
		<button
			type="button"
			onClick={onOpen}
			title={name}
			aria-label={t("viewModelDetails", "View details of {{name}}", { name })}
			className={cn(
				"block max-w-full truncate rounded text-left after:absolute after:inset-0 after:z-[1] after:content-[''] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary",
				className,
			)}
		>
			{name}
		</button>
	);
}

function ModelSpecs({ view }: Readonly<{ view: ModelCardView }>) {
	const { t } = useTranslation("common");
	const specs: string[] = [];
	if (view.contextLength) specs.push(formatContextLength(view.contextLength));
	const embedding = view.bit.parameters as
		| IEmbeddingModelParameters
		| undefined;
	if (
		(view.bit.type === IBitTypes.Embedding ||
			view.bit.type === IBitTypes.ImageEmbedding) &&
		embedding?.vector_length
	)
		specs.push(
			t("vectorDimensions", "{{count}} dims", {
				count: embedding.vector_length,
			}),
		);
	if (view.deployment === "local" && view.isInstalled && view.size)
		specs.push(`${view.size} ${t("onDisk", "on disk")}`);
	return (
		<>
			{specs.map((spec) => (
				<span key={spec} className="font-mono tabular-nums">
					{spec}
				</span>
			))}
			{view.requiredTier && (
				<span className="font-semibold text-amber-600 dark:text-amber-400">
					{t("requiredtierPlan", "{{requiredTier}} plan", {
						requiredTier: view.requiredTier,
					})}
				</span>
			)}
		</>
	);
}

function PriceSummary({
	view,
	compact = false,
}: Readonly<{ view: ModelCardView; compact?: boolean }>) {
	const priceText = usePriceText()(
		view.price,
		view.deployment === "local" && !view.isInstalled ? view.size : undefined,
	);
	return (
		<div className="min-w-0 flex-1">
			<div className="flex min-w-0 items-baseline gap-2">
				<span
					className={cn(
						"truncate font-mono font-semibold tabular-nums",
						compact ? "text-[12.5px]" : "text-[13px]",
						priceText.free && "text-emerald-600 dark:text-emerald-400",
					)}
				>
					{priceText.main}
				</span>
				{view.priceTier && (
					<PriceTierMark tier={view.priceTier} type={view.bit.type} />
				)}
			</div>
			{priceText.sub && (
				<p className="truncate text-[11.5px] text-muted-foreground">
					{priceText.sub}
				</p>
			)}
		</div>
	);
}

function ProfileButton({ view }: Readonly<{ view: ModelCardView }>) {
	const { t } = useTranslation("common");
	const name = view.bit.meta.en?.name ?? view.bit.id;
	return (
		<Button
			variant="outline"
			size="sm"
			aria-pressed={view.isInProfile}
			aria-label={
				view.isInProfile
					? t("removeNameFromProfile", "Remove {{name}} from your profile", {
							name,
						})
					: t("addNameToProfile", "Add {{name}} to your profile", { name })
			}
			onClick={view.onToggleProfile}
			className={cn(
				"relative z-10 h-8 shrink-0 gap-1.5 px-2.5 text-xs font-semibold",
				view.isInProfile &&
					"border-primary/40 bg-primary/10 text-primary hover:bg-primary/15 hover:text-primary",
			)}
		>
			{view.isInProfile ? (
				<CheckIcon className="h-3.5 w-3.5" />
			) : (
				<PlusIcon className="h-3.5 w-3.5" />
			)}
			{view.isInProfile ? t("modelInProfile", "In profile") : t("add", "Add")}
		</Button>
	);
}

function ProviderLine({
	view,
	className,
}: Readonly<{ view: ModelCardView; className?: string }>) {
	const { t } = useTranslation("common");
	const source = modelSourceLabel(view.bit);
	return (
		<p
			className={cn(
				"flex min-w-0 items-center gap-1.5 text-[11.5px] text-muted-foreground",
				className,
			)}
		>
			<span className="truncate">{source}</span>
			{view.isCustom && (
				<LockIcon
					className="h-3 w-3 shrink-0"
					aria-label={t("privateToYou", "Private to you")}
				/>
			)}
			{source !== LOCAL_PROVIDER_LABEL && (
				<DeploymentLabel kind={view.deployment} className="shrink-0" />
			)}
			<DeviceBitBadge bit={view.bit} />
		</p>
	);
}

function ModelCardTile(view: Readonly<ModelCardView>) {
	const { t } = useTranslation("common");
	const meta = view.bit.meta.en;
	if (!meta) return null;
	const audio = view.media.audio[0];

	return (
		<article
			className={cn(
				"group relative flex h-full min-w-0 flex-col overflow-hidden rounded-xl border bg-card transition-colors hover:border-foreground/25 hover:bg-muted/20 dark:border-white/10 dark:hover:border-white/20",
				view.isInProfile && "border-primary/45 dark:border-primary/45",
			)}
		>
			{view.overlay}
			<MediaStrip images={view.media.images} videos={view.media.video} />

			<div className="flex flex-1 flex-col gap-2.5 p-3.5 pb-3">
				<div className="flex items-start gap-2.5">
					<ProviderGlyph bit={view.bit} size={36} className="shrink-0" />
					<div className="min-w-0 flex-1">
						<h3 className="m-0 text-[14px] font-semibold leading-snug tracking-tight">
							<OpenDetailsButton name={meta.name} onOpen={view.onOpen} />
						</h3>
						<ProviderLine view={view} className="mt-0.5" />
					</div>
					<div className="relative z-10 -mt-1 -mr-1.5">{view.menu}</div>
				</div>

				{audio && (
					<AudioSample
						src={audio}
						label={meta.name}
						caption={
							view.media.audio.length > 1
								? t("audioSampleCount", "{{count}} samples", {
										count: view.media.audio.length,
									})
								: t("audioSampleOne", "Listen to a sample")
						}
						className="relative z-10"
					/>
				)}

				{view.description && (
					<p className="line-clamp-2 text-[12.5px] leading-relaxed text-muted-foreground">
						{view.description}
					</p>
				)}

				<div className="mt-auto flex flex-wrap items-center gap-x-2 gap-y-1.5 text-[11.5px] text-muted-foreground">
					<ModelRatingBadge
						rating={view.rating}
						type={view.bit.type}
						className="relative z-10"
					/>
					<ModalityFlow type={view.bit.type} compact />
					<ModelSpecs view={view} />
				</div>
			</div>

			<div className="flex items-center gap-2.5 border-t px-3.5 py-2.5 dark:border-white/10">
				<PriceSummary view={view} />
				<ProfileButton view={view} />
			</div>
		</article>
	);
}

function ModelCardRow(view: Readonly<ModelCardView>) {
	const meta = view.bit.meta.en;
	if (!meta) return null;
	const audio = view.media.audio[0];

	return (
		<div
			className={cn(
				"group relative flex min-w-0 flex-wrap items-center gap-x-4 gap-y-2 rounded-lg border bg-card px-3 py-2 transition-colors hover:bg-muted/30 dark:border-white/10 md:grid md:grid-cols-[minmax(0,2.4fr)_minmax(0,1.4fr)_12.5rem_minmax(0,1.4fr)_11rem]",
				view.isInProfile && "border-primary/40 dark:border-primary/40",
			)}
		>
			{view.overlay}
			<div className="flex min-w-0 flex-1 items-center gap-2.5">
				<ProviderGlyph bit={view.bit} size={30} className="shrink-0" />
				<div className="min-w-0">
					<OpenDetailsButton
						name={meta.name}
						onOpen={view.onOpen}
						className="text-[13.5px] font-semibold"
					/>
					<ProviderLine view={view} />
				</div>
			</div>
			<div className="flex min-w-0 flex-wrap items-center gap-2 text-[11.5px] text-muted-foreground">
				<ModalityFlow type={view.bit.type} compact />
				<ModelSpecs view={view} />
			</div>
			<div>
				<ModelRatingBadge
					rating={view.rating}
					type={view.bit.type}
					className="relative z-10"
				/>
			</div>
			<PriceSummary view={view} compact />
			<div className="relative z-10 flex items-center justify-end gap-1.5">
				{audio && <AudioSampleButton src={audio} label={meta.name} />}
				<ProfileButton view={view} />
				{view.menu}
			</div>
		</div>
	);
}

interface ModelCardDropdownProps {
	name: string;
	canDownload: boolean;
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
	name,
	canDownload,
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
					aria-label={t("modelActionsFor", "More actions for {{name}}", {
						name,
					})}
					className="h-7 w-7 shrink-0 p-0 text-muted-foreground/70 transition-colors hover:text-foreground"
					onClick={(e) => e.stopPropagation()}
				>
					<MoreHorizontalIcon className="h-4 w-4" />
				</Button>
			</DropdownMenuTrigger>
			<DropdownMenuContent align="end" className="w-52">
				{onEdit && (
					<DropdownMenuItem
						onClick={(e) => {
							e.stopPropagation();
							onEdit();
						}}
					>
						<PencilIcon className="h-4 w-4 mr-2" />
						{t("editModel", "Edit model")}
					</DropdownMenuItem>
				)}
				{canDownload && (
					<DropdownMenuItem
						onClick={(e) => {
							e.stopPropagation();
							onToggleDownload();
						}}
					>
						{isInstalled ? (
							<>
								<TrashIcon className="h-4 w-4 mr-2" />
								{t("removeDownload", "Remove Download")}
							</>
						) : (
							<>
								<DownloadCloudIcon className="h-4 w-4 mr-2" />
								{t("downloadWithSize", "Download ({{size}})", {
									size: humanFileSize(bitSize),
								})}
							</>
						)}
					</DropdownMenuItem>
				)}
				<DropdownMenuItem
					onClick={(e) => {
						e.stopPropagation();
						onToggleProfile();
					}}
				>
					{isInProfile ? (
						<>
							<XIcon className="h-4 w-4 mr-2" />
							{t("removeFromProfile", "Remove from profile")}
						</>
					) : (
						<>
							<PlusIcon className="h-4 w-4 mr-2" />
							{t("addToProfile", "Add to Profile")}
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
							{t("viewRepository", "View Repository")}
						</DropdownMenuItem>
					</>
				)}
				{onDelete && (
					<>
						<DropdownMenuSeparator />
						<DropdownMenuItem
							variant="destructive"
							onClick={(e) => {
								e.stopPropagation();
								onDelete();
							}}
						>
							<TrashIcon className="h-4 w-4 mr-2" />
							{t("deleteModel", "Delete model")}
						</DropdownMenuItem>
					</>
				)}
			</DropdownMenuContent>
		</DropdownMenu>
	);
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
	if (length >= 1_000_000)
		return i18next.t("valmCtx", "{{val}}M ctx", {
			val: (length / 1_000_000).toFixed(1),
		});
	if (length >= 1000)
		return i18next.t("valkCtx", "{{val}}K ctx", {
			val: Math.round(length / 1000),
		});
	return i18next.t("lengthCtx", "{{length}} ctx", { length });
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
		coding: {
			icon: "💻",
			label: i18next.t("coding", "Coding"),
			color: "text-blue-500",
		},
		cost: {
			icon: "💰",
			label: i18next.t("costEfficiency", "Cost Efficiency"),
			color: "text-green-500",
		},
		creativity: {
			icon: "🎨",
			label: i18next.t("creativity", "Creativity"),
			color: "text-purple-500",
		},
		factuality: {
			icon: "📚",
			label: i18next.t("factuality", "Factuality"),
			color: "text-amber-500",
		},
		function_calling: {
			icon: "🔧",
			label: i18next.t("functionCalling", "Function Calling"),
			color: "text-cyan-500",
		},
		multilinguality: {
			icon: "🌍",
			label: i18next.t("multilingual", "Multilingual"),
			color: "text-teal-500",
		},
		openness: {
			icon: "🔓",
			label: i18next.t("openness", "Openness"),
			color: "text-orange-500",
		},
		reasoning: {
			icon: "🧠",
			label: i18next.t("reasoning", "Reasoning"),
			color: "text-pink-500",
		},
		safety: {
			icon: "🛡️",
			label: i18next.t("safety", "Safety"),
			color: "text-red-500",
		},
		speed: {
			icon: "⚡",
			label: i18next.t("speed", "Speed"),
			color: "text-yellow-500",
		},
	};
	return icons[key] || { icon: "❓", label: key, color: "text-gray-500" };
}
