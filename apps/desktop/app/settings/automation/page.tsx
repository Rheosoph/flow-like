"use client";

import {
	Alert,
	AlertDescription,
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
	AlertDialogTrigger,
	AlertTitle,
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	Progress,
	Skeleton,
	type UseQueryResult,
	cn,
	useQuery,
} from "@flow-like/flow-like-ui";
import { getErrorMessage } from "@flow-like/flow-like-ui/lib/error-message";
import { humanFileSize } from "@flow-like/flow-like-ui/lib/utils";
import { useTranslation } from "@flow-like/locales";
import { invoke } from "@tauri-apps/api/core";
import { type UnlistenFn, listen } from "@tauri-apps/api/event";
import {
	ArrowLeft,
	CheckCircle2,
	Download,
	Globe,
	Info,
	Loader2,
	RefreshCw,
	Trash2,
	TriangleAlert,
} from "lucide-react";
import Link from "next/link";
import {
	type ReactNode,
	useCallback,
	useId,
	useSyncExternalStore,
} from "react";
import { toast } from "sonner";
import {
	useInvalidateTauriInvoke,
	useTauriInvoke,
} from "../../../components/useInvoke";

type BrowserKind = "chrome" | "edge";

type BrowserFlavor =
	| "chrome"
	| "edge"
	| "chromium"
	| "snap_chromium"
	| "chrome_for_testing";

type InstallStage =
	| "resolving"
	| "downloading"
	| "verifying"
	| "extracting"
	| "configuring"
	| "done";

interface InstalledBrowser {
	readonly kind: BrowserKind;
	readonly flavor: BrowserFlavor;
	readonly path: string;
	readonly version: string | null;
}

interface BrowserEngineStatus {
	readonly installed: readonly InstalledBrowser[];
	readonly cft_version: string | null;
	readonly cft_path: string | null;
	readonly cft_supported: boolean;
	readonly cft_pinned: string;
	readonly cache_dir: string;
	readonly download_size_mb: number;
	readonly source: string;
}

interface InstallProgress {
	readonly stage: InstallStage;
	readonly received: number | null;
	readonly total: number | null;
}

interface CftOperation {
	readonly progress: InstallProgress | null;
	readonly removing: boolean;
}

type RefreshStatus = () => Promise<BrowserEngineStatus | undefined>;

interface CftInstaller extends CftOperation {
	readonly install: () => Promise<void>;
	readonly remove: () => Promise<void>;
}

const STATUS_COMMAND = "browser_engine_status";
const LATEST_COMMAND = "browser_engine_latest_cft";
const INSTALL_COMMAND = "browser_engine_install_cft";
const REMOVE_COMMAND = "browser_engine_remove_cft";
const PROGRESS_EVENT = "browser-engine-progress";
const LATEST_STALE_MS = 60 * 60 * 1000;
const CFT_NAME = "Chrome for Testing";
const BROWSER_KINDS: readonly BrowserKind[] = ["chrome", "edge"];
const FLAVOR_NAMES: Readonly<Record<BrowserFlavor, string>> = {
	chrome: "Google Chrome",
	edge: "Microsoft Edge",
	chromium: "Chromium",
	snap_chromium: "Chromium (Snap)",
	chrome_for_testing: CFT_NAME,
};

function compareVersions(left: string, right: string): number {
	const leftParts = left.split(".").map((part) => Number.parseInt(part, 10));
	const rightParts = right.split(".").map((part) => Number.parseInt(part, 10));
	for (let i = 0; i < Math.max(leftParts.length, rightParts.length); i++) {
		const difference = (leftParts[i] || 0) - (rightParts[i] || 0);
		if (difference !== 0) return Math.sign(difference);
	}
	return 0;
}

function newerVersion(
	latest: string | null | undefined,
	installed: string | null,
): string | null {
	if (!latest || !installed) return null;
	return compareVersions(latest, installed) > 0 ? latest : null;
}

function hasRegularChrome(browsers: readonly InstalledBrowser[]): boolean {
	return browsers.some(
		(browser) =>
			browser.kind === "chrome" && browser.flavor !== "snap_chromium",
	);
}

