"use client";
import type { UseQueryResult } from "@tanstack/react-query";
import {
	CameraIcon,
	ClockIcon,
	DownloadCloudIcon,
	ExternalLinkIcon,
	FileIcon,
	FileSearch,
	ImageIcon,
	LockIcon,
	MessagesSquareIcon,
	MinusIcon,
	MoreVerticalIcon,
	Package2Icon,
	PackageCheckIcon,
	PlusIcon,
	ScanEyeIcon,
	SparklesIcon,
	TrashIcon,
	UniversityIcon,
	WorkflowIcon,
} from "lucide-react";
import {
	type JSX,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { Progress } from "../../components/ui/progress";
import { useHub } from "../../hooks/use-hub";
import { useInvoke } from "../../hooks/use-invoke";
import { type IBit, IBitTypes } from "../../lib/schema/bit/bit";
import { humanFileSize } from "../../lib/utils";
import { useBackend } from "../../state/backend-state";
import { useDownloadManager } from "../../state/download-manager";
import type { ISettingsProfile } from "../../types";
import { Avatar, AvatarFallback, AvatarImage } from "./avatar";
import { Badge } from "./badge";
import { Button } from "./button";
import { Card, CardContent, CardHeader } from "./card";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "./dropdown-menu";

export function BitCard({
	bit,
	wide = false,
	subscriptionsPath = "/subscription",
}: Readonly<{ bit: IBit; wide: boolean; subscriptionsPath?: string }>) {
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

	// Check if the model requires a higher tier than the user has
	// Model tier is stored in bit.parameters.provider.params.tier (e.g., "FREE", "PREMIUM", "PRO", "ENTERPRISE")
	// User tier is stored in userInfo.data.tier (e.g., "FREE", "PREMIUM", "PRO", "ENTERPRISE")
	// Hub tiers config shows which llm_tiers each user tier can access
	const tierInfo = useMemo(() => {
		const params = bit.parameters as {
			provider?: { params?: { tier?: string } };
		};
		const modelTier = params?.provider?.params?.tier;

		// No tier restriction for local models (no tier specified) or if hub config not loaded
		if (!modelTier || !hub?.tiers) {
			return { isRestricted: false, requiredTier: null };
		}

		// Default to FREE tier if user is not logged in or tier not available
		const userTierKey = (userInfo.data?.tier ?? "FREE").toUpperCase();
		const userTierConfig = hub.tiers[userTierKey];

		// If user tier config not found, assume restricted
		if (!userTierConfig) {
			return { isRestricted: true, requiredTier: modelTier };
		}

		const allowedModelTiers = userTierConfig.llm_tiers ?? [];
		const isRestricted = !allowedModelTiers.includes(modelTier);

		return { isRestricted, requiredTier: isRestricted ? modelTier : null };
	}, [bit.parameters, hub?.tiers, userInfo.data?.tier]);

	// A bit is considered "virtual" if it has no download link OR its size resolves to 0.
	// These represent hosted / proxied models (no local artifact). We immediately treat
	// a download request as a no-op success so the UI does not show a perpetual "Queued" state.
	const isVirtualBit = useMemo(
		() => !bit.download_link || (bitSize.data === 0 && bitSize.isSuccess),
		[bit.download_link, bitSize.data, bitSize.isSuccess],
	);

	const downloadBit = useCallback(
		async (b: IBit) => {
			// Short‑circuit for virtual bits: no progress overlay, just refresh install state.
			if (!b.download_link || isVirtualBit) {
				await isInstalled.refetch();
				return; // success path
			}
			setProgress(0);
			try {
				await download(b); // de-duplicated in manager
				await isInstalled.refetch();
			} finally {
				// keep subscription; overlay hides when done/unmounted
				if (mountedRef.current) {
					setProgress(undefined);
					lastPctRef.current = 0;
					lastUpdateRef.current = 0;
				}
			}
		},
		[download, isInstalled, isVirtualBit],
	);

	const toggleDownload = useCallback(async () => {
		if (isInstalled.data) {
			await backend.bitState.deleteBit(bit);
			await isInstalled.refetch();
			return;
		}
		await downloadBit(bit);
	}, [isInstalled.data, backend.bitState, bit, downloadBit]);

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
		await currentProfile.refetch();
	}, [currentProfile.data, bit, downloadBit, backend.bitState]);

	const openRepository = useCallback(() => {
		if (bit.repository) {
			window.open(bit.repository, "_blank");
		}
	}, [bit.repository]);

	if (bit.meta.en === undefined) return null;

	const isInProfile =
		(currentProfile.data?.hub_profile.bits || []).findIndex(
			(id) => id.split(":").pop() === bit.id,
		) > -1;

	const isProfileCompatible = [
		"Embedding",
		"ImageEmbedding",
		"Vlm",
		"Llm",
	].includes(bit.type);

	return (
		<div
			className={`${wide ? "md:col-span-2" : ""} group/card`}
			data-testid="box"
		>
			<Card
				className={`
        relative h-full w-full cursor-pointer transition-all duration-300 ease-out
        hover:shadow-2xl hover:shadow-primary/10 hover:-translate-y-2
        ${isInstalled.data ? "ring-1 ring-emerald-500/20 shadow-emerald-500/10" : ""}
        ${isInProfile ? "bg-linear-to-br from-primary/5 to-emerald-500/5" : ""}
        overflow-hidden border hover:border-primary/30
        backdrop-blur-xs bg-card/80
      `}
			>
				{progress !== undefined && !isVirtualBit && (
					<div className="absolute inset-0 bg-background/80 backdrop-blur-xs z-30 flex items-center justify-center">
						{isQueuedState ? (
							<div className="text-center space-y-2">
								<div className="flex items-center gap-2 justify-center">
									<ClockIcon className="h-5 w-5 text-primary animate-pulse" />
									<p className="text-sm text-muted-foreground">Queued…</p>
								</div>
								<p className="text-xs text-muted-foreground">
									Waiting for available download slot
								</p>
							</div>
						) : (
							<div className="text-center space-y-4">
								<Progress value={progress} className="w-48" />
								<div className="flex items-center gap-2">
									<DownloadCloudIcon className="h-6 w-6 text-primary animate-pulse" />
									<p className="text-sm text-muted-foreground">
										{Math.round(progress)}% Downloaded
									</p>
								</div>
							</div>
						)}
					</div>
				)}

				<div className="absolute top-3 right-3 z-20">
					<DropdownMenu>
						<DropdownMenuTrigger asChild>
							<Button
								size="sm"
								variant="ghost"
								className="h-8 w-8 p-0 opacity-60 hover:opacity-100 transition-opacity"
								onClick={(e) => e.stopPropagation()}
							>
								<MoreVerticalIcon className="h-4 w-4" />
							</Button>
						</DropdownMenuTrigger>
						<DropdownMenuContent align="end" className="w-48">
							<DropdownMenuItem
								onClick={(e) => {
									e.stopPropagation();
									toggleDownload();
								}}
								className="flex items-center gap-2"
							>
								{isInstalled.data ? (
									<>
										<TrashIcon className="h-4 w-4" />
										Remove Download
									</>
								) : (
									<>
										<DownloadCloudIcon className="h-4 w-4" />
										Download ({humanFileSize(bitSize.data ?? 0)})
									</>
								)}
							</DropdownMenuItem>

							{isProfileCompatible && (
								<DropdownMenuItem
									onClick={(e) => {
										e.stopPropagation();
										toggleProfile();
									}}
									className="flex items-center gap-2"
								>
									{isInProfile ? (
										<>
											<MinusIcon className="h-4 w-4" />
											Remove from Profile
										</>
									) : (
										<>
											<PlusIcon className="h-4 w-4" />
											Add to Profile
										</>
									)}
								</DropdownMenuItem>
							)}

							{bit.repository && (
								<>
									<DropdownMenuSeparator />
									<DropdownMenuItem
										onClick={(e) => {
											e.stopPropagation();
											openRepository();
										}}
										className="flex items-center gap-2"
									>
										<ExternalLinkIcon className="h-4 w-4" />
										View Repository
									</DropdownMenuItem>
								</>
							)}
						</DropdownMenuContent>
					</DropdownMenu>
				</div>

				<CardHeader className="pb-3">
					<div className="flex items-center space-x-4 mb-4 pr-10">
						<div className="relative">
							<Avatar className="h-14 w-14 border-2 border-background shadow-lg ring-2 ring-primary/10 transition-all duration-300 group-hover/card:ring-primary/30">
								<AvatarImage
									src={bit.meta?.en?.icon ?? "/app-logo.webp"}
									className="transition-transform duration-300 group-hover/card:scale-110"
								/>
								<AvatarFallback className="bg-linear-to-br from-primary/20 to-secondary/20">
									<BitTypeIcon type={bit.type} className="h-6 w-6" />
								</AvatarFallback>
							</Avatar>

							<div className="absolute -bottom-1 -right-1 bg-primary text-primary-foreground rounded-full p-1.5 shadow-lg">
								<BitTypeIcon type={bit.type} className="h-3 w-3" />
							</div>
						</div>
						<div className="flex-1 min-w-0">
							<div className="flex items-center space-x-2 mb-1">
								<h3 className="font-semibold text-lg truncate group-hover/card:text-primary transition-colors">
									{bit.meta.en.name}
								</h3>
								{isInProfile && (
									<SparklesIcon className="h-4 w-4 text-primary animate-pulse" />
								)}
								{isQueued(bit.hash) && !isVirtualBit && (
									<Badge variant="outline" className="text-xs">
										<ClockIcon className="h-3 w-3 mr-1" />
										Queued
									</Badge>
								)}
								{bit.repository?.startsWith("https://huggingface.co/") && (
									<img
										src="/hf-logo.png"
										width={20}
										height={20}
										alt="Hugging Face"
										className="opacity-70 hover:opacity-100 transition-opacity ml-1"
									/>
								)}
							</div>

							<div className="flex items-center space-x-2">
								<Badge
									variant="secondary"
									className="text-xs font-medium text-secondary-foreground"
								>
									<BitTypeIcon
										type={bit.type}
										className="h-3 w-3 mr-1 text-secondary-foreground"
									/>
									{bitTypeToText(bit.type)}
								</Badge>

								<Badge
									variant={isInstalled.data ? "default" : "outline"}
									className={`text-xs font-medium transition-all ${
										isInstalled.data || bitSize.data === 0
											? "bg-emerald-500 hover:bg-emerald-600 text-white"
											: "hover:bg-primary/10"
									}`}
								>
									{isInstalled.data || bitSize.data === 0 ? (
										<PackageCheckIcon className="h-3 w-3 mr-1" />
									) : (
										<DownloadCloudIcon className="h-3 w-3 mr-1" />
									)}
									{bitSize.data === 0
										? "Hosted"
										: humanFileSize(bitSize.data ?? 0)}
								</Badge>

								{tierInfo.isRestricted && tierInfo.requiredTier && (
									<a
										href={subscriptionsPath}
										onClick={(e) => e.stopPropagation()}
										className="inline-flex"
									>
										<Badge
											variant="outline"
											className="text-xs font-medium bg-amber-500/10 text-amber-600 border-amber-500/30 hover:bg-amber-500/20 transition-colors cursor-pointer"
										>
											<LockIcon className="h-3 w-3 mr-1" />
											{tierInfo.requiredTier}
										</Badge>
									</a>
								)}
							</div>
						</div>
					</div>
				</CardHeader>

				<CardContent className="pt-0 flex flex-col flex-1">
					<div className="h-16 mb-4 flex items-center">
						<p className="text-muted-foreground text-sm leading-relaxed line-clamp-3 text-start">
							{bit.meta.en.description}
						</p>
					</div>

					<div className="flex flex-nowrap gap-2 mt-auto">
						{bit.meta.en.tags.slice(0, 3).map((tag) => (
							<Badge
								key={tag}
								variant="outline"
								className="text-xs hover:bg-primary/10 transition-colors whitespace-nowrap"
							>
								{tag}
							</Badge>
						))}
						{bit.meta.en.tags.length > 3 && (
							<Badge variant="outline" className="text-xs">
								+{bit.meta.en.tags.length - 3}
							</Badge>
						)}
					</div>
				</CardContent>

				<div className="absolute inset-0 bg-linear-to-br from-transparent via-transparent to-primary/5 opacity-0 group-hover/card:opacity-100 transition-opacity duration-300 pointer-events-none" />
			</Card>
		</div>
	);
}

export function BitTypeIcon({
	type,
	className,
}: Readonly<{ type: IBitTypes; className?: string }>): JSX.Element | null {
	const combinedClass =
		"min-h-4 min-w-4 h-4 w-4 text-foreground" +
		(className ? ` ${className}` : "");

	switch (type) {
		case IBitTypes.Llm:
			return <MessagesSquareIcon className={combinedClass} />;
		case IBitTypes.Vlm:
			return <CameraIcon className={combinedClass} />;
		case IBitTypes.Embedding:
			return <FileSearch className={combinedClass} />;
		case IBitTypes.ImageEmbedding:
			return <ScanEyeIcon className={combinedClass} />;
		case IBitTypes.File:
			return <FileIcon className={combinedClass} />;
		case IBitTypes.Media:
			return <ImageIcon className={combinedClass} />;
		case IBitTypes.Template:
			return <WorkflowIcon className={combinedClass} />;
		case IBitTypes.Course:
			return <UniversityIcon className={combinedClass} />;
	}
	return <Package2Icon className={combinedClass} />;
}

export function bitTypeToText(bitType: IBitTypes): string {
	return bitType.replace(/([A-Z])/g, (match, letter, index) =>
		index === 0 ? letter : ` ${letter}`,
	);
}
