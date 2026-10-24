"use client";

import { useTranslation } from "@flow-like/locales";
import type { TFunction } from "i18next";
import {
	ArrowLeft,
	ArrowUpRight,
	ChevronDown,
	CircleCheck,
	CircleHelp,
	CircleX,
	ClipboardCopy,
	Cpu,
	Download,
	Globe,
	Gpu,
	HardDrive,
	type LucideIcon,
	MemoryStick,
	Monitor,
	Package,
	Puzzle,
	RefreshCw,
	UserRound,
	Zap,
} from "lucide-react";
import Link from "next/link";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { toast } from "sonner";
import {
	type BoardRenderer,
	useBoardRenderer,
} from "../../../hooks/use-board-renderer";
import { useDeveloperMode } from "../../../hooks/use-developer-mode";
import { useInvoke } from "../../../hooks/use-invoke";
import { useWebGlSupport } from "../../../hooks/use-webgl-support";
import { CAPABILITY_LABELS } from "../../../lib/diagnostics/capabilities";
import {
	describeArchitecture,
	describeBrowser,
	describeOs,
	formatDiagnosticsReport,
	formatSize,
	formatWebGpuLimit,
	shortCommit,
	storageUsagePercent,
} from "../../../lib/diagnostics/format";
import { gpuDisplayName } from "../../../lib/diagnostics/graphics";
import {
	type DiagnosticsContext,
	collectDiagnostics,
	withDiagnosticsContext,
} from "../../../lib/diagnostics/probes";
import {
	type DiagnosticsBrowser,
	type DiagnosticsCapability,
	type DiagnosticsCapabilityId,
	type DiagnosticsGraphics,
	type DiagnosticsHardware,
	type DiagnosticsReport,
	type DiagnosticsSession,
	type DiagnosticsStorage,
	type DiagnosticsWebGl,
	type DiagnosticsWebGpu,
	NOTABLE_WEBGPU_FEATURES,
	WEBGPU_LIMIT_NAMES,
	type WebGpuStatus,
} from "../../../lib/diagnostics/types";
import { cn, humanFileSize } from "../../../lib/utils";
import {
	useAuthStatusStore,
	useBackend,
	useBackendReady,
} from "../../../state/backend-state";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import {
	Card,
	CardAction,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../../ui/card";
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from "../../ui/collapsible";
import { Progress } from "../../ui/progress";
import { Skeleton } from "../../ui/skeleton";

type SettingsT = TFunction<"settings">;

export interface DiagnosticsPageProps {
	readonly build: DiagnosticsContext["build"];
	readonly apiBaseUrl?: string | null;
	readonly storageHref?: string;
	readonly backHref?: string;
}

const WEBGPU_TONE: Readonly<Record<WebGpuStatus, string>> = {
	available: "border-primary/30 bg-primary/10 text-primary",
	unsupported: "border-border bg-muted text-muted-foreground",
	"no-adapter": "border-border bg-muted text-muted-foreground",
	timeout: "border-destructive/30 bg-destructive/10 text-destructive",
	error: "border-destructive/30 bg-destructive/10 text-destructive",
};

const LOADING_CARDS = ["build", "session", "browser", "hardware"] as const;
const LOW_MEMORY_GB = 4;

export function DiagnosticsPage({
	build,
	apiBaseUrl,
	storageHref,
	backHref = "/settings",
}: DiagnosticsPageProps) {
	const { t } = useTranslation("settings");
	const { probes, running, run } = useDiagnosticsProbes();
	const session = useDiagnosticsSession();
	const report = useMemo(
		() =>
			probes
				? withDiagnosticsContext(probes, { build, apiBaseUrl, session })
				: null,
		[probes, build, apiBaseUrl, session],
	);

	const copyReport = useCallback(async () => {
		if (!report) return;
		try {
			await navigator.clipboard.writeText(formatDiagnosticsReport(report));
			toast.success(
				t("diagnosticsReportCopied", "Report copied to the clipboard"),
			);
		} catch (error) {
			toast.error(t("diagnosticsCopyFailed", "Could not copy the report"), {
				description: t(
					"diagnosticsCopyFailedHint",
					"Download the JSON instead and attach it to your message.",
				),
			});
			console.warn("[diagnostics] clipboard write failed", error);
		}
	}, [report, t]);

	const downloadReport = useCallback(() => {
		if (report) downloadJson(report);
	}, [report]);

	return (
		<div className="h-full min-h-0 flex-1 overflow-auto px-1 pb-8">
			<div className="mx-auto max-w-screen-2xl space-y-5">
				<DiagnosticsHeader
					backHref={backHref}
					ready={report !== null}
					running={running}
					onCopy={copyReport}
					onDownload={downloadReport}
					onRerun={run}
				/>
				{report ? (
					<DiagnosticsBody report={report} storageHref={storageHref} />
				) : (
					<DiagnosticsLoading />
				)}
			</div>
		</div>
	);
}

function useDiagnosticsProbes() {
	const [probes, setProbes] = useState<DiagnosticsReport | null>(null);
	const [running, setRunning] = useState(true);
	const latest = useRef(0);

	const run = useCallback(() => {
		const id = ++latest.current;
		setRunning(true);
		void collectDiagnostics().then(
			(next) => {
				if (latest.current !== id) return;
				setProbes(next);
				setRunning(false);
			},
			() => {
				if (latest.current === id) setRunning(false);
			},
		);
	}, []);

	useEffect(() => {
		run();
		return () => {
			latest.current += 1;
		};
	}, [run]);

	return { probes, running, run };
}

function activeRenderer(
	preferred: BoardRenderer,
	webGl2: boolean,
): BoardRenderer {
	return preferred === "egui" && !webGl2 ? "react-flow" : preferred;
}

function useDiagnosticsSession(): DiagnosticsSession {
	const { i18n } = useTranslation("settings");
	const backend = useBackend();
	const backendReady = useBackendReady();
	const signedInState = useAuthStatusStore((state) => state.signedIn);
	const signedIn = signedInState !== false;
	const info = useInvoke(
		backend.userState.getInfo,
		backend.userState,
		[],
		backendReady && signedIn,
	);
	const { developerMode } = useDeveloperMode();
	const { renderer, ready } = useBoardRenderer();
	const webGl2 = useWebGlSupport("webgl2");
	const accountId = signedIn ? (info.data?.id ?? null) : null;
	const language = i18n.resolvedLanguage ?? i18n.language ?? null;

	return useMemo(
		() => ({
			signedIn: signedInState ?? (accountId ? true : null),
			accountId,
			developerMode,
			language,
			boardRendererPreference: ready ? renderer : null,
			boardRendererInUse:
				ready && webGl2 !== null ? activeRenderer(renderer, webGl2) : null,
		}),
		[
			signedInState,
			accountId,
			developerMode,
			language,
			ready,
			renderer,
			webGl2,
		],
	);
}

function downloadJson(report: DiagnosticsReport) {
	const blob = new Blob([JSON.stringify(report, null, 2)], {
		type: "application/json",
	});
	const url = URL.createObjectURL(blob);
	const link = document.createElement("a");
	link.href = url;
	link.download = `flow-like-diagnostics-${report.generatedAt.replace(/[:.]/g, "-")}.json`;
	document.body.append(link);
	link.click();
	link.remove();
	window.setTimeout(() => URL.revokeObjectURL(url), 1000);
}

function formatTimestamp(value: string | null, language: string) {
	if (!value) return null;
	const date = new Date(value);
	if (Number.isNaN(date.getTime())) return value;
	try {
		return new Intl.DateTimeFormat(language, {
			dateStyle: "medium",
			timeStyle: "medium",
		}).format(date);
	} catch {
		return date.toISOString();
	}
}

function availabilityLabel(available: boolean | null, t: SettingsT) {
	if (available === null) return t("diagnosticsUnknown", "Unknown");
	return available
		? t("diagnosticsAvailable", "Available")
		: t("diagnosticsNotAvailable", "Not available");
}

function webGpuStatusLabel(webgpu: DiagnosticsWebGpu, t: SettingsT) {
	switch (webgpu.status) {
		case "available":
			return webgpu.isFallbackAdapter
				? t("diagnosticsWebGpuSoftware", "Software adapter")
				: t("diagnosticsAvailable", "Available");
		case "unsupported":
			return t("diagnosticsWebGpuUnsupported", "Not supported");
		case "no-adapter":
			return t("diagnosticsWebGpuNoAdapter", "No adapter");
		case "timeout":
			return t("diagnosticsWebGpuTimeout", "Timed out");
		case "error":
			return t("diagnosticsWebGpuError", "Failed");
	}
}

function rendererLabel(renderer: BoardRenderer | null, t: SettingsT) {
	if (renderer === null) return null;
	return renderer === "egui"
		? t("diagnosticsRendererWasm", "WASM board")
		: "React Flow";
}

function colorSchemeLabel(
	scheme: DiagnosticsBrowser["colorScheme"],
	t: SettingsT,
) {
	if (scheme === null) return null;
	return scheme === "dark"
		? t("diagnosticsColorSchemeDark", "Dark")
		: t("diagnosticsColorSchemeLight", "Light");
}

function capabilityUses(
	t: SettingsT,
): Partial<Record<DiagnosticsCapabilityId, string>> {
	return {
		"web-workers": t(
			"diagnosticsUsedForWorkers",
			"FlowScript editor and board change snapshots",
		),
		"compression-stream": t(
			"diagnosticsUsedForCompression",
			"Board change snapshots",
		),
		"broadcast-channel": t(
			"diagnosticsUsedForBroadcast",
			"Putting idle background board tabs to sleep",
		),
		"web-locks": t("diagnosticsUsedForLocks", "Node-suggestion learning"),
		indexeddb: t(
			"diagnosticsUsedForIndexedDb",
			"Local undo history, caches and offline changes",
		),
	};
}

function DiagnosticsHeader({
	backHref,
	ready,
	running,
	onCopy,
	onDownload,
	onRerun,
}: Readonly<{
	backHref: string;
	ready: boolean;
	running: boolean;
	onCopy: () => void;
	onDownload: () => void;
	onRerun: () => void;
}>) {
	const { t } = useTranslation("settings");
	return (
		<div className="flex flex-col gap-4 pt-1 lg:flex-row lg:items-start lg:justify-between">
			<div className="min-w-0">
				<Link
					href={backHref}
					className="mb-2 inline-flex items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground"
				>
					<ArrowLeft className="size-4" />{" "}
					{t("diagnosticsBackToSettings", "Settings")}
				</Link>
				<h1 className="text-3xl font-bold tracking-tight">
					{t("diagnosticsTitle", "About & diagnostics")}
				</h1>
				<p className="mt-1 text-muted-foreground">
					{t(
						"diagnosticsSubtitle",
						"Version, browser and device details for troubleshooting. Copy the report and paste it into a support chat.",
					)}
				</p>
			</div>
			<div className="flex flex-wrap gap-2 lg:shrink-0 lg:justify-end">
				<Button size="sm" onClick={onCopy} disabled={!ready}>
					<ClipboardCopy /> {t("diagnosticsCopyReport", "Copy report")}
				</Button>
				<Button
					variant="outline"
					size="sm"
					onClick={onDownload}
					disabled={!ready}
				>
					<Download /> {t("diagnosticsDownloadJson", "Download JSON")}
				</Button>
				<Button
					variant="outline"
					size="sm"
					onClick={onRerun}
					disabled={running}
				>
					<RefreshCw className={cn(running && "animate-spin")} />{" "}
					{t("diagnosticsRunAgain", "Run checks again")}
				</Button>
			</div>
		</div>
	);
}

function DiagnosticsLoading() {
	const { t } = useTranslation("settings");
	return (
		<div className="space-y-5" aria-busy="true">
			<output className="sr-only">
				{t("diagnosticsRunning", "Running checks…")}
			</output>
			<Skeleton className="h-32 w-full rounded-2xl" />
			<div className="grid gap-5 xl:grid-cols-2">
				{LOADING_CARDS.map((key) => (
					<Skeleton key={key} className="h-64 rounded-xl" />
				))}
			</div>
			<Skeleton className="h-80 w-full rounded-xl" />
		</div>
	);
}

function DiagnosticsBody({
	report,
	storageHref,
}: Readonly<{ report: DiagnosticsReport; storageHref?: string }>) {
	return (
		<>
			<SummaryStrip report={report} />
			<div className="grid gap-5 xl:grid-cols-2">
				<BuildCard report={report} />
				<SessionCard session={report.session} />
				<BrowserCard browser={report.browser} className="xl:col-span-2" />
				<HardwareCard hardware={report.hardware} />
				<StorageCard storage={report.storage} storageHref={storageHref} />
				<GraphicsCard graphics={report.graphics} className="xl:col-span-2" />
				<CapabilitiesCard
					capabilities={report.capabilities}
					className="xl:col-span-2"
				/>
			</div>
		</>
	);
}

function SummaryStrip({ report }: Readonly<{ report: DiagnosticsReport }>) {
	const { t, i18n } = useTranslation("settings");
	const { build, browser, graphics } = report;
	const commit = shortCommit(build.commit);
	const checkedAt = formatTimestamp(report.generatedAt, i18n.language);
	return (
		<section className="relative overflow-hidden rounded-2xl border bg-linear-to-br from-card via-card to-primary/7 p-5 shadow-sm md:p-6">
			<div className="pointer-events-none absolute -right-24 -top-28 size-80 rounded-full bg-primary/10 blur-3xl" />
			<dl className="relative grid gap-5 sm:grid-cols-2 lg:grid-cols-5">
				<SummaryItem icon={Package} label={t("diagnosticsVersion", "Version")}>
					<span className="flex min-w-0 flex-wrap items-center gap-1.5">
						<Value>{build.version}</Value>
						{commit && (
							<Badge variant="outline" className="font-mono">
								{commit}
							</Badge>
						)}
						{build.mode && build.mode !== "production" && (
							<Badge variant="secondary">{build.mode}</Badge>
						)}
					</span>
				</SummaryItem>
				<SummaryItem icon={Globe} label={t("diagnosticsBrowser", "Browser")}>
					<Value>{describeBrowser(browser)}</Value>
				</SummaryItem>
				<SummaryItem
					icon={Monitor}
					label={t("diagnosticsOperatingSystem", "Operating system")}
				>
					<Value>{describeOs(browser)}</Value>
				</SummaryItem>
				<SummaryItem icon={Gpu} label="GPU">
					<Value>{gpuDisplayName(graphics)}</Value>
				</SummaryItem>
				<SummaryItem icon={Zap} label="WebGPU">
					<WebGpuPill webgpu={graphics.webgpu} />
				</SummaryItem>
			</dl>
			{checkedAt && (
				<p className="relative mt-4 text-xs text-muted-foreground">
					{t("diagnosticsCheckedAt", "Checked {{time}}", { time: checkedAt })}
				</p>
			)}
		</section>
	);
}

function SummaryItem({
	icon: Icon,
	label,
	children,
}: Readonly<{ icon: LucideIcon; label: string; children: ReactNode }>) {
	return (
		<div className="min-w-0">
			<dt className="flex items-center gap-1.5 text-xs font-medium uppercase tracking-wide text-muted-foreground">
				<Icon className="size-3.5" aria-hidden="true" />
				{label}
			</dt>
			<dd className="mt-1.5 wrap-break-word text-sm font-semibold">
				{children}
			</dd>
		</div>
	);
}

function WebGpuPill({ webgpu }: Readonly<{ webgpu: DiagnosticsWebGpu }>) {
	const { t } = useTranslation("settings");
	return (
		<Badge
			variant="outline"
			className={cn(
				WEBGPU_TONE[webgpu.isFallbackAdapter ? "unsupported" : webgpu.status],
			)}
		>
			{webGpuStatusLabel(webgpu, t)}
		</Badge>
	);
}

function Value({ children }: Readonly<{ children: ReactNode }>) {
	const { t } = useTranslation("settings");
	if (children === null || children === undefined || children === "") {
		return (
			<span className="font-normal text-muted-foreground">
				{t("diagnosticsUnknown", "Unknown")}
			</span>
		);
	}
	return <>{children}</>;
}

function YesNo({ value }: Readonly<{ value: boolean | null }>) {
	const { t } = useTranslation("settings");
	if (value === null) return <Value>{null}</Value>;
	return value ? (
		<span>{t("diagnosticsYes", "Yes")}</span>
	) : (
		<span className="text-muted-foreground">{t("diagnosticsNo", "No")}</span>
	);
}

function SectionCard({
	icon: Icon,
	title,
	description,
	action,
	className,
	children,
}: Readonly<{
	icon: LucideIcon;
	title: string;
	description?: string;
	action?: ReactNode;
	className?: string;
	children: ReactNode;
}>) {
	return (
		<Card className={cn("min-w-0 gap-4", className)}>
			<CardHeader>
				<CardTitle className="flex items-center gap-2.5 text-base">
					<span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
						<Icon className="size-4" aria-hidden="true" />
					</span>
					{title}
				</CardTitle>
				{description && <CardDescription>{description}</CardDescription>}
				{action && <CardAction>{action}</CardAction>}
			</CardHeader>
			<CardContent className="min-w-0">{children}</CardContent>
		</Card>
	);
}

function Rows({
	columns = 1,
	children,
}: Readonly<{ columns?: 1 | 2; children: ReactNode }>) {
	return (
		<div className="overflow-hidden">
			<dl
				className={cn("-mt-px grid gap-x-8", columns === 2 && "md:grid-cols-2")}
			>
				{children}
			</dl>
		</div>
	);
}

function InfoRow({
	label,
	hint,
	mono,
	span,
	children,
}: Readonly<{
	label: string;
	hint?: ReactNode;
	mono?: boolean;
	span?: boolean;
	children: ReactNode;
}>) {
	return (
		<div
			className={cn(
				"flex min-w-0 flex-wrap items-start justify-between gap-x-4 gap-y-1 border-t border-border/50 py-2.5 text-sm",
				span && "md:col-span-2",
			)}
		>
			<dt className="shrink-0 text-muted-foreground">{label}</dt>
			<dd
				className={cn(
					"min-w-0 flex-1 wrap-break-word text-right font-medium",
					mono && "font-mono text-xs leading-5",
				)}
			>
				{children}
			</dd>
			{hint && (
				<dd className="basis-full text-xs text-muted-foreground">{hint}</dd>
			)}
		</div>
	);
}

function BuildCard({ report }: Readonly<{ report: DiagnosticsReport }>) {
	const { t, i18n } = useTranslation("settings");
	const { build, environment } = report;
	return (
		<SectionCard icon={Package} title={t("diagnosticsBuild", "Build")}>
			<Rows>
				<InfoRow label={t("diagnosticsVersion", "Version")}>
					<Value>{build.version}</Value>
				</InfoRow>
				<InfoRow label={t("diagnosticsCommit", "Commit")} mono>
					<Value>{build.commit}</Value>
				</InfoRow>
				<InfoRow label={t("diagnosticsBuiltAt", "Built")}>
					<Value>{formatTimestamp(build.builtAt, i18n.language)}</Value>
				</InfoRow>
				<InfoRow label={t("diagnosticsBuildMode", "Build mode")}>
					<Value>{build.mode}</Value>
				</InfoRow>
				<InfoRow label={t("diagnosticsOrigin", "Origin")} mono>
					<Value>{environment.origin}</Value>
				</InfoRow>
				<InfoRow label={t("diagnosticsApiBaseUrl", "API")} mono>
					<Value>{environment.apiBaseUrl}</Value>
				</InfoRow>
				<InfoRow label={t("diagnosticsSecureContext", "Secure context")}>
					<YesNo value={environment.secureContext} />
				</InfoRow>
				<InfoRow
					label={t("diagnosticsCrossOriginIsolated", "Cross-origin isolated")}
				>
					<YesNo value={environment.crossOriginIsolated} />
				</InfoRow>
			</Rows>
		</SectionCard>
	);
}

function SessionCard({
	session,
}: Readonly<{ session: DiagnosticsSession | null }>) {
	const { t } = useTranslation("settings");
	const fellBack =
		session?.boardRendererPreference === "egui" &&
		session.boardRendererInUse === "react-flow";
	return (
		<SectionCard icon={UserRound} title={t("diagnosticsSession", "Session")}>
			<Rows>
				<InfoRow label={t("diagnosticsSignedIn", "Signed in")}>
					<YesNo value={session?.signedIn ?? null} />
				</InfoRow>
				<InfoRow label={t("diagnosticsAccountId", "Account ID")} mono>
					<Value>{session?.accountId}</Value>
				</InfoRow>
				<InfoRow label={t("diagnosticsDeveloperMode", "Developer mode")}>
					<YesNo value={session?.developerMode ?? null} />
				</InfoRow>
				<InfoRow label={t("diagnosticsLanguage", "Language")}>
					<Value>{session?.language}</Value>
				</InfoRow>
				<InfoRow
					label={t("diagnosticsRendererPreference", "Board renderer setting")}
				>
					<Value>
						{rendererLabel(session?.boardRendererPreference ?? null, t)}
					</Value>
				</InfoRow>
				<InfoRow
					label={t("diagnosticsRendererInUse", "Board renderer in use")}
					hint={
						fellBack
							? t(
									"diagnosticsRendererFallback",
									"This browser has no WebGL2, so boards open in React Flow.",
								)
							: undefined
					}
				>
					<Value>{rendererLabel(session?.boardRendererInUse ?? null, t)}</Value>
				</InfoRow>
			</Rows>
		</SectionCard>
	);
}

function BrowserCard({
	browser,
	className,
}: Readonly<{ browser: DiagnosticsBrowser; className?: string }>) {
	const { t } = useTranslation("settings");
	return (
		<SectionCard
			icon={Globe}
			title={t("diagnosticsBrowser", "Browser")}
			className={className}
		>
			<Rows columns={2}>
				<InfoRow label={t("diagnosticsBrowser", "Browser")}>
					<Value>{describeBrowser(browser)}</Value>
				</InfoRow>
				<InfoRow label={t("diagnosticsOperatingSystem", "Operating system")}>
					<Value>{describeOs(browser)}</Value>
				</InfoRow>
				<InfoRow label={t("diagnosticsArchitecture", "Architecture")}>
					<Value>{describeArchitecture(browser)}</Value>
				</InfoRow>
				<InfoRow label={t("diagnosticsDeviceModel", "Device model")}>
					<Value>{browser.model}</Value>
				</InfoRow>
				<InfoRow label={t("diagnosticsMobile", "Mobile")}>
					<YesNo value={browser.mobile} />
				</InfoRow>
				<InfoRow label={t("diagnosticsClientHints", "Client hints")}>
					<YesNo value={browser.clientHints} />
				</InfoRow>
				<InfoRow label={t("diagnosticsLanguages", "Languages")}>
					<Value>{browser.languages.join(", ")}</Value>
				</InfoRow>
				<InfoRow label={t("diagnosticsTimeZone", "Time zone")}>
					<Value>{browser.timeZone}</Value>
				</InfoRow>
				<InfoRow label={t("diagnosticsOnline", "Online")}>
					<YesNo value={browser.online} />
				</InfoRow>
				<InfoRow label={t("diagnosticsCookies", "Cookies enabled")}>
					<YesNo value={browser.cookieEnabled} />
				</InfoRow>
				<InfoRow label={t("diagnosticsViewport", "Window size")}>
					<Value>{formatSize(browser.viewport)}</Value>
				</InfoRow>
				<InfoRow label={t("diagnosticsScreen", "Screen size")}>
					<Value>{formatSize(browser.screen)}</Value>
				</InfoRow>
				<InfoRow label={t("diagnosticsPixelRatio", "Pixel ratio")}>
					<Value>{browser.devicePixelRatio}</Value>
				</InfoRow>
				<InfoRow label={t("diagnosticsColorScheme", "System color scheme")}>
					<Value>{colorSchemeLabel(browser.colorScheme, t)}</Value>
				</InfoRow>
				<InfoRow label={t("diagnosticsReducedMotion", "Reduced motion")}>
					<YesNo value={browser.reducedMotion} />
				</InfoRow>
				<InfoRow label={t("diagnosticsUserAgent", "User agent")} mono span>
					<Value>{browser.userAgent}</Value>
				</InfoRow>
			</Rows>
		</SectionCard>
	);
}

function HardwareCard({
	hardware,
}: Readonly<{ hardware: DiagnosticsHardware }>) {
	const { t } = useTranslation("settings");
	const memory = hardware.deviceMemoryGb;
	const lowMemory = memory !== null && memory < LOW_MEMORY_GB;
	return (
		<SectionCard icon={Cpu} title={t("diagnosticsHardware", "Hardware")}>
			<Rows>
				<InfoRow label={t("diagnosticsLogicalCores", "CPU threads")}>
					<Value>{hardware.logicalCores}</Value>
				</InfoRow>
				<InfoRow
					label={t("diagnosticsDeviceMemory", "Device memory")}
					hint={
						lowMemory ? (
							<span className="text-destructive">
								{t(
									"diagnosticsLowMemoryLearningOff",
									"Below 4 GB, so node-suggestion learning is switched off.",
								)}
							</span>
						) : (
							t(
								"diagnosticsDeviceMemoryHint",
								"Reported by Chromium-based browsers only, rounded to a coarse value. Below 4 GB, node-suggestion learning is switched off.",
							)
						)
					}
				>
					<span className="inline-flex items-center gap-1.5">
						<MemoryStick
							className="size-3.5 text-muted-foreground"
							aria-hidden="true"
						/>
						<Value>{memory === null ? null : `${memory} GB`}</Value>
					</span>
				</InfoRow>
				<InfoRow label={t("diagnosticsTouchPoints", "Touch points")}>
					<Value>{hardware.maxTouchPoints}</Value>
				</InfoRow>
			</Rows>
		</SectionCard>
	);
}

function StorageCard({
	storage,
	storageHref,
}: Readonly<{ storage: DiagnosticsStorage; storageHref?: string }>) {
	const { t } = useTranslation("settings");
	const percent = storageUsagePercent(storage.usage, storage.quota);
	const details = Object.entries(storage.usageDetails ?? {});
	return (
		<SectionCard
			icon={HardDrive}
			title={t("diagnosticsStorage", "Storage")}
			action={
				storageHref ? (
					<Button asChild variant="outline" size="sm">
						<Link href={storageHref}>
							{t("diagnosticsManageStorage", "Manage storage")}
							<ArrowUpRight />
						</Link>
					</Button>
				) : null
			}
		>
			<div className="mb-4 space-y-2">
				<div className="flex flex-wrap items-baseline justify-between gap-x-3 text-sm">
					<span className="text-lg font-semibold tabular-nums">
						<Value>
							{storage.usage === null ? null : humanFileSize(storage.usage)}
						</Value>
					</span>
					{storage.quota !== null && (
						<span className="text-muted-foreground tabular-nums">
							{t("diagnosticsOfQuota", "of {{quota}} available to this site", {
								quota: humanFileSize(storage.quota),
							})}
						</span>
					)}
				</div>
				<Progress
					value={percent ?? 0}
					aria-label={t("diagnosticsStorageUsage", "Storage usage")}
				/>
			</div>
			<Rows>
				<InfoRow
					label={t("diagnosticsPersisted", "Persistent storage")}
					hint={
						storage.persisted === false
							? t(
									"diagnosticsPersistedHint",
									"The browser may clear this site's data when the device runs low on space.",
								)
							: undefined
					}
				>
					<YesNo value={storage.persisted} />
				</InfoRow>
				{details.map(([name, value]) => (
					<InfoRow key={name} label={name}>
						<span className="tabular-nums">{humanFileSize(value)}</span>
					</InfoRow>
				))}
				{storage.error && (
					<InfoRow label={t("diagnosticsError", "Error")} mono>
						{storage.error}
					</InfoRow>
				)}
			</Rows>
		</SectionCard>
	);
}

function GraphicsCard({
	graphics,
	className,
}: Readonly<{ graphics: DiagnosticsGraphics; className?: string }>) {
	const { t } = useTranslation("settings");
	return (
		<SectionCard
			icon={Gpu}
			title={t("diagnosticsGraphics", "Graphics")}
			className={className}
		>
			<div className="grid gap-x-8 gap-y-6 lg:grid-cols-2">
				<WebGpuDetails webgpu={graphics.webgpu} />
				<WebGlDetails
					webgl={graphics.webgl}
					offscreenCanvas={graphics.offscreenCanvas}
				/>
			</div>
		</SectionCard>
	);
}

function SubHeading({ children }: Readonly<{ children: ReactNode }>) {
	return (
		<h3 className="mb-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
			{children}
		</h3>
	);
}

function WebGpuDetails({ webgpu }: Readonly<{ webgpu: DiagnosticsWebGpu }>) {
	const { t } = useTranslation("settings");
	const adapter = webgpu.adapter;
	const available = webgpu.status === "available";
	return (
		<section className="min-w-0">
			<SubHeading>WebGPU</SubHeading>
			<Rows>
				<InfoRow label={t("diagnosticsStatus", "Status")}>
					<WebGpuPill webgpu={webgpu} />
				</InfoRow>
				{webgpu.error && (
					<InfoRow label={t("diagnosticsError", "Error")} mono>
						{webgpu.error}
					</InfoRow>
				)}
				{available && (
					<>
						<InfoRow label={t("diagnosticsVendor", "Vendor")}>
							<Value>{adapter?.vendor}</Value>
						</InfoRow>
						<InfoRow label={t("diagnosticsArchitecture", "Architecture")}>
							<Value>{adapter?.architecture}</Value>
						</InfoRow>
						<InfoRow label={t("diagnosticsDevice", "Device")}>
							<Value>{adapter?.device}</Value>
						</InfoRow>
						<InfoRow label={t("diagnosticsDescription", "Description")}>
							<Value>{adapter?.description}</Value>
						</InfoRow>
						<InfoRow
							label={t("diagnosticsFallbackAdapter", "Fallback adapter")}
						>
							<YesNo value={webgpu.isFallbackAdapter} />
						</InfoRow>
						{NOTABLE_WEBGPU_FEATURES.map((feature) => (
							<InfoRow key={feature} label={feature}>
								<YesNo value={webgpu.notableFeatures[feature]} />
							</InfoRow>
						))}
						{WEBGPU_LIMIT_NAMES.map((name) => {
							const limit = webgpu.limits[name];
							return (
								<InfoRow key={name} label={name} mono>
									<Value>
										{limit === undefined
											? null
											: formatWebGpuLimit(name, limit)}
									</Value>
								</InfoRow>
							);
						})}
					</>
				)}
			</Rows>
			{available && <FeatureList features={webgpu.features} />}
		</section>
	);
}

function FeatureList({ features }: Readonly<{ features: readonly string[] }>) {
	const { t } = useTranslation("settings");
	const [open, setOpen] = useState(false);
	if (features.length === 0) return null;
	return (
		<Collapsible open={open} onOpenChange={setOpen} className="mt-2">
			<CollapsibleTrigger asChild>
				<Button variant="ghost" size="sm" className="-ml-2">
					<ChevronDown
						className={cn("transition-transform", open && "rotate-180")}
					/>
					{t("diagnosticsWebGpuFeatures", "WebGPU features ({{total}})", {
						total: features.length,
					})}
				</Button>
			</CollapsibleTrigger>
			<CollapsibleContent>
				<ul className="mt-2 flex flex-wrap gap-1.5">
					{features.map((feature) => (
						<li key={feature}>
							<Badge variant="outline" className="font-mono">
								{feature}
							</Badge>
						</li>
					))}
				</ul>
			</CollapsibleContent>
		</Collapsible>
	);
}

function WebGlDetails({
	webgl,
	offscreenCanvas,
}: Readonly<{ webgl: DiagnosticsWebGl; offscreenCanvas: boolean }>) {
	const { t } = useTranslation("settings");
	return (
		<section className="min-w-0">
			<SubHeading>WebGL</SubHeading>
			<Rows>
				<InfoRow label="WebGL">
					<YesNo value={webgl.webgl} />
				</InfoRow>
				<InfoRow
					label="WebGL2"
					hint={t(
						"diagnosticsWebGl2UsedFor",
						"Used for the WASM board editor. Without it, boards fall back to React Flow.",
					)}
				>
					<span className={cn(!webgl.webgl2 && "text-destructive")}>
						<YesNo value={webgl.webgl2} />
					</span>
				</InfoRow>
				<InfoRow label={t("diagnosticsVersion", "Version")} mono>
					<Value>{webgl.version}</Value>
				</InfoRow>
				<InfoRow label={t("diagnosticsVendor", "Vendor")} mono>
					<Value>{webgl.vendor}</Value>
				</InfoRow>
				<InfoRow
					label={t("diagnosticsRenderer", "Renderer")}
					mono
					hint={
						webgl.renderer && !webgl.unmasked
							? t(
									"diagnosticsRendererMasked",
									"The browser hides the exact GPU model.",
								)
							: undefined
					}
				>
					<Value>{webgl.renderer}</Value>
				</InfoRow>
				<InfoRow label={t("diagnosticsMaxTextureSize", "Max texture size")}>
					<Value>{webgl.maxTextureSize}</Value>
				</InfoRow>
				<InfoRow label="OffscreenCanvas">
					<YesNo value={offscreenCanvas} />
				</InfoRow>
			</Rows>
		</section>
	);
}

function CapabilitiesCard({
	capabilities,
	className,
}: Readonly<{
	capabilities: readonly DiagnosticsCapability[];
	className?: string;
}>) {
	const { t } = useTranslation("settings");
	const uses = useMemo(() => capabilityUses(t), [t]);
	return (
		<SectionCard
			icon={Puzzle}
			title={t("diagnosticsCapabilities", "Capabilities")}
			description={t(
				"diagnosticsCapabilitiesDescription",
				"Browser features the app relies on. Missing ones marked in red affect a feature.",
			)}
			className={className}
		>
			<div className="overflow-hidden">
				<ul className="-mt-px grid gap-x-8 md:grid-cols-2">
					{capabilities.map((capability) => (
						<CapabilityRow
							key={capability.id}
							capability={capability}
							usedFor={uses[capability.id]}
						/>
					))}
				</ul>
			</div>
		</SectionCard>
	);
}

function CapabilityRow({
	capability,
	usedFor,
}: Readonly<{ capability: DiagnosticsCapability; usedFor?: string }>) {
	const { t } = useTranslation("settings");
	const { available, detail } = capability;
	const Icon =
		available === true
			? CircleCheck
			: available === false
				? CircleX
				: CircleHelp;
	const tone =
		available === true
			? "text-primary"
			: available === false && usedFor
				? "text-destructive"
				: "text-muted-foreground";
	return (
		<li className="flex min-w-0 items-start gap-3 border-t border-border/50 py-2.5">
			<Icon className={cn("mt-0.5 size-4 shrink-0", tone)} aria-hidden="true" />
			<div className="min-w-0 flex-1">
				<div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-0.5">
					<span className="text-sm font-medium">
						{CAPABILITY_LABELS[capability.id]}
					</span>
					<span className={cn("text-xs font-medium", tone)}>
						{availabilityLabel(available, t)}
					</span>
				</div>
				{detail && (
					<p className="wrap-break-word font-mono text-xs text-muted-foreground">
						{detail}
					</p>
				)}
				{usedFor && (
					<p className="text-xs text-muted-foreground">
						{t("diagnosticsUsedFor", "Used for: {{use}}", { use: usedFor })}
					</p>
				)}
			</div>
		</li>
	);
}