/** Mirrors `launch::find`: installed Chrome or Chromium, then Chrome for Testing, then Snap Chromium. */
function defaultBrowserPath(
	kind: BrowserKind,
	browsers: readonly InstalledBrowser[],
	cftInstalled: boolean,
): string | null {
	const ofKind = browsers.filter((browser) => browser.kind === kind);
	if (kind === "edge" || hasRegularChrome(browsers)) {
		return (
			ofKind.find((browser) => browser.flavor !== "snap_chromium")?.path ??
			ofKind[0]?.path ??
			null
		);
	}
	return cftInstalled ? null : (ofKind[0]?.path ?? null);
}

function downloadPercent(progress: InstallProgress): number | null {
	if (
		progress.stage !== "downloading" ||
		progress.received === null ||
		!progress.total
	) {
		return null;
	}
	return Math.min(100, (progress.received / progress.total) * 100);
}

function useLatestCft(enabled: boolean): UseQueryResult<string | null> {
	return useQuery({
		queryKey: LATEST_COMMAND.split("_"),
		queryFn: () => invoke<string | null>(LATEST_COMMAND),
		enabled,
		staleTime: LATEST_STALE_MS,
		retry: false,
	});
}

const IDLE_OPERATION: CftOperation = { progress: null, removing: false };

/** Module state, so an install or removal still shows after leaving and reopening the page. */
let cftOperation = IDLE_OPERATION;
const cftOperationListeners = new Set<() => void>();

function setCftOperation(next: CftOperation): void {
	cftOperation = next;
	for (const listener of cftOperationListeners) listener();
}

function subscribeCftOperation(listener: () => void): () => void {
	cftOperationListeners.add(listener);
	return () => {
		cftOperationListeners.delete(listener);
	};
}

const currentCftOperation = (): CftOperation => cftOperation;
const idleCftOperation = (): CftOperation => IDLE_OPERATION;

function showInstallProgress(progress: InstallProgress): void {
	setCftOperation({ progress, removing: false });
}

async function listenForInstallProgress(): Promise<UnlistenFn | undefined> {
	try {
		return await listen<InstallProgress>(PROGRESS_EVENT, (event) =>
			showInstallProgress(event.payload),
		);
	} catch (error) {
		console.warn("Listening for browser engine progress failed:", error);
		return undefined;
	}
}

async function installCft(refresh: RefreshStatus): Promise<string> {
	showInstallProgress({ stage: "resolving", received: null, total: null });
	const unlisten = await listenForInstallProgress();
	try {
		return await invoke<string>(INSTALL_COMMAND);
	} finally {
		unlisten?.();
		await refresh();
		setCftOperation(IDLE_OPERATION);
	}
}

async function removeCft(
	refresh: RefreshStatus,
): Promise<BrowserEngineStatus | undefined> {
	setCftOperation({ progress: null, removing: true });
	let status: BrowserEngineStatus | undefined;
	try {
		await invoke(REMOVE_COMMAND);
	} finally {
		status = await refresh();
		setCftOperation(IDLE_OPERATION);
	}
	return status;
}

function useCftInstaller(refresh: RefreshStatus): CftInstaller {
	const { t } = useTranslation("settings");
	const operation = useSyncExternalStore(
		subscribeCftOperation,
		currentCftOperation,
		idleCftOperation,
	);

	const install = useCallback(async () => {
		try {
			const version = await installCft(refresh);
			toast.success(
				t(
					"automation.browser.installedToast",
					"Chrome for Testing {{version}} is installed",
					{ version },
				),
			);
		} catch (error) {
			toast.error(
				t(
					"automation.browser.installFailed",
					"Could not install Chrome for Testing",
				),
				{ description: getErrorMessage(error) },
			);
		}
	}, [refresh, t]);

	const remove = useCallback(async () => {
		try {
			const status = await removeCft(refresh);
			if (status?.cft_version) {
				toast.info(
					t(
						"automation.browser.removeKept",
						"Chrome for Testing is in use by an open browser and was kept. Close that browser and try again.",
					),
				);
			} else {
				toast.success(
					t(
						"automation.browser.removedToast",
						"Chrome for Testing was removed",
					),
				);
			}
		} catch (error) {
			toast.error(
				t(
					"automation.browser.removeFailed",
					"Could not remove Chrome for Testing",
				),
				{ description: getErrorMessage(error) },
			);
		}
	}, [refresh, t]);

	return { ...operation, install, remove };
}

