"use client";
import { useTranslation } from "@flow-like/locales";
import type { UseQueryResult } from "@tanstack/react-query";
import {
	CheckIcon,
	ClockIcon,
	CopyIcon,
	DownloadCloudIcon,
	ExternalLinkIcon,
	PencilIcon,
	PlusIcon,
	TrashIcon,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import {
	isBitInProfile,
	useAddModelToProfile,
} from "../../hooks/use-add-model-to-profile";
import { useInvoke } from "../../hooks/use-invoke";
import { isMlxModelBit } from "../../lib/bit/mlx-model-pack";
import { getModelCapability } from "../../lib/bit/model-capabilities";
import { bitPreviewMedia } from "../../lib/bit/model-media";
import { type PriceTier, bitPrice } from "../../lib/bit/model-pricing";
import { bitRating } from "../../lib/bit/model-rating";
import { type IBit, IBitTypes } from "../../lib/schema/bit/bit";
import type { IEmbeddingModelParameters } from "../../lib/schema/bit/bit/embedding-model-parameters";
import type {
	IBitModelClassification,
	ILlmParameters,
} from "../../lib/schema/bit/bit/llm-parameters";
import { cn, humanFileSize } from "../../lib/utils";
import { useBackend } from "../../state/backend-state";
import { useDownloadManager } from "../../state/download-manager";
import type { ISettingsProfile } from "../../types";
import { Button } from "./button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "./dialog";
import type { IModelEvaluation } from "./model-benchmarks";
import { ModelBenchmarks } from "./model-benchmarks";
import {
	getCapabilityIcon,
	modelDeployment,
	modelSourceLabel,
} from "./model-card";
import {
	type Deployment,
	DeploymentBadge,
	HostingRegionBadge,
	MODALITY,
	ModalityFlow,
	ProviderGlyph,
	bitModalities,
} from "./model-kit";
import { AudioSample, MediaGallery, mediaLabel } from "./model-media";
import { ModelPricePanel, usePriceText } from "./model-price";
import { ModelRatingBadge, ModelRatingPanel } from "./model-rating";
import { Progress } from "./progress";

export interface ModelDetailDialogProps {
	bit: IBit | null;
	/** Isolates profile and hub queries for embedded collections. */
	queryScope?: string[];
	onProfileChange?: () => void | Promise<void>;
	onEdit?: (bit: IBit) => void;
	open: boolean;
	onOpenChange: (open: boolean) => void;
	webMode?: boolean;
	/** Where the price sits among models of the same type, from the surrounding catalog. */
	priceTier?: PriceTier;
}

function useDownloadProgress(bit: IBit | null) {
	const onProgress = useDownloadManager((s) => s.onProgress);
	const isQueued = useDownloadManager((s) => s.isQueued);
	const getLatestPct = useDownloadManager((s) => s.getLatestPct);
	const [progress, setProgress] = useState<number | undefined>();
	const mountedRef = useRef(true);
	const lastPctRef = useRef(0);
	const lastUpdateRef = useRef(0);

	useEffect(() => {
		if (!bit) return;
		mountedRef.current = true;
		const initial = getLatestPct(bit.hash);
		if (typeof initial === "number") {
			setProgress(initial);
			lastPctRef.current = initial;
		} else if (isQueued(bit.hash)) {
			setProgress(0);
			lastPctRef.current = 0;
		}
		const unsubscribe = onProgress(bit.hash, (dl) => {
			const rawProgress = dl.progress();
			const pct = Math.round(rawProgress * 100);
			const now = Date.now();
			const completed =
				rawProgress >= 0.999 || dl.total().downloaded >= dl.total().max;
			if (!mountedRef.current) return;
			if (completed) {
				setProgress(undefined);
				lastPctRef.current = 0;
				lastUpdateRef.current = now;
				return;
			}
			if (
				Math.abs(pct - lastPctRef.current) >= 1 ||
				now - lastUpdateRef.current >= 250
			) {
				setProgress(pct);
				lastPctRef.current = pct;
				lastUpdateRef.current = now;
			}
		});
		return () => {
			mountedRef.current = false;
			unsubscribe();
			lastPctRef.current = 0;
			lastUpdateRef.current = 0;
			setProgress(undefined);
		};
	}, [bit, getLatestPct, isQueued, onProgress]);

	const reset = useCallback(() => {
		if (!mountedRef.current) return;
		setProgress(undefined);
		lastPctRef.current = 0;
		lastUpdateRef.current = 0;
	}, []);

	return { progress, setProgress, reset };
}

export function ModelDetailDialog({
	bit,
	queryScope = [],
	onProfileChange,
	onEdit,
	open,
	onOpenChange,
	webMode = false,
	priceTier,
}: Readonly<ModelDetailDialogProps>) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const download = useDownloadManager((s) => s.download);
	const { progress, setProgress, reset } = useDownloadProgress(bit);
	const { add } = useAddModelToProfile(queryScope);
	const priceText = usePriceText();

	const isInstalled: UseQueryResult<boolean> = useInvoke(
		backend.bitState.isBitInstalled,
		backend.bitState,
		// biome-ignore lint/style/noNonNullAssertion: bit is guaranteed by enabled flag
		[bit!],
		!!bit,
		queryScope,
	);
	const bitSize: UseQueryResult<number> = useInvoke(
		backend.bitState.getBitSize,
		backend.bitState,
		// biome-ignore lint/style/noNonNullAssertion: bit is guaranteed by enabled flag
		[bit!],
		!!bit,
		queryScope,
	);
	const currentProfile: UseQueryResult<ISettingsProfile> = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
		true,
		queryScope,
	);
	const detailedBit = useInvoke(
		backend.bitState.getBit,
		backend.bitState,
		[bit?.id ?? "", bit?.hub],
		!!bit && open,
		[bit?.updated ?? "", ...queryScope],
		60_000,
	);
	const displayBit = detailedBit.data ?? bit;

	// The backend-resolved pack size is authoritative: it accounts for artifacts
	// the bit itself never names (inline MLX manifests, llama.cpp projectors).
	const isVirtualBit = useMemo(() => {
		if (displayBit?.type === IBitTypes.Forecasting) return false;
		if (bitSize.isSuccess) return (bitSize.data ?? 0) === 0;
		return (
			(displayBit?.dependencies?.length ?? 0) === 0 &&
			!displayBit?.download_link &&
			!(displayBit && isMlxModelBit(displayBit))
		);
	}, [displayBit, bitSize.data, bitSize.isSuccess]);

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
				reset();
			}
		},
		[download, isInstalled, isVirtualBit, setProgress, reset],
	);

	const refetchIsInstalled = isInstalled.refetch;
	const handleDownload = useCallback(async () => {
		if (!displayBit) return;
		if (isInstalled.data) {
			await backend.bitState.deleteBit(displayBit);
			await refetchIsInstalled();
			return;
		}
		try {
			await downloadBit(displayBit);
		} catch (error) {
			console.error("Failed to download bit:", error);
			toast.error(t("downloadFailed", "Download failed"), {
				description: error instanceof Error ? error.message : undefined,
			});
		}
	}, [
		displayBit,
		isInstalled.data,
		backend.bitState,
		downloadBit,
		refetchIsInstalled,
		t,
	]);

	const refetchCurrentProfile = currentProfile.refetch;
	const handleToggleProfile = useCallback(async () => {
		const profile = currentProfile.data;
		if (!displayBit || !profile) return;
		if (!isBitInProfile(profile, displayBit.id)) {
			if (await add(displayBit, downloadBit)) await onProfileChange?.();
			return;
		}
		try {
			await backend.bitState.removeBit(displayBit, profile);
			await refetchCurrentProfile();
			await onProfileChange?.();
		} catch (error) {
			console.error(
				`Removing model ${displayBit.id} from the profile failed:`,
				error,
			);
			toast.error(
				error instanceof Error
					? error.message
					: t("failedToUpdateProfile", "Failed to update profile"),
			);
		}
	}, [
		currentProfile.data,
		displayBit,
		add,
		downloadBit,
		backend.bitState,
		refetchCurrentProfile,
		onProfileChange,
		t,
	]);

	const rating = useMemo(
		() => (displayBit ? bitRating(displayBit) : null),
		[displayBit],
	);
	const media = useMemo(
		() =>
			displayBit
				? bitPreviewMedia(displayBit)
				: { images: [], audio: [], video: [] },
		[displayBit],
	);

	if (!displayBit?.meta.en) return null;

	const meta = displayBit.meta.en;
	const isInProfile = isBitInProfile(currentProfile.data, displayBit.id);
	const isHosted =
		displayBit.type !== IBitTypes.Forecasting &&
		(bitSize.data === 0 || isVirtualBit);
	const deployment = modelDeployment(displayBit, isHosted);
	const size = !isVirtualBit && bitSize.data ? humanFileSize(bitSize.data) : "";
	const price = bitPrice(displayBit, deployment.price);
	const summary = priceText(
		price,
		deployment.kind === "local" && !isInstalled.data ? size : undefined,
	);
	const canDownload = !webMode && !isVirtualBit && deployment.kind !== "native";
	const classification = (displayBit.parameters as ILlmParameters | undefined)
		?.model_classification;
	const benchmarkNotes = (
		displayBit.parameters as { benchmark_notes?: unknown } | undefined
	)?.benchmark_notes;
	const isSpeech =
		displayBit.type === IBitTypes.Tts || displayBit.type === IBitTypes.Stt;

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent
				onOpenAutoFocus={(event) => {
					event.preventDefault();
					(event.currentTarget as HTMLElement | null)?.focus();
				}}
				className="h-[min(820px,calc(100dvh-2rem))] gap-0 overflow-hidden p-0 outline-none sm:max-w-240"
			>
				<DialogHeader className="flex-row items-start gap-3.5 border-b px-6 pt-5 pb-4 pr-14 text-left sm:text-left">
					<ProviderGlyph bit={displayBit} size={48} className="shrink-0" />
					<div className="min-w-0 flex-1">
						<DialogTitle className="text-xl font-semibold leading-tight tracking-tight">
							{meta.name}
						</DialogTitle>
						<DialogDescription className="mt-1 text-[13px]">
							{modelSourceLabel(displayBit)}
						</DialogDescription>
						<div className="mt-2.5 flex flex-wrap items-center gap-1.5">
							<DeploymentBadge kind={deployment.kind} />
							<HostingRegionBadge
								bit={displayBit}
								className="h-5.5 gap-1 rounded-md px-2 text-[11px] font-semibold"
							/>
							<ModalityFlow type={displayBit.type} />
							<ModelRatingBadge rating={rating} type={displayBit.type} />
							{isInProfile && (
								<span className="inline-flex h-5.5 items-center gap-1 rounded-md border border-primary/40 bg-primary/10 px-2 text-[11px] font-semibold text-primary">
									<CheckIcon className="h-3 w-3" />
									{t("inYourProfile", "In your profile")}
								</span>
							)}
						</div>
					</div>
				</DialogHeader>

				{progress !== undefined && !isVirtualBit && (
					<div className="flex items-center gap-3 border-b bg-muted/40 px-6 py-2.5">
						{progress === 0 ? (
							<>
								<ClockIcon className="h-4 w-4 animate-pulse text-primary" />
								<span className="text-sm">
									{t("queuedForDownload", "Queued for download...")}
								</span>
							</>
						) : (
							<>
								<Progress value={progress} className="h-2 flex-1" />
								<span className="w-12 text-right text-sm tabular-nums">{`${progress}%`}</span>
							</>
						)}
					</div>
				)}

				<div className="min-h-0 flex-1 overflow-y-auto overscroll-contain">
					<div className="flex flex-wrap items-start gap-6 px-6 py-5">
						<div className="flex min-w-0 flex-[999_1_26rem] flex-col gap-6">
							{(media.images.length > 0 || media.video.length > 0) && (
								<MediaGallery
									key={displayBit.id}
									images={media.images}
									videos={media.video}
									name={meta.name}
								/>
							)}
							{media.audio.length > 0 && (
								<DetailSection
									title={
										displayBit.type === IBitTypes.Tts
											? t("voiceSamples", "Voice samples")
											: t("sampleRecordings", "Sample recordings")
									}
								>
									<div className="flex flex-col gap-2">
										{media.audio.map((src, index) => (
											<AudioSample
												key={src}
												src={src}
												size="large"
												label={meta.name}
												caption={
													mediaLabel(src) ||
													t(
														"sampleCounter",
														"Sample {{position}} of {{total}}",
														{
															position: index + 1,
															total: media.audio.length,
														},
													)
												}
											/>
										))}
									</div>
								</DetailSection>
							)}
							{rating && (
								<ModelRatingPanel rating={rating} type={displayBit.type} />
							)}
							{displayBit.model_evaluation && (
								<ModelBenchmarks
									evaluation={displayBit.model_evaluation as IModelEvaluation}
									type={displayBit.type}
									hideRating
								/>
							)}
							{typeof benchmarkNotes === "string" && benchmarkNotes.trim() && (
								<p className="text-xs text-muted-foreground">
									{benchmarkNotes}
								</p>
							)}
							<DetailSection title={t("about", "About")}>
								<p className="max-w-[64ch] whitespace-pre-line text-[13.5px] leading-relaxed text-foreground/85">
									{meta.long_description?.trim() || meta.description}
								</p>
								{meta.use_case && (
									<p className="mt-2 text-[13px] text-muted-foreground">
										<span className="font-medium text-foreground/80">
											{t("goodFor", "Good for:")}
										</span>{" "}
										{meta.use_case}
									</p>
								)}
							</DetailSection>
							{classification && !isSpeech && (
								<ModelCapabilities
									classification={classification}
									unavailableFields={
										(
											displayBit.parameters as
												| { classification_unavailable_fields?: unknown }
												| undefined
										)?.classification_unavailable_fields
									}
								/>
							)}
						</div>

						<aside
							aria-label={t("pricingAndDetails", "Pricing and details")}
							className="flex min-w-0 flex-[1_1_17rem] flex-col gap-5"
						>
							<ModelPricePanel
								price={price}
								tier={priceTier}
								type={displayBit.type}
								size={deployment.kind === "local" ? size : undefined}
							/>
							<ModelFacts
								bit={displayBit}
								deployment={deployment.kind}
								size={size}
								installed={!!isInstalled.data}
							/>
							<ModelLinks bit={displayBit} />
						</aside>
					</div>
				</div>

				<footer className="flex flex-wrap items-center gap-x-3 gap-y-2 border-t bg-background px-6 py-3.5">
					<p className="m-0 flex min-w-0 flex-[1_1_14rem] flex-wrap items-baseline gap-x-2">
						<span
							className={cn(
								"font-mono text-[15px] font-semibold tabular-nums",
								summary.free && "text-emerald-600 dark:text-emerald-400",
							)}
						>
							{summary.main}
						</span>
						{summary.sub && (
							<span className="text-[12.5px] text-muted-foreground">
								{summary.sub}
							</span>
						)}
					</p>
					{onEdit && bit && (
						<Button variant="outline" onClick={() => onEdit(bit)}>
							<PencilIcon className="size-4" />
							{t("editModel", "Edit model")}
						</Button>
					)}
					{canDownload && (isInstalled.data || isInProfile) && (
						<Button
							variant="outline"
							onClick={handleDownload}
							disabled={progress !== undefined}
						>
							{isInstalled.data ? (
								<>
									<TrashIcon className="size-4" />
									{t("removeDownload", "Remove Download")}
								</>
							) : (
								<>
									<DownloadCloudIcon className="size-4" />
									{t("downloadWithSize", "Download ({{size}})", {
										size: size || humanFileSize(0),
									})}
								</>
							)}
						</Button>
					)}
					{isInProfile ? (
						<Button variant="outline" onClick={handleToggleProfile}>
							{t("removeFromProfile", "Remove from profile")}
						</Button>
					) : (
						<Button onClick={handleToggleProfile}>
							{canDownload && !isInstalled.data ? (
								<DownloadCloudIcon className="size-4" />
							) : (
								<PlusIcon className="size-4" />
							)}
							{canDownload && !isInstalled.data && size
								? t("addAndDownload", "Add to profile and download {{size}}", {
										size,
									})
								: t("addToProfile", "Add to Profile")}
						</Button>
					)}
				</footer>
			</DialogContent>
		</Dialog>
	);
}

