"use client";

import { contractDefaults } from "@flow-like/widget-sdk";
import {
	Blocks,
	MonitorPause,
	Play,
	Square,
	TriangleAlert,
} from "lucide-react";
import {
	Component,
	type ErrorInfo,
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import {
	formatWidgetContractSummary,
	microWidgetPreviewLru,
	summarizeWidgetContract,
} from "../../lib/package-widgets";
import { isTauri } from "../../lib/platform";
import type { PackageWidgetEntry } from "../../lib/schema/wasm";
import { cn } from "../../lib/utils";
import { A2UIMicroWidget } from "../a2ui/layout/A2UIMicroWidget";
import type {
	A2UIComponent,
	MicroWidgetInstanceComponent,
} from "../a2ui/types";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../ui/card";

const MAX_KEYWORD_CHIPS = 4;

type PausedReason = "lru" | "offscreen" | "error" | null;

export interface WidgetCardProps {
	/** Manifest widget entry — everything the static card needs. */
	widget: PackageWidgetEntry;
	/** Package id; required (with `packageVersion`) for live previews. */
	packageId?: string;
	/** Version shown by the surrounding page; used for web preview URLs. */
	packageVersion?: string;
	/** Widget bundle sha256 for desktop (`flow-widget://`) preview serving. */
	bundleHash?: string | null;
	className?: string;
}

interface PreviewErrorBoundaryProps {
	onError: () => void;
	children: ReactNode;
}

/** Degrades a crashed live preview back to the static card. */
class PreviewErrorBoundary extends Component<
	PreviewErrorBoundaryProps,
	{ hasError: boolean }
> {
	constructor(props: PreviewErrorBoundaryProps) {
		super(props);
		this.state = { hasError: false };
	}

	static getDerivedStateFromError() {
		return { hasError: true };
	}

	componentDidCatch(error: Error, info: ErrorInfo) {
		console.warn("[WidgetCard] preview crashed:", error, info.componentStack);
		this.props.onError();
	}

	render() {
		if (this.state.hasError) return null;
		return this.props.children;
	}
}

function WidgetThumbnail({
	widget,
	className,
}: {
	widget: PackageWidgetEntry;
	className?: string;
}) {
	const [failed, setFailed] = useState(false);
	const src = failed ? null : (widget.thumbnail ?? widget.icon ?? null);

	return (
		<div
			className={cn(
				"relative aspect-video w-full overflow-hidden rounded-md border bg-muted dark:border-white/15",
				className,
			)}
		>
			{src ? (
				<img
					src={src}
					alt=""
					className="h-full w-full object-cover"
					onError={() => setFailed(true)}
				/>
			) : (
				<div className="flex h-full w-full items-center justify-center">
					<Blocks className="h-10 w-10 text-muted-foreground/40" />
				</div>
			)}
		</div>
	);
}

function ContractSummaryBadges({ widget }: { widget: PackageWidgetEntry }) {
	const summary = useMemo(
		() => summarizeWidgetContract(widget.contract),
		[widget.contract],
	);
	const parts = useMemo(
		() => formatWidgetContractSummary(summary).split(" · "),
		[summary],
	);
	return (
		<div className="flex flex-wrap gap-1">
			{parts.map((part) => (
				<Badge key={part} variant="outline" className="text-xs font-normal">
					{part}
				</Badge>
			))}
		</div>
	);
}

function KeywordChips({ keywords }: { keywords: string[] }) {
	if (keywords.length === 0) return null;
	return (
		<div className="flex flex-wrap gap-1">
			{keywords.slice(0, MAX_KEYWORD_CHIPS).map((keyword) => (
				<Badge key={keyword} variant="secondary" className="text-xs">
					{keyword}
				</Badge>
			))}
			{keywords.length > MAX_KEYWORD_CHIPS && (
				<Badge variant="secondary" className="text-xs">
					+{keywords.length - MAX_KEYWORD_CHIPS}
				</Badge>
			)}
		</div>
	);
}

function PausedHint({ reason }: { reason: PausedReason }) {
	if (!reason) return null;
	const label =
		reason === "error"
			? "Preview unavailable — showing static card"
			: "Preview paused";
	const Icon = reason === "error" ? TriangleAlert : MonitorPause;
	return (
		<p className="flex items-center gap-1.5 text-xs text-muted-foreground">
			<Icon className="h-3.5 w-3.5 shrink-0" />
			{label}
		</p>
	);
}

function renderNoChild(): ReactNode {
	return null;
}

interface LivePreviewProps {
	component: MicroWidgetInstanceComponent;
	instanceId: string;
	onError: () => void;
	onOffscreen: () => void;
}

function LivePreview({
	component,
	instanceId,
	onError,
	onOffscreen,
}: LivePreviewProps) {
	const containerRef = useRef<HTMLDivElement>(null);

	// Iframe discipline: a live preview that scrolls fully out of view releases
	// its slot instead of keeping a hidden sandbox running.
	useEffect(() => {
		const node = containerRef.current;
		if (!node || typeof IntersectionObserver === "undefined") return;
		const observer = new IntersectionObserver(
			(entries) => {
				for (const entry of entries) {
					if (!entry.isIntersecting) onOffscreen();
				}
			},
			{ rootMargin: "256px" },
		);
		observer.observe(node);
		return () => observer.disconnect();
	}, [onOffscreen]);

	return (
		<div
			ref={containerRef}
			className="overflow-hidden rounded-md border bg-muted/40 dark:border-white/15"
		>
			<PreviewErrorBoundary onError={onError}>
				<A2UIMicroWidget
					component={component as A2UIComponent}
					componentId={instanceId}
					surfaceId="widget-card-preview"
					renderChild={renderNoChild}
				/>
			</PreviewErrorBoundary>
		</div>
	);
}

/**
 * Card for a package-manifest widget (§9.4): static variant renders name,
 * description, thumbnail, contract summary and keywords from the manifest
 * alone; the live variant mounts the real sandboxed `A2UIMicroWidget`
 * (preview mode, contract-default props) on explicit request, capped at
 * `MICRO_WIDGET_PREVIEW_LIMIT` concurrent instances via a shared LRU.
 * Reused by the store package detail, publish review and publication review.
 */
export function WidgetCard({
	widget,
	packageId,
	packageVersion,
	bundleHash,
	className,
}: WidgetCardProps) {
	const [live, setLive] = useState(false);
	const [pausedReason, setPausedReason] = useState<PausedReason>(null);

	const instanceId = useMemo(
		() => `preview-${packageId ?? "local"}-${widget.id}`,
		[packageId, widget.id],
	);

	// Desktop serves previews from the unpacked local bundle (needs the hash);
	// web serves from the registry widget-asset route (needs id + version).
	const canPreview = Boolean(
		packageId && packageVersion && (!isTauri() || bundleHash),
	);

	const previewComponent = useMemo<MicroWidgetInstanceComponent>(
		() => ({
			id: instanceId,
			type: "microWidgetInstance",
			instanceId,
			packageId: packageId ?? "",
			widgetId: widget.id,
			packageVersion: packageVersion ?? "",
			bundleHash: bundleHash ?? undefined,
			contract: widget.contract,
			props: contractDefaults(widget.contract),
			preview: true,
		}),
		[instanceId, packageId, packageVersion, bundleHash, widget],
	);

	const pausePreview = useCallback(
		(reason: PausedReason) => {
			microWidgetPreviewLru.release(instanceId);
			setLive(false);
			setPausedReason(reason);
		},
		[instanceId],
	);

	const startPreview = useCallback(() => {
		microWidgetPreviewLru.activate(instanceId, () => {
			setLive(false);
			setPausedReason("lru");
		});
		setPausedReason(null);
		setLive(true);
	}, [instanceId]);

	const togglePreview = useCallback(() => {
		if (live) {
			pausePreview(null);
		} else {
			startPreview();
		}
	}, [live, pausePreview, startPreview]);

	const handlePreviewError = useCallback(
		() => pausePreview("error"),
		[pausePreview],
	);
	const handleOffscreen = useCallback(
		() => pausePreview("offscreen"),
		[pausePreview],
	);

	useEffect(() => {
		return () => microWidgetPreviewLru.release(instanceId);
	}, [instanceId]);

	return (
		<Card className={cn("gap-3", className)}>
			<CardHeader className="pb-0">
				<div className="flex items-start justify-between gap-2">
					<div className="min-w-0">
						<CardTitle className="truncate text-sm font-medium">
							{widget.name}
						</CardTitle>
						{widget.description && (
							<CardDescription className="mt-1 line-clamp-2 text-xs">
								{widget.description}
							</CardDescription>
						)}
					</div>
					{canPreview && (
						<Button
							variant={live ? "secondary" : "outline"}
							size="sm"
							className="h-7 shrink-0 gap-1.5 px-2 text-xs"
							onClick={togglePreview}
						>
							{live ? (
								<>
									<Square className="h-3 w-3" />
									Stop
								</>
							) : (
								<>
									<Play className="h-3 w-3" />
									Preview
								</>
							)}
						</Button>
					)}
				</div>
			</CardHeader>
			<CardContent className="space-y-3">
				{live ? (
					<LivePreview
						component={previewComponent}
						instanceId={instanceId}
						onError={handlePreviewError}
						onOffscreen={handleOffscreen}
					/>
				) : (
					<WidgetThumbnail widget={widget} />
				)}
				{!live && <PausedHint reason={pausedReason} />}
				<ContractSummaryBadges widget={widget} />
				<KeywordChips keywords={widget.keywords ?? []} />
			</CardContent>
		</Card>
	);
}

export interface WidgetCardGridProps {
	widgets: PackageWidgetEntry[];
	packageId?: string;
	packageVersion?: string;
	bundleHash?: string | null;
	className?: string;
}

/** Responsive grid of `WidgetCard`s, symmetric to the nodes tab grid. */
export function WidgetCardGrid({
	widgets,
	packageId,
	packageVersion,
	bundleHash,
	className,
}: WidgetCardGridProps) {
	return (
		<div
			className={cn(
				"grid grid-cols-1 gap-4 md:grid-cols-2 lg:grid-cols-3",
				className,
			)}
		>
			{widgets.map((widget) => (
				<WidgetCard
					key={widget.id}
					widget={widget}
					packageId={packageId}
					packageVersion={packageVersion}
					bundleHash={bundleHash}
				/>
			))}
		</div>
	);
}