export default function AutomationSettingsPage() {
	const status = useTauriInvoke<BrowserEngineStatus>(STATUS_COMMAND, {});
	const cftInstalled = Boolean(status.data?.cft_version);
	const latest = useLatestCft(
		Boolean(status.data?.cft_supported) && cftInstalled,
	);
	const invalidate = useInvalidateTauriInvoke();
	const { refetch } = status;
	const refresh = useCallback(async () => (await refetch()).data, [refetch]);
	const installer = useCftInstaller(refresh);

	const refreshAll = useCallback(async () => {
		await Promise.all([invalidate(STATUS_COMMAND), invalidate(LATEST_COMMAND)]);
	}, [invalidate]);

	return (
		<div className="h-full min-h-0 overflow-auto">
			<div className="mx-auto flex max-w-5xl flex-col gap-6 px-2 pb-8">
				<PageHeader refreshing={status.isFetching} onRefresh={refreshAll} />
				{status.isLoading && <LoadingCards />}
				{status.error && (
					<LoadError error={status.error} onRetry={refreshAll} />
				)}
				{status.data && (
					<>
						<DetectedBrowsersCard status={status.data} />
						<ChromeForTestingCard
							status={status.data}
							latest={latest}
							installer={installer}
						/>
					</>
				)}
			</div>
		</div>
	);
}

function PageHeader({
	refreshing,
	onRefresh,
}: Readonly<{ refreshing: boolean; onRefresh: () => Promise<void> }>) {
	const { t } = useTranslation("settings");
	return (
		<div className="flex items-start justify-between gap-4 pt-1">
			<div>
				<Link
					href="/settings"
					className="mb-2 inline-flex items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground"
				>
					<ArrowLeft className="size-4" />
					{t("automation.browser.back", "Settings")}
				</Link>
				<h1 className="text-3xl font-bold tracking-tight">
					{t("automation.browser.pageTitle", "Automation")}
				</h1>
				<p className="mt-1 text-muted-foreground">
					{t(
						"automation.browser.pageDescription",
						"The browsers that browser flows launch on this computer",
					)}
				</p>
			</div>
			<Button
				variant="outline"
				size="sm"
				onClick={onRefresh}
				disabled={refreshing}
			>
				<RefreshCw className={cn(refreshing && "animate-spin")} />
				{t("automation.browser.refresh", "Refresh")}
			</Button>
		</div>
	);
}

function LoadingCards() {
	return (
		<div className="flex flex-col gap-6">
			<Skeleton className="h-48 w-full rounded-xl" />
			<Skeleton className="h-64 w-full rounded-xl" />
		</div>
	);
}

function LoadError({
	error,
	onRetry,
}: Readonly<{ error: unknown; onRetry: () => Promise<void> }>) {
	const { t } = useTranslation("settings");
	return (
		<Alert variant="destructive">
			<TriangleAlert />
			<AlertTitle>
				{t(
					"automation.browser.loadFailed",
					"Could not read the browser status",
				)}
			</AlertTitle>
			<AlertDescription className="flex flex-col items-start gap-3">
				<span className="break-all">{getErrorMessage(error)}</span>
				<Button variant="outline" size="sm" onClick={onRetry}>
					{t("automation.browser.retry", "Try again")}
				</Button>
			</AlertDescription>
		</Alert>
	);
}