function DetailSection({
	title,
	children,
}: Readonly<{ title: string; children: React.ReactNode }>) {
	return (
		<section className="space-y-2">
			<h3 className="text-sm font-semibold">{title}</h3>
			{children}
		</section>
	);
}

function ModelFacts({
	bit,
	deployment,
	size,
	installed,
}: Readonly<{
	bit: IBit;
	deployment: Deployment;
	size: string;
	installed: boolean;
}>) {
	const { t } = useTranslation("common");
	const [copied, setCopied] = useState(false);
	const deploymentLabel = {
		hosted: t("deploymentHosted", "Hosted"),
		local: t("deploymentOnDevice", "On this device"),
		remote: t("deploymentRemote", "Remote"),
		native: t("deploymentFlowRuntime", "Inside your flows"),
	}[deployment];
	const params = bit.parameters as
		| (ILlmParameters & IEmbeddingModelParameters)
		| undefined;
	const flow = bitModalities(bit.type);
	const modelId = params?.provider?.model_id || bit.id;
	const released = bit.model_evaluation?.release_date ?? bit.created;
	const releasedDate = released ? new Date(released) : null;

	const facts: { label: string; value: string }[] = [
		{
			label: t("detailInputOutput", "Input → output"),
			value: `${flow.inputs.map((m) => MODALITY[m].label).join(" + ")} → ${MODALITY[flow.output].label}`,
		},
	];
	if (params?.context_length)
		facts.push({
			label: t("detailContextWindow", "Context window"),
			value: t("detailTokens", "{{value}} tokens", {
				value: params.context_length.toLocaleString(),
			}),
		});
	if (params?.vector_length)
		facts.push({
			label: t("detailVectorLength", "Vector length"),
			value: params.vector_length.toLocaleString(),
		});
	if (params?.input_length)
		facts.push({
			label: t("detailMaxInput", "Max input"),
			value: t("detailTokens", "{{value}} tokens", {
				value: params.input_length.toLocaleString(),
			}),
		});
	facts.push({ label: t("detailRuns", "Runs"), value: deploymentLabel });
	if (size)
		facts.push({
			label: t("detailDownload", "Download"),
			value: installed ? `${size} · ${t("installed", "Installed")}` : size,
		});
	if (releasedDate && !Number.isNaN(releasedDate.getTime()))
		facts.push({
			label: t("detailReleased", "Released"),
			value: releasedDate.toLocaleDateString(undefined, {
				year: "numeric",
				month: "short",
			}),
		});
	if (bit.license)
		facts.push({ label: t("detailLicense", "License"), value: bit.license });
	if (bit.version)
		facts.push({ label: t("detailVersion", "Version"), value: bit.version });

	const copyId = () => {
		navigator.clipboard
			?.writeText(modelId)
			.then(() => {
				setCopied(true);
				setTimeout(() => setCopied(false), 1600);
			})
			.catch(() => toast.error(t("copyFailed", "Could not copy")));
	};

	return (
		<section className="space-y-2.5 px-1">
			<h3 className="text-[13px] font-semibold">{t("details", "Details")}</h3>
			<dl className="m-0 space-y-2 text-[12.5px]">
				{facts.map((fact) => (
					<div key={fact.label} className="flex justify-between gap-3">
						<dt className="whitespace-nowrap text-muted-foreground">
							{fact.label}
						</dt>
						<dd className="m-0 min-w-0 text-right">{fact.value}</dd>
					</div>
				))}
			</dl>
			<div className="flex items-center gap-1.5 rounded-lg border py-1 pr-1 pl-2.5">
				<span
					title={modelId}
					className="min-w-0 flex-1 truncate font-mono text-xs text-foreground/80"
				>
					{modelId}
				</span>
				<Button
					variant="ghost"
					size="sm"
					className="h-7 gap-1 px-2 text-xs text-muted-foreground"
					aria-label={t("copyModelId", "Copy model ID")}
					onClick={copyId}
				>
					{copied ? (
						<CheckIcon className="size-3.5" />
					) : (
						<CopyIcon className="size-3.5" />
					)}
					{copied ? t("copied", "Copied") : t("copy", "Copy")}
				</Button>
			</div>
		</section>
	);
}

