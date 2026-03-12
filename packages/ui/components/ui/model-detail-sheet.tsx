"use client";
import type { UseQueryResult } from "@tanstack/react-query";
import {
	CheckIcon,
	ClockIcon,
	DownloadCloudIcon,
	ExternalLinkIcon,
	PlusIcon,
	SparklesIcon,
	TrashIcon,
	XIcon,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useHub } from "../../hooks/use-hub";
import { useInvoke } from "../../hooks/use-invoke";
import type { IBit } from "../../lib/schema/bit/bit";
import type { IEmbeddingModelParameters } from "../../lib/schema/bit/bit/embedding-model-parameters";
import type {
	IBitModelClassification,
	ILlmParameters,
} from "../../lib/schema/bit/bit/llm-parameters";
import { humanFileSize } from "../../lib/utils";
import { useBackend } from "../../state/backend-state";
import { useDownloadManager } from "../../state/download-manager";
import type { ISettingsProfile } from "../../types";
import { Avatar, AvatarFallback, AvatarImage } from "./avatar";
import { Badge } from "./badge";
import { Button } from "./button";
import {
	ModalityIcons,
	ModelTypeIcon,
	formatContextLength,
	getCapabilityIcon,
	isEmbeddingBit,
	supportsRemoteEmbeddingExecution,
} from "./model-card";
import type { IModelEvaluation } from "./model-benchmarks";
import { ModelBenchmarks } from "./model-benchmarks";
import { Progress } from "./progress";
import {
	Sheet,
	SheetClose,
	SheetContent,
	SheetDescription,
	SheetHeader,
	SheetTitle,
} from "./sheet";

export interface ModelDetailSheetProps {
	bit: IBit | null;
	open: boolean;
	onOpenChange: (open: boolean) => void;
	webMode?: boolean;
}