function DetectedBrowsersCard({
	status,
}: Readonly<{ status: BrowserEngineStatus }>) {
	const { t } = useTranslation("settings");
	const cftInstalled = Boolean(status.cft_version);
	return (
		<Card>
			<CardHeader>
				<CardTitle>
					{t("automation.browser.detectedTitle", "Detected browsers")}
				</CardTitle>
				<CardDescription>
					{t(
						"automation.browser.detectedDescription",
						'Open Browser uses the browser marked "Used by flows" for its Browser Type. For Chrome, an installed Chrome or Chromium comes first, then Chrome for Testing, then Chromium from Snap.',
					)}
				</CardDescription>
			</CardHeader>
			<CardContent className="flex flex-col gap-5">
				{status.installed.length === 0 ? (
					<p className="text-sm text-muted-foreground">
						{t(
							"automation.browser.detectedEmpty",
							"No Chrome, Chromium or Edge is installed on this computer. Browser flows with Browser Type Edge need Microsoft Edge.",
						)}
					</p>
				) : (
					BROWSER_KINDS.map((kind) => (
						<BrowserGroup
							key={kind}
							kind={kind}
							browsers={status.installed.filter(
								(browser) => browser.kind === kind,
							)}
							defaultPath={defaultBrowserPath(
								kind,
								status.installed,
								cftInstalled,
							)}
						/>
					))
				)}
			</CardContent>
		</Card>
	);
}

function BrowserKindHeading({ kind }: Readonly<{ kind: BrowserKind }>) {
	const { t } = useTranslation("settings");
	return (
		<h3 className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
			{kind === "chrome"
				? t("automation.browser.groupChrome", "Browser Type: Chrome")
				: t("automation.browser.groupEdge", "Browser Type: Edge")}
		</h3>
	);
}

function BrowserGroup({
	kind,
	browsers,
	defaultPath,
}: Readonly<{
	kind: BrowserKind;
	browsers: readonly InstalledBrowser[];
	defaultPath: string | null;
}>) {
	const { t } = useTranslation("settings");
	return (
		<section className="flex flex-col gap-2">
			<BrowserKindHeading kind={kind} />
			{browsers.length === 0 ? (
				<p className="text-sm text-muted-foreground">
					{t("automation.browser.groupEmpty", "None found")}
				</p>
			) : (
				<ul className="flex flex-col gap-2">
					{browsers.map((browser) => (
						<BrowserRow
							key={browser.path}
							browser={browser}
							isDefault={browser.path === defaultPath}
						/>
					))}
				</ul>
			)}
		</section>
	);
}

function BrowserRow({
	browser,
	isDefault,
}: Readonly<{ browser: InstalledBrowser; isDefault: boolean }>) {
	const { t } = useTranslation("settings");
	return (
		<li className="flex items-center gap-3 rounded-lg border px-3 py-2.5">
			<div className="flex size-8 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary">
				<Globe className="size-4" />
			</div>
			<div className="min-w-0 flex-1">
				<div className="flex flex-wrap items-center gap-2">
					<span className="text-sm font-medium">
						{FLAVOR_NAMES[browser.flavor]}
					</span>
					{isDefault && (
						<Badge variant="secondary">
							{t("automation.browser.defaultBadge", "Used by flows")}
						</Badge>
					)}
				</div>
				<p
					className="truncate font-mono text-xs text-muted-foreground"
					title={browser.path}
				>
					{browser.path}
				</p>
			</div>
			<span className="shrink-0 text-xs tabular-nums text-muted-foreground">
				{browser.version ??
					t("automation.browser.unknownVersion", "Unknown version")}
			</span>
		</li>
	);
}

function ChromeForTestingCard({
	status,
	latest,
	installer,
}: Readonly<{
	status: BrowserEngineStatus;
	latest: UseQueryResult<string | null>;
	installer: CftInstaller;
}>) {
	const { t } = useTranslation("settings");
	const usedByFlows =
		Boolean(status.cft_version) && !hasRegularChrome(status.installed);
	return (
		<Card>
			<CardHeader className="flex flex-row items-start gap-4 space-y-0">
				<div className="flex size-10 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary">
					<Download className="size-5" />
				</div>
				<div className="min-w-0 flex-1 space-y-1.5">
					<div className="flex flex-wrap items-center gap-2">
						<CardTitle>{CFT_NAME}</CardTitle>
						{usedByFlows && (
							<Badge variant="secondary">
								{t("automation.browser.defaultBadge", "Used by flows")}
							</Badge>
						)}
					</div>
					<CardDescription>
						{t(
							"automation.browser.cftDescription",
							"A standalone Chrome build from Google, made for automation. Browser flows with Browser Type Chrome use it when no Chrome or Chromium is installed, or only Chromium from Snap.",
						)}
					</CardDescription>
				</div>
			</CardHeader>
			<CardContent className="flex flex-col gap-5">
				<CftDetails status={status} latest={latest} />
				{status.cft_supported ? (
					<CftActions
						status={status}
						updateVersion={newerVersion(latest.data, status.cft_version)}
						installer={installer}
					/>
				) : (
					<UnsupportedNotice />
				)}
			</CardContent>
		</Card>
	);
}