function ModelLinks({ bit }: Readonly<{ bit: IBit }>) {
	const { t } = useTranslation("common");
	const meta = bit.meta.en;
	const links = [
		{ href: bit.repository, label: t("viewRepository", "View Repository") },
		{ href: meta?.docs_url, label: t("documentation", "Documentation") },
		{ href: meta?.website, label: t("website", "Website") },
	].filter((link): link is { href: string; label: string } =>
		Boolean(link.href && /^https?:\/\//i.test(link.href)),
	);
	const tags = meta?.tags ?? [];
	if (tags.length === 0 && links.length === 0) return null;
	return (
		<section className="space-y-2.5 px-1">
			{tags.length > 0 && (
				<>
					<h3 className="text-[13px] font-semibold">{t("tags", "Tags")}</h3>
					<div className="flex flex-wrap gap-1.5">
						{tags.map((tag) => (
							<span
								key={tag}
								className="rounded-md border px-2 py-0.5 font-mono text-[11.5px] text-foreground/80"
							>
								{tag}
							</span>
						))}
					</div>
				</>
			)}
			{links.length > 0 && (
				<div className="flex flex-wrap gap-x-4 gap-y-1.5 pt-1">
					{links.map((link) => (
						<a
							key={link.href}
							href={link.href}
							target="_blank"
							rel="noopener noreferrer"
							className="inline-flex items-center gap-1.5 text-[12.5px] font-medium text-muted-foreground hover:text-foreground"
						>
							<ExternalLinkIcon className="size-3.5" />
							{link.label}
						</a>
					))}
				</div>
			)}
		</section>
	);
}

function capBarColor(pct: number): string {
	if (pct >= 70) return "bg-emerald-500";
	if (pct >= 40) return "bg-amber-500";
	return "bg-red-500";
}

function capTextColor(pct: number): string {
	if (pct >= 70) return "text-emerald-600 dark:text-emerald-400";
	if (pct >= 40) return "text-amber-600 dark:text-amber-400";
	return "text-red-600 dark:text-red-400";
}

function ModelCapabilities({
	classification,
	unavailableFields,
}: Readonly<{
	classification: IBitModelClassification;
	unavailableFields?: unknown;
}>) {
	const { t } = useTranslation("common");
	const capabilities = Object.keys(classification)
		.flatMap((key) => {
			const value = getModelCapability(
				{
					model_classification: classification,
					classification_unavailable_fields: unavailableFields,
				},
				key,
			);
			return value === null ? [] : [[key, value] as const];
		})
		.sort((a, b) => b[1] - a[1]);

	if (capabilities.length === 0) return null;

	return (
		<DetailSection title={t("capabilities", "Capabilities")}>
			<div className="grid grid-cols-2 gap-x-4 gap-y-3">
				{capabilities.map(([key, value]) => {
					const { icon, label } = getCapabilityIcon(key);
					const pct = Math.round(value * 100);
					return (
						<div key={key} className="space-y-1">
							<div className="flex items-center justify-between text-xs">
								<span className="flex items-center gap-1 text-muted-foreground">
									<span className="text-xs">{icon}</span>
									<span>{label}</span>
								</span>
								<span
									className={`font-semibold tabular-nums ${capTextColor(pct)}`}
								>{`${pct}%`}</span>
							</div>
							<div className="h-1.5 w-full overflow-hidden rounded-full bg-muted">
								<div
									className={`h-full rounded-full transition-all ${capBarColor(pct)}`}
									style={{ width: `${pct}%` }}
								/>
							</div>
						</div>
					);
				})}
			</div>
		</DetailSection>
	);
}