export function ModelDetailSheet({
	bit,
	open,
	onOpenChange,
	webMode = false,
}: Readonly<ModelDetailSheetProps>) {
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
		if (!bit) return;
		mountedRef.current = true;
		const bitHash = bit.hash;
		const initial = getLatestPct(bitHash);
		if (typeof initial === "number") {
			setProgress(initial);
			lastPctRef.current = initial;
		} else if (isQueued(bitHash)) {
			setProgress(0);
			lastPctRef.current = 0;
		}

		unsubscribeRef.current = onProgress(bitHash, (dl) => {
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
	}, [bit, getLatestPct, isQueued, onProgress]);

	const isInstalled: UseQueryResult<boolean> = useInvoke(
		backend.bitState.isBitInstalled,
		backend.bitState,
		// biome-ignore lint/style/noNonNullAssertion: bit is guaranteed by enabled flag
		[bit!],
		!!bit,
	);
	const bitSize: UseQueryResult<number> = useInvoke(
		backend.bitState.getBitSize,
		backend.bitState,
		// biome-ignore lint/style/noNonNullAssertion: bit is guaranteed by enabled flag
		[bit!],
		!!bit,
	);
	const currentProfile: UseQueryResult<ISettingsProfile> = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const detailedBit = useInvoke(
		backend.bitState.getBit,
		backend.bitState,
		[bit?.id ?? "", bit?.hub],
		!!bit && open,
		[bit?.updated ?? ""],
		60_000,
	);
	const userInfo = useInvoke(backend.userState.getInfo, backend.userState, []);
	const displayBit = detailedBit.data ?? bit;

	const isVirtualBit = useMemo(
		() => !displayBit?.download_link || (bitSize.data === 0 && bitSize.isSuccess),
		[displayBit?.download_link, bitSize.data, bitSize.isSuccess],
	);

	const tierInfo = useMemo(() => {
		if (!displayBit) return { isRestricted: false, requiredTier: null };
		const params = displayBit.parameters as {
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
	}, [displayBit, hub?.tiers, userInfo.data?.tier]);

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
	const handleDownload = useCallback(async () => {
		if (!displayBit) return;
		if (isInstalled.data) {
			await backend.bitState.deleteBit(displayBit);
			await refetchIsInstalled();
			return;
		}
		await downloadBit(displayBit);
	}, [
		displayBit,
		isInstalled.data,
		backend.bitState,
		downloadBit,
		refetchIsInstalled,
	]);

	const refetchCurrentProfile = currentProfile.refetch;
	const handleToggleProfile = useCallback(async () => {
		if (!displayBit) return;
		const profile = currentProfile.data;
		if (!profile) return;
		const bitIndex = profile.hub_profile.bits.findIndex(
			(id) => id.split(":").pop() === displayBit.id,
		);
		if (bitIndex === -1) {
			await downloadBit(displayBit);
			await backend.bitState.addBit(displayBit, profile);
		} else {
			await backend.bitState.removeBit(displayBit, profile);
		}
		await refetchCurrentProfile();
	}, [
		displayBit,
		currentProfile.data,
		downloadBit,
		backend.bitState,
		refetchCurrentProfile,
	]);

	if (!displayBit || !displayBit.meta.en) return null;

	const meta = displayBit.meta.en;
	const isInProfile =
		(currentProfile.data?.hub_profile.bits || []).findIndex(
			(id) => id.split(":")[1] === displayBit.id,
		) > -1;

	const params = displayBit.parameters as
		| ILlmParameters
		| IEmbeddingModelParameters;
	const classification = (params as ILlmParameters)?.model_classification;
	const contextLength = (params as ILlmParameters)?.context_length;
	const embeddingParams = params as IEmbeddingModelParameters;
	const isHosted = bitSize.data === 0 || isVirtualBit;
	const canRunRemotely = supportsRemoteEmbeddingExecution(displayBit);
	const isEmbeddingModel = isEmbeddingBit(displayBit);

	return (
		<Sheet open={open} onOpenChange={onOpenChange}>
			<SheetContent className="overflow-y-auto px-4 md:min-w-2/5">
				<SheetHeader className="pb-4">
					<div className="flex items-start gap-3">
						<Avatar className="h-12 w-12 border">
							<AvatarImage src={meta.icon ?? "/app-logo.webp"} />
							<AvatarFallback>
								<ModelTypeIcon type={displayBit.type} className="h-5 w-5" />
							</AvatarFallback>
						</Avatar>
						<div className="flex-1 min-w-0">
							<SheetTitle className="flex items-center gap-2 text-lg">
								{meta.name}
								{isInProfile && (
									<SparklesIcon className="h-4 w-4 text-primary" />
								)}
							</SheetTitle>
							<SheetDescription>
								<ModalityIcons type={displayBit.type} />
							</SheetDescription>
						</div>
					</div>
					<SheetClose />
				</SheetHeader>

				<div className="space-y-6">
					{/* Download Progress */}
					{progress !== undefined && !isVirtualBit && (
						<div className="flex items-center gap-3 p-3 rounded-lg bg-muted/50">
							{isQueuedState ? (
								<>
									<ClockIcon className="h-4 w-4 text-primary animate-pulse" />
									<span className="text-sm">Queued for download...</span>
								</>
							) : (
								<>
									<Progress value={progress} className="flex-1 h-2" />
									<span className="text-sm tabular-nums w-12 text-right">
										{progress}%
									</span>
								</>
							)}
						</div>
					)}

					{/* Status Badges */}
					<div className="flex flex-wrap gap-2">
						{isHosted ? (
							<Badge className="bg-sky-500/10 text-sky-600 border-sky-500/30">
								Hosted
							</Badge>
						) : isInstalled.data ? (
							<Badge className="bg-emerald-500/10 text-emerald-600 border-emerald-500/30">
								<CheckIcon className="h-3 w-3 mr-1" />
								Installed
							</Badge>
						) : (
							<Badge variant="outline">
								<DownloadCloudIcon className="h-3 w-3 mr-1" />
								{humanFileSize(bitSize.data ?? 0)}
							</Badge>
						)}
						{contextLength && (
							<Badge variant="outline">
								{formatContextLength(contextLength)}
							</Badge>
						)}
						{canRunRemotely && (
							<Badge className="bg-cyan-500/10 text-cyan-700 border-cyan-500/30">
								Remote
							</Badge>
						)}
						{isEmbeddingModel && !canRunRemotely && (
							<Badge className="bg-zinc-500/10 text-zinc-600 border-zinc-500/30">
								Local only
							</Badge>
						)}
						{tierInfo.isRestricted && tierInfo.requiredTier && (
							<Badge className="bg-amber-500/10 text-amber-600 border-amber-500/30">
								{tierInfo.requiredTier} Required
							</Badge>
						)}
					</div>

					{/* Description */}
					<div>
						<h4 className="text-sm font-medium mb-2">Description</h4>
						<p className="text-sm text-muted-foreground">{meta.description}</p>
					</div>

					{/* Capabilities */}
					{classification && (
						<ModelCapabilities classification={classification} />
					)}

					{/* Benchmarks & Evaluation */}
					{displayBit.model_evaluation && (
						<ModelBenchmarks
							evaluation={displayBit.model_evaluation as IModelEvaluation}
						/>
					)}

					{/* Embedding Parameters */}
					{embeddingParams?.vector_length && (
						<div>
							<h4 className="text-sm font-medium mb-2">Embedding Details</h4>
							<div className="grid grid-cols-2 gap-2 text-sm">
								<div className="flex justify-between p-2 rounded bg-muted/50">
									<span className="text-muted-foreground">Vector Length</span>
									<span>{embeddingParams.vector_length}</span>
								</div>
								{embeddingParams.input_length && (
									<div className="flex justify-between p-2 rounded bg-muted/50">
										<span className="text-muted-foreground">Max Input</span>
										<span>{embeddingParams.input_length}</span>
									</div>
								)}
							</div>
						</div>
					)}

					{/* Tags */}
					{meta.tags.length > 0 && (
						<div>
							<h4 className="text-sm font-medium mb-2">Tags</h4>
							<div className="flex flex-wrap gap-1.5">
								{meta.tags.map((tag) => (
									<Badge key={tag} variant="outline" className="text-xs">
										{tag}
									</Badge>
								))}
							</div>
						</div>
					)}

					{/* Actions */}
					<div className="flex flex-col gap-2 pt-2">
						{!webMode && !isHosted && (
							<Button
								onClick={handleDownload}
								variant={isInstalled.data ? "destructive" : "default"}
								className="w-full"
								disabled={progress !== undefined}
							>
								{isInstalled.data ? (
									<>
										<TrashIcon className="h-4 w-4 mr-2" />
										Remove Download
									</>
								) : (
									<>
										<DownloadCloudIcon className="h-4 w-4 mr-2" />
										Download ({humanFileSize(bitSize.data ?? 0)})
									</>
								)}
							</Button>
						)}
						<Button
							onClick={handleToggleProfile}
							variant="outline"
							className="w-full"
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
						</Button>
						{displayBit.repository && (
							<Button
								variant="ghost"
								className="w-full"
								onClick={() =>
									window.open(displayBit.repository ?? "", "_blank")
								}
							>
								<ExternalLinkIcon className="h-4 w-4 mr-2" />
								View Repository
							</Button>
						)}
					</div>
				</div>
			</SheetContent>
		</Sheet>
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
}: Readonly<{ classification: IBitModelClassification }>) {
	const capabilities = (
		Object.entries(classification).filter(
			([_, value]) => typeof value === "number" && value > 0,
		) as [string, number][]
	).sort((a, b) => b[1] - a[1]);

	if (capabilities.length === 0) return null;

	return (
		<div>
			<h4 className="text-sm font-medium mb-3">Capabilities</h4>
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
								>
									{pct}%
								</span>
							</div>
							<div className="h-1.5 w-full rounded-full bg-muted overflow-hidden">
								<div
									className={`h-full rounded-full transition-all ${capBarColor(pct)}`}
									style={{ width: `${pct}%` }}
								/>
							</div>
						</div>
					);
				})}
			</div>
		</div>
	);
}