function DetailRow({
	label,
	children,
}: Readonly<{ label: string; children: ReactNode }>) {
	return (
		<div className="flex min-w-0 flex-col gap-0.5">
			<dt className="text-xs text-muted-foreground">{label}</dt>
			<dd className="min-w-0 text-sm">{children}</dd>
		</div>
	);
}

function CftDetails({
	status,
	latest,
}: Readonly<{
	status: BrowserEngineStatus;
	latest: UseQueryResult<string | null>;
}>) {
	const { t } = useTranslation("settings");
	return (
		<dl className="grid gap-x-6 gap-y-3 sm:grid-cols-2">
			<DetailRow
				label={t("automation.browser.installedLabel", "Installed version")}
			>
				{status.cft_version ??
					t("automation.browser.notInstalled", "Not installed")}
			</DetailRow>
			{status.cft_version && (
				<DetailRow
					label={t("automation.browser.latestLabel", "Latest version")}
				>
					<LatestVersion latest={latest} installed={status.cft_version} />
				</DetailRow>
			)}
			<DetailRow
				label={t("automation.browser.minimumLabel", "Minimum version")}
			>
				{status.cft_pinned}
			</DetailRow>
			<DetailRow label={t("automation.browser.sizeLabel", "Download size")}>
				{t("automation.browser.sizeValue", "About {{size}} MB", {
					size: status.download_size_mb,
				})}
			</DetailRow>
			<DetailRow label={t("automation.browser.sourceLabel", "Source")}>
				{status.source}
			</DetailRow>
			<DetailRow label={t("automation.browser.cacheLabel", "Cache folder")}>
				<span className="break-all font-mono text-xs">{status.cache_dir}</span>
			</DetailRow>
		</dl>
	);
}

function LatestVersion({
	latest,
	installed,
}: Readonly<{ latest: UseQueryResult<string | null>; installed: string }>) {
	const { t } = useTranslation("settings");
	if (latest.isLoading) {
		return (
			<span className="text-muted-foreground">
				{t("automation.browser.latestChecking", "Checking…")}
			</span>
		);
	}
	if (latest.error || !latest.data) {
		return (
			<span
				className="text-muted-foreground"
				title={latest.error ? getErrorMessage(latest.error) : undefined}
			>
				{t("automation.browser.latestFailed", "Could not check for updates")}
			</span>
		);
	}
	if (newerVersion(latest.data, installed)) return <>{latest.data}</>;
	return (
		<span className="inline-flex items-center gap-1.5">
			<CheckCircle2 className="size-3.5 text-primary" />
			{t("automation.browser.upToDate", "Up to date")}
		</span>
	);
}

function CftActions({
	status,
	updateVersion,
	installer,
}: Readonly<{
	status: BrowserEngineStatus;
	updateVersion: string | null;
	installer: CftInstaller;
}>) {
	const { t } = useTranslation("settings");
	if (installer.progress) {
		return <InstallProgressBar progress={installer.progress} />;
	}
	if (!status.cft_version) {
		return (
			<div>
				<Button onClick={installer.install} disabled={installer.removing}>
					<Download />
					{t("automation.browser.install", "Install")}
				</Button>
			</div>
		);
	}
	return (
		<div className="flex flex-wrap items-center gap-2">
			{updateVersion && (
				<Button onClick={installer.install} disabled={installer.removing}>
					<RefreshCw />
					{t("automation.browser.update", "Update to {{version}}", {
						version: updateVersion,
					})}
				</Button>
			)}
			<RemoveCftButton cacheDir={status.cache_dir} installer={installer} />
		</div>
	);
}

