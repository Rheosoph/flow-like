"use client";

import {
	Button,
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
	PackageStatusBadge,
	Popover,
	PopoverContent,
	PopoverTrigger,
	cn,
} from "@flow-like/flow-like-ui";
import {
	PackageCard,
	PackageCardStat,
	PackageCardStats,
	PackageRatingStar,
	formatCompact,
} from "@flow-like/flow-like-ui/components/store/package-card";
import { packageStoreHref } from "@flow-like/flow-like-ui/components/store/package-navigation";
import { useRegistryPackage } from "@flow-like/flow-like-ui/components/store/package-workspace/use-registry-package";
import type { WorkspaceTab } from "@flow-like/flow-like-ui/components/store/package-workspace/workspace-href";
import { getErrorMessage } from "@flow-like/flow-like-ui/lib/error-message";
import { openExternalUrl } from "@flow-like/flow-like-ui/lib/open-external";
import { isOwner } from "@flow-like/flow-like-ui/lib/permission/wasm-package-permission";
import { TEMPLATE_LANGUAGES } from "@flow-like/flow-like-ui/lib/schema/developer";
import { PackageStatus } from "@flow-like/flow-like-ui/lib/schema/wasm";
import { useTranslation } from "@flow-like/locales";
import { useMutation } from "@tanstack/react-query";
import {
	AlertCircle,
	AlertTriangle,
	Archive,
	ArrowRight,
	Bug,
	ChevronDown,
	CloudUpload,
	Code2,
	ExternalLink,
	Eye,
	FileText,
	FolderOpen,
	GitBranch,
	History,
	Link2,
	Loader2,
	type LucideIcon,
	MoreHorizontal,
	PenLine,
	RefreshCw,
	Users,
	X,
} from "lucide-react";
import Link from "next/link";
import {
	type ReactNode,
	type RefObject,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { usePackageStatus } from "../../hooks/use-package-status";
import { fetcher } from "../../lib/api";
import { desktopPlatform } from "../../lib/platform";
import {
	loadIntoCatalog,
	openInEditor,
	projectRoutes,
	revealProject,
} from "./local-projects";
import { MINE_STATE_TONE, useMineFilterLabels } from "./mine-labels";
import {
	type MineAction,
	type MineEntry,
	localSummary,
	mineActionTab,
} from "./mine-model";

export interface MinePackageCardProps {
	entry: MineEntry;
	onInspect: (path: string) => void;
	onRemove: (projectIds: string[]) => void;
	onLinkFolder: () => void;
	onProjectChanged: (path: string) => void;
	linkingFolder: boolean;
}

/** Above the card's stretched workspace link. */
const ABOVE_CARD_LINK = "relative z-2";

/** The workspace for an entry: its newest checkout, or the registry id when there is none here. */
export function entryWorkspaceHref(entry: MineEntry, tab?: WorkspaceTab) {
	const newest = entry.checkouts[0];
	return projectRoutes.workspace(
		newest ? { project: newest.path, tab } : { id: entry.packageId, tab },
	);
}

function useIsVisible(ref: RefObject<HTMLElement | null>) {
	const [isVisible, setIsVisible] = useState(false);
	useEffect(() => {
		const element = ref.current;
		if (!element) return;
		const observer = new IntersectionObserver(
			([entry]) => {
				if (entry?.isIntersecting) {
					setIsVisible(true);
					observer.disconnect();
				}
			},
			{ rootMargin: "200px" },
		);
		observer.observe(element);
		return () => observer.disconnect();
	}, [ref]);
	return isVisible;
}

export function useCheckoutActions(onProjectChanged: (path: string) => void) {
	const { t } = useTranslation("common");
	const reload = useMutation({
		mutationFn: loadIntoCatalog,
		onSuccess: (count, path) => {
			toast.success(
				t("loadedCountNodesIntoTheCatalog", {
					defaultValue_one: "Loaded {{count}} node into the catalog",
					defaultValue_other: "Loaded {{count}} nodes into the catalog",
					count,
				}),
			);
			onProjectChanged(path);
		},
		onError: (error) => toast.error(getErrorMessage(error)),
	});
	const openEditor = useMutation({
		mutationFn: openInEditor,
		onError: (error) => toast.error(getErrorMessage(error)),
	});
	const reveal = useMutation({
		mutationFn: revealProject,
		onError: (error) => toast.error(getErrorMessage(error)),
	});
	return { reload, openEditor, reveal };
}

export type CheckoutActions = ReturnType<typeof useCheckoutActions>;

function StatePill({ entry }: { entry: MineEntry }) {
	const { t } = useTranslation("common");
	const labels = useMineFilterLabels();
	const tone = MINE_STATE_TONE[entry.state];
	const className = cn(
		ABOVE_CARD_LINK,
		"inline-flex h-5 max-w-full items-center gap-1.5 rounded-full border border-border/60 bg-background/85 px-2 text-[11px] font-medium backdrop-blur-sm",
		tone.text,
	);
	const content = (
		<>
			<span
				aria-hidden="true"
				className={cn("size-1.5 shrink-0 rounded-full", tone.dot)}
			/>
			<span className="truncate">{labels[entry.state]}</span>
		</>
	);
	if (entry.state === "disabled") {
		return (
			<Link
				href={entryWorkspaceHref(entry, "releases")}
				title={t("openReleases", "Open Releases")}
				className={cn(
					className,
					"outline-none transition-colors hover:bg-background focus-visible:ring-2 focus-visible:ring-ring",
				)}
			>
				{content}
			</Link>
		);
	}
	return (
		<span className={cn(className, "pointer-events-none")}>{content}</span>
	);
}

function useVersionLabel(entry: MineEntry): string {
	const { t } = useTranslation("common");
	if (entry.state === "unpublished-changes") {
		return t("versionAheadOfLive", "v{{local}} → v{{live}} live", {
			local: entry.localVersion,
			live: entry.liveVersion,
		});
	}
	if (entry.state === "local-only") {
		return entry.localVersion
			? t("versionNotPublished", "v{{version}} · not published", {
					version: entry.localVersion,
				})
			: t("notPublished", "Not published");
	}
	if (entry.state === "disabled") {
		return t("versionDisabled", "v{{version}} · disabled", {
			version: entry.liveVersion,
		});
	}
	return entry.registry?.status === PackageStatus.PendingReview
		? t("versionInReview", "v{{version}} in review", {
				version: entry.liveVersion,
			})
		: t("versionLive", "v{{version}} live", { version: entry.liveVersion });
}

function useStateStat(entry: MineEntry): { label: string; className: string } {
	const { t } = useTranslation("common");
	const status = entry.registry?.status;
	if (status === PackageStatus.Rejected)
		return { label: t("rejected", "Rejected"), className: "text-destructive" };
	if (status === PackageStatus.Disabled)
		return { label: t("disabled", "Disabled"), className: "text-destructive" };
	if (entry.state === "local-only")
		return { label: t("draft", "Draft"), className: "text-muted-foreground" };
	if (entry.state === "unpublished-changes")
		return {
			label: t("ahead", "Ahead"),
			className: MINE_STATE_TONE["unpublished-changes"].text,
		};
	if (status === PackageStatus.PendingReview)
		return {
			label: t("review", "Review"),
			className: MINE_STATE_TONE["in-review"].text,
		};
	return { label: t("live", "Live"), className: MINE_STATE_TONE.live.text };
}

function MineFooter({ entry }: { entry: MineEntry }) {
	const { t } = useTranslation("common");
	const stat = useStateStat(entry);
	const registry = entry.registry;
	const rated = (registry?.ratingCount ?? 0) > 0;
	return (
		<PackageCardStats>
			<PackageCardStat
				value={registry ? formatCompact(registry.downloadCount) : "—"}
				label={t("installs", "Installs")}
				valueClassName="tabular-nums"
			/>
			<PackageCardStat
				value={
					rated ? (
						<>
							<PackageRatingStar />
							{(registry?.avgRating ?? 0).toFixed(1)}
						</>
					) : (
						"—"
					)
				}
				label={t("rating", "Rating")}
				valueClassName="flex items-center gap-1 tabular-nums"
			/>
			<PackageCardStat
				value={stat.label}
				label={t("state", "State")}
				valueClassName={cn("truncate", stat.className)}
			/>
		</PackageCardStats>
	);
}

function StatusText({
	icon: Icon,
	className,
	children,
	title,
}: {
	icon: LucideIcon;
	className: string;
	children: ReactNode;
	title?: string;
}) {
	return (
		<span
			className={cn("flex min-w-0 flex-1 items-center gap-1.5", className)}
			title={title}
		>
			<Icon aria-hidden="true" className="size-3.5 shrink-0" />
			<span className="truncate">{children}</span>
		</span>
	);
}

const WARNING_TEXT = "font-medium text-tertiary";

function StatusMessage({ entry }: { entry: MineEntry }) {
	const { t } = useTranslation("common");
	const issue = entry.issues[0];
	const newest = entry.checkouts[0];
	const owner = isOwner(entry.registry?.viewerPermission ?? 0);

	if (issue?.kind === "placeholder-id")
		return (
			<StatusText icon={AlertTriangle} className={WARNING_TEXT}>
				{t(
					"placeholderIdChooseYourOwn",
					"Placeholder id — choose your own before publishing",
				)}
			</StatusText>
		);
	if (issue?.kind === "id-taken")
		return (
			<StatusText icon={AlertCircle} className="font-medium text-destructive">
				{t(
					"idTakenRenameBeforePublishing",
					"Id taken — rename before publishing",
				)}
			</StatusText>
		);
	if (entry.state === "disabled")
		return (
			<StatusText icon={Archive} className="font-medium text-destructive">
				{owner
					? t(
							"disabledRestoreFromReleases",
							"Disabled — restore it from Releases",
						)
					: t(
							"disabledOnlyAnOwnerCanRestore",
							"Disabled — only an Owner can restore it",
						)}
			</StatusText>
		);
	if (issue?.kind === "lint")
		return (
			<StatusText icon={AlertCircle} className="font-medium text-destructive">
				{t("countLintErrorsFixBeforePublishing", {
					defaultValue_one: "{{count}} lint error — fix before publishing",
					defaultValue_other: "{{count}} lint errors — fix before publishing",
					count: issue.errors,
				})}
			</StatusText>
		);
	if (issue?.kind === "stale")
		return (
			<StatusText icon={AlertTriangle} className={WARNING_TEXT}>
				{t(
					"wasmChangedReloadIntoTheCatalog",
					"WASM changed — reload into catalog",
				)}
			</StatusText>
		);
	if (!newest)
		return (
			<StatusText
				icon={Users}
				className={MINE_STATE_TONE["not-on-this-machine"].text}
			>
				{owner
					? t("noFolderHereYoureTheOwner", "No folder here · you're the Owner")
					: t(
							"noFolderHereYoureAMaintainer",
							"No folder here · you're a Maintainer",
						)}
			</StatusText>
		);
	return (
		<StatusText
			icon={FolderOpen}
			className="font-mono text-[11px] text-muted-foreground"
			title={newest.path}
		>
			{newest.path}
		</StatusText>
	);
}

function CheckoutTag({
	className,
	children,
}: {
	className: string;
	children: ReactNode;
}) {
	return (
		<span
			className={cn(
				"rounded-full px-1.5 text-[11px] font-medium leading-4.5",
				className,
			)}
		>
			{children}
		</span>
	);
}

function CheckoutsPopover({
	entry,
	onRemove,
}: {
	entry: MineEntry;
	onRemove: (projectIds: string[]) => void;
}) {
	const { t } = useTranslation("common");
	const count = entry.checkouts.length;
	return (
		<Popover>
			<PopoverTrigger asChild>
				<button
					type="button"
					className={cn(
						ABOVE_CARD_LINK,
						"inline-flex h-5 shrink-0 items-center gap-1 rounded-full bg-muted px-2 text-[10.5px] font-medium text-foreground transition-colors hover:bg-secondary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring data-[state=open]:bg-secondary",
					)}
				>
					<GitBranch aria-hidden="true" className="size-2.5" />
					{t("countCheckouts", {
						defaultValue_one: "{{count}} checkout",
						defaultValue_other: "{{count}} checkouts",
						count,
					})}
					<ChevronDown aria-hidden="true" className="size-2.5" />
				</button>
			</PopoverTrigger>
			<PopoverContent align="end" className="w-80 p-3">
				<p className="text-xs font-semibold">
					{t(
						"sameManifestIdCountFoldersHere",
						"Same manifest id, {{count}} folders here",
						{
							count,
						},
					)}
				</p>
				<ul className="mt-1">
					{entry.checkouts.map((checkout) => (
						<li
							key={checkout.projectId}
							className="flex items-center gap-2 border-t border-border/60 py-1.5"
						>
							<div className="min-w-0 flex-1">
								<div className="flex items-center gap-2">
									<span className="font-mono text-xs">
										{checkout.version ?? "—"}
									</span>
									{checkout.newest && (
										<CheckoutTag className="bg-primary/10 text-primary">
											{t("newest", "newest")}
										</CheckoutTag>
									)}
									{checkout.matchesLive && (
										<CheckoutTag className="bg-emerald-500/10 text-emerald-600 dark:text-emerald-400">
											{t("equalsLive", "= live")}
										</CheckoutTag>
									)}
								</div>
								<Link
									href={projectRoutes.workspace({ project: checkout.path })}
									className="block truncate rounded-sm font-mono text-[11px] text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
									title={checkout.path}
								>
									{checkout.path}
								</Link>
							</div>
							<Button
								variant="ghost"
								size="icon"
								className="size-7 shrink-0 text-muted-foreground"
								aria-label={t("removeFromList", "Remove from list")}
								title={t(
									"removeFromListFilesStayOnDisk",
									"Remove from list · files stay on disk",
								)}
								onClick={() => onRemove([checkout.projectId])}
							>
								<X />
							</Button>
						</li>
					))}
				</ul>
				<p className="mt-1 text-[11px] text-muted-foreground">
					{t(
						"publishAndReloadUseTheNewestCheckout",
						"Publish and reload use the newest checkout.",
					)}
				</p>
			</PopoverContent>
		</Popover>
	);
}

function StatusLine({
	entry,
	onRemove,
}: {
	entry: MineEntry;
	onRemove: (projectIds: string[]) => void;
}) {
	const newest = entry.checkouts[0];
	const compileStatus = usePackageStatus(newest ? `dev:${newest.path}` : "");
	return (
		<div className="flex h-5 min-w-0 items-center gap-1.5 text-xs">
			<StatusMessage entry={entry} />
			{(compileStatus === "compiling" || compileStatus === "error") && (
				<PackageStatusBadge status={compileStatus} />
			)}
			{entry.checkouts.length > 1 && (
				<CheckoutsPopover entry={entry} onRemove={onRemove} />
			)}
		</div>
	);
}

export interface MineActionView {
	label: string;
	icon: LucideIcon;
	iconClassName?: string;
	/** The icon follows the label ("Open →"). */
	trailingIcon?: boolean;
	emphasis?: boolean;
}

/** Label and icon of a Mine action, shared by the card and the workspace header. */
export function useMineActionView(
	entry: MineEntry,
	action: MineAction,
): MineActionView | null {
	const { t } = useTranslation("common");
	switch (action.kind) {
		case "choose-id":
			return {
				label: t("chooseAnId", "Choose an id"),
				icon: PenLine,
				iconClassName: "text-tertiary",
			};
		case "rename-id":
			return {
				label: t("renameId", "Rename id"),
				icon: PenLine,
				iconClassName: "text-destructive",
			};
		case "fix-errors":
			return {
				label: t("fixCountErrors", {
					defaultValue_one: "Fix {{count}} error",
					defaultValue_other: "Fix {{count}} errors",
					count: action.errors,
				}),
				icon: AlertCircle,
				iconClassName: "text-destructive",
			};
		case "reload":
			return {
				label: t("reloadIntoCatalog", "Reload into catalog"),
				icon: RefreshCw,
				iconClassName: "text-tertiary",
			};
		case "publish": {
			const first = action.first || !action.version;
			return first
				? {
						label: t("publishEllipsis", "Publish…"),
						icon: CloudUpload,
						iconClassName: "text-muted-foreground",
					}
				: {
						label: t("publishVersion", "Publish {{version}}", {
							version: action.version,
						}),
						icon: CloudUpload,
						emphasis: true,
					};
		}
		case "view-review":
			return {
				label: t("viewReview", "View review"),
				icon: Eye,
				iconClassName: MINE_STATE_TONE["in-review"].text,
			};
		case "view-releases":
			return {
				label: isOwner(entry.registry?.viewerPermission ?? 0)
					? t("restoreEllipsis", "Restore…")
					: t("viewReleases", "View releases"),
				icon: History,
				iconClassName: "text-destructive",
			};
		case "open":
			return {
				label: t("open", "Open"),
				icon: ArrowRight,
				iconClassName: MINE_STATE_TONE.live.text,
				trailingIcon: true,
			};
		case "link-folder":
			return null;
	}
}

export function MineActionContent({
	view,
	busy,
}: Readonly<{ view: MineActionView; busy?: boolean }>) {
	const Icon = busy ? Loader2 : view.icon;
	const icon = (
		<Icon
			className={cn(busy ? "animate-spin" : view.iconClassName)}
			aria-hidden="true"
		/>
	);
	return (
		<>
			{!view.trailingIcon && icon}
			<span className="truncate">{view.label}</span>
			{view.trailingIcon && icon}
		</>
	);
}

const CARD_ACTION_CLASS = "h-8 min-w-0 flex-1";

function PrimaryAction({
	entry,
	action,
	actions,
}: {
	entry: MineEntry;
	action: Exclude<MineAction, { kind: "link-folder" }>;
	actions: CheckoutActions;
}) {
	const view = useMineActionView(entry, action);
	if (!view) return null;
	if (action.kind === "reload") {
		return (
			<Button
				size="sm"
				variant="secondary"
				className={CARD_ACTION_CLASS}
				disabled={actions.reload.isPending}
				onClick={() => actions.reload.mutate(action.path)}
			>
				<MineActionContent view={view} busy={actions.reload.isPending} />
			</Button>
		);
	}
	const href =
		action.kind === "publish"
			? projectRoutes.publish(action.path)
			: entryWorkspaceHref(entry, mineActionTab(action) ?? undefined);
	return (
		<Button
			asChild
			size="sm"
			variant={view.emphasis ? "default" : "secondary"}
			className={CARD_ACTION_CLASS}
		>
			<Link href={href}>
				<MineActionContent view={view} />
			</Link>
		</Button>
	);
}

function useRepositoryUrl(packageId: string): string | null {
	const auth = useAuth();
	const { entry } = useRegistryPackage(packageId, fetcher, auth, {
		localFallback: false,
	});
	const repository = entry?.manifest?.repository?.trim();
	return repository && /^https?:\/\//i.test(repository) ? repository : null;
}

function RemoteActions({
	packageId,
	onLinkFolder,
	linkingFolder,
}: {
	packageId: string;
	onLinkFolder: () => void;
	linkingFolder: boolean;
}) {
	const { t } = useTranslation("common");
	const repository = useRepositoryUrl(packageId);
	return (
		<>
			<Button
				size="sm"
				variant="secondary"
				className={CARD_ACTION_CLASS}
				disabled={linkingFolder}
				onClick={onLinkFolder}
			>
				{linkingFolder ? (
					<Loader2 className="animate-spin" />
				) : (
					<Link2 className={MINE_STATE_TONE["not-on-this-machine"].text} />
				)}
				<span className="truncate">{t("linkFolder", "Link folder…")}</span>
			</Button>
			{repository && (
				<Button
					size="sm"
					variant="outline"
					className={CARD_ACTION_CLASS}
					onClick={() => openExternalUrl(repository)}
				>
					<GitBranch className="text-muted-foreground" />
					<span className="truncate">{t("cloneRepo", "Clone repo")}</span>
				</Button>
			)}
		</>
	);
}

export function useRevealLabel(): string {
	const { t } = useTranslation("common");
	const platform = desktopPlatform();
	if (platform === "macos") return t("showInFinder", "Show in Finder");
	if (platform === "windows") return t("showInExplorer", "Show in Explorer");
	return t("showInFolder", "Show in folder");
}

function MenuLink({
	href,
	icon: Icon,
	children,
}: {
	href: string;
	icon: LucideIcon;
	children: ReactNode;
}) {
	return (
		<DropdownMenuItem asChild>
			<Link href={href}>
				<Icon className="text-muted-foreground" />
				{children}
			</Link>
		</DropdownMenuItem>
	);
}

function MoreMenu({
	entry,
	actions,
	onRemove,
}: {
	entry: MineEntry;
	actions: CheckoutActions;
	onRemove: (projectIds: string[]) => void;
}) {
	const { t } = useTranslation("common");
	const revealLabel = useRevealLabel();
	const newest = entry.checkouts[0];
	const registry = entry.registry;
	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<Button
					variant="outline"
					size="icon"
					className="size-8 shrink-0 text-muted-foreground"
					aria-label={t("moreActionsFor", "More actions for {{name}}", {
						name: entry.name,
					})}
				>
					<MoreHorizontal />
				</Button>
			</DropdownMenuTrigger>
			<DropdownMenuContent align="end" className="w-60">
				{newest && (
					<>
						<DropdownMenuItem
							onSelect={() => actions.openEditor.mutate(newest.path)}
						>
							<Code2 className="text-muted-foreground" />
							{t("openInEditor", "Open in editor")}
						</DropdownMenuItem>
						<DropdownMenuItem
							disabled={actions.reload.isPending}
							onSelect={() => actions.reload.mutate(newest.path)}
						>
							<RefreshCw className="text-muted-foreground" />
							{t("reloadIntoCatalog", "Reload into catalog")}
						</DropdownMenuItem>
						<MenuLink href={entryWorkspaceHref(entry, "test")} icon={Bug}>
							{t("debugAmpTest", "Debug & test")}
						</MenuLink>
						<MenuLink
							href={entryWorkspaceHref(entry, "manifest")}
							icon={FileText}
						>
							{t("editManifest", "Edit manifest")}
						</MenuLink>
						<DropdownMenuItem
							onSelect={() => actions.reveal.mutate(newest.path)}
						>
							<FolderOpen className="text-muted-foreground" />
							{revealLabel}
						</DropdownMenuItem>
					</>
				)}
				{registry && (
					<>
						{newest && <DropdownMenuSeparator />}
						<MenuLink href={entryWorkspaceHref(entry, "access")} icon={Users}>
							{t("accessAndPeople", "Access & people")}
						</MenuLink>
						<MenuLink
							href={entryWorkspaceHref(entry, "releases")}
							icon={History}
						>
							{t("releases", "Releases")}
						</MenuLink>
						<MenuLink
							href={packageStoreHref({ id: registry.id, tab: "mine" })}
							icon={ExternalLink}
						>
							{t("viewStorePage", "View store page")}
						</MenuLink>
					</>
				)}
				{newest && (
					<>
						<DropdownMenuSeparator />
						<DropdownMenuItem
							className="items-start"
							onSelect={() =>
								onRemove(entry.checkouts.map((checkout) => checkout.projectId))
							}
						>
							<X className="mt-0.5 text-muted-foreground" />
							<span className="flex flex-col">
								<span>{t("removeFromList", "Remove from list")}</span>
								<span className="text-xs text-muted-foreground">
									{t("filesStayOnDisk", "Files stay on disk")}
								</span>
							</span>
						</DropdownMenuItem>
					</>
				)}
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

/**
 * The whole card opens the workspace through a stretched link painted over the
 * card; the actions row, the ⋯ menu, the checkouts popover and the state pill
 * sit above it. The title link stays the keyboard tab stop.
 */
export function MinePackageCard({
	entry,
	onInspect,
	onRemove,
	onLinkFolder,
	onProjectChanged,
	linkingFolder,
}: MinePackageCardProps) {
	const ref = useRef<HTMLDivElement>(null);
	const visible = useIsVisible(ref);
	const inspectPath = entry.checkouts[0]?.path;
	const actions = useCheckoutActions(onProjectChanged);
	const summary = useMemo(() => localSummary(entry), [entry]);
	const versionLabel = useVersionLabel(entry);
	const language = TEMPLATE_LANGUAGES.find(
		(candidate) => candidate.value === entry.language,
	);
	const action = entry.primaryAction;
	const overviewHref = entryWorkspaceHref(entry);

	useEffect(() => {
		if (visible && inspectPath) onInspect(inspectPath);
	}, [visible, inspectPath, onInspect]);

	return (
		<div ref={ref} className="min-w-0">
			<PackageCard
				pkg={summary}
				href={null}
				titleHref={overviewHref}
				versionLabel={versionLabel}
				overlay={<StatePill entry={entry} />}
				iconSrc={language?.img}
				footer={
					<>
						<MineFooter entry={entry} />
						<Link
							href={overviewHref}
							tabIndex={-1}
							aria-hidden="true"
							className="absolute inset-0 z-1 rounded-xl"
						/>
					</>
				}
				className={cn(
					entry.state === "not-on-this-machine" &&
						"border-dashed border-violet-500/50",
					entry.state === "disabled" && "border-dashed border-destructive/40",
				)}
				actions={
					<>
						<StatusLine entry={entry} onRemove={onRemove} />
						<div
							className={cn(
								ABOVE_CARD_LINK,
								"mt-2.5 flex min-w-0 items-center gap-1.5",
							)}
						>
							{action.kind === "link-folder" ? (
								<RemoteActions
									packageId={action.packageId}
									onLinkFolder={onLinkFolder}
									linkingFolder={linkingFolder}
								/>
							) : (
								<PrimaryAction
									entry={entry}
									action={action}
									actions={actions}
								/>
							)}
							<MoreMenu entry={entry} actions={actions} onRemove={onRemove} />
						</div>
					</>
				}
			/>
		</div>
	);
}