function RemoveCftButton({
	cacheDir,
	installer,
}: Readonly<{ cacheDir: string; installer: CftInstaller }>) {
	const { t } = useTranslation("settings");
	return (
		<AlertDialog>
			<AlertDialogTrigger asChild>
				<Button variant="outline" disabled={installer.removing}>
					{installer.removing ? (
						<Loader2 className="animate-spin" />
					) : (
						<Trash2 />
					)}
					{t("automation.browser.remove", "Remove")}
				</Button>
			</AlertDialogTrigger>
			<AlertDialogContent>
				<AlertDialogHeader>
					<AlertDialogTitle>
						{t("automation.browser.removeTitle", "Remove Chrome for Testing?")}
					</AlertDialogTitle>
					<AlertDialogDescription className="wrap-break-word">
						{t(
							"automation.browser.removeDescription",
							"This deletes the downloaded builds in {{path}}. Browser flows with Browser Type Chrome then need an installed Chrome or Chromium, or a new download.",
							{ path: cacheDir },
						)}
					</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel>
						{t("automation.browser.cancel", "Cancel")}
					</AlertDialogCancel>
					<AlertDialogAction onClick={installer.remove}>
						{t("automation.browser.remove", "Remove")}
					</AlertDialogAction>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}

const INDETERMINATE_PROGRESS_ARIA = {
	"aria-valuenow": undefined,
	"aria-valuetext": undefined,
} as const;

function InstallProgressBar({
	progress,
}: Readonly<{ progress: InstallProgress }>) {
	const percent = downloadPercent(progress);
	const stageId = useId();
	return (
		<div className="flex flex-col gap-2">
			<div className="flex flex-wrap items-center justify-between gap-2 text-sm">
				<span className="inline-flex items-center gap-2 font-medium">
					<Loader2 className="size-4 animate-spin text-primary" />
					<span id={stageId} aria-live="polite">
						<StageLabel stage={progress.stage} />
					</span>
				</span>
				<DownloadAmount progress={progress} />
			</div>
			<Progress
				value={percent ?? 100}
				aria-labelledby={stageId}
				{...(percent === null && INDETERMINATE_PROGRESS_ARIA)}
				className={cn(percent === null && "animate-pulse")}
			/>
		</div>
	);
}

function StageLabel({ stage }: Readonly<{ stage: InstallStage }>) {
	const { t } = useTranslation("settings");
	switch (stage) {
		case "resolving":
			return t(
				"automation.browser.stage.resolving",
				"Looking up the latest version…",
			);
		case "downloading":
			return t("automation.browser.stage.downloading", "Downloading…");
		case "verifying":
			return t("automation.browser.stage.verifying", "Verifying the download…");
		case "extracting":
			return t("automation.browser.stage.extracting", "Unpacking…");
		case "configuring":
			return t(
				"automation.browser.stage.configuring",
				"Configuring the browser…",
			);
		case "done":
			return t("automation.browser.stage.done", "Finishing…");
	}
}

function DownloadAmount({ progress }: Readonly<{ progress: InstallProgress }>) {
	const { t } = useTranslation("settings");
	if (progress.stage !== "downloading" || progress.received === null) {
		return null;
	}
	const received = humanFileSize(progress.received, true);
	return (
		<span className="tabular-nums text-muted-foreground">
			{progress.total
				? t("automation.browser.downloadedOf", "{{received}} of {{total}}", {
						received,
						total: humanFileSize(progress.total, true),
					})
				: t("automation.browser.downloaded", "{{received}} downloaded", {
						received,
					})}
		</span>
	);
}

function UnsupportedNotice() {
	const { t } = useTranslation("settings");
	return (
		<Alert>
			<Info />
			<AlertTitle>
				{t(
					"automation.browser.unsupportedTitle",
					"Not available on this computer",
				)}
			</AlertTitle>
			<AlertDescription>
				{t(
					"automation.browser.unsupportedDescription",
					"Chrome for Testing has no build for this platform, such as Windows on ARM. Install Microsoft Edge or Google Chrome to run browser flows.",
				)}
			</AlertDescription>
		</Alert>
	);
}
