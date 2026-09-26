"use client";

import {
	Button,
	RelativeTime,
	Skeleton,
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@flow-like/flow-like-ui";
import type { WorkspaceTab } from "@flow-like/flow-like-ui/components/store/package-workspace/workspace-href";
import {
	liveVersion,
	overviewChecks,
} from "@flow-like/flow-like-ui/components/store/package-workspace/workspace-model";
import {
	Field,
	FieldGrid,
	WorkspaceChecks,
	WorkspaceLinkButton,
	WorkspaceSection,
} from "@flow-like/flow-like-ui/components/store/package-workspace/workspace-parts";
import { asArray } from "@flow-like/flow-like-ui/lib/response-shape";
import { TEMPLATE_LANGUAGES } from "@flow-like/flow-like-ui/lib/schema/developer";
import type { RegistryEntry } from "@flow-like/flow-like-ui/lib/schema/wasm";
import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import {
	AlertTriangle,
	ArrowRight,
	CheckCircle2,
	Code2,
	FileCode,
	FolderOpen,
	GitCompare,
	type LucideIcon,
	PenLine,
	RefreshCw,
	ShieldCheck,
} from "lucide-react";
import { useMemo } from "react";
import { type BuildArtifactInfo, buildArtifactInfo } from "../local-projects";
import type { MineCheckout } from "../mine-model";
import { type CheckoutActions, useRevealLabel } from "../mine-package-card";
import { type DiffedNode, diffNodes, hasNodeChanges } from "./local-diff";
import { type NodeDebuggerSession, ReinspectButton } from "./node-debugger";

const PREVIEW_NODE_COUNT = 6;

export interface LocalOverviewProps {
	projectPath: string;
	packageId?: string;
	/** Show the "Placeholder id" notice; the host decides, since a maintained `com.example.*` id is kept. */
	placeholder?: boolean;
	version?: string | null;
	checkout?: MineCheckout;
	checkoutCount: number;
	session: NodeDebuggerSession;
	/** Only a registry entry the caller maintains; the node diff is against its live version. */
	registryEntry?: RegistryEntry;
	actions: CheckoutActions;
	onSelectTab: (tab: WorkspaceTab) => void;
}

function useBuildInfo(session: NodeDebuggerSession) {
	const artifact =
		session.inspection?.wasmPath || session.inspection?.widgetBundlePath || "";
	return useQuery<BuildArtifactInfo>({
		queryKey: ["developer-build-info", artifact, session.inspectedAt],
		queryFn: () => buildArtifactInfo(artifact),
		enabled: !!artifact,
		retry: false,
		meta: { persist: false },
	});
}

/** Main column of the Overview for a linked checkout: placeholder warning, checkout and node changes. */
export function LocalOverview({
	projectPath,
	packageId,
	placeholder,
	version,
	checkout,
	checkoutCount,
	session,
	registryEntry,
	actions,
	onSelectTab,
}: Readonly<LocalOverviewProps>) {
	const build = useBuildInfo(session);
	return (
		<>
			{placeholder && (
				<PlaceholderNotice
					id={packageId ?? ""}
					onChoose={() => onSelectTab("manifest")}
				/>
			)}
			<CheckoutSection
				projectPath={projectPath}
				version={version}
				checkout={checkout}
				checkoutCount={checkoutCount}
				builtAt={build.data?.builtAt}
				actions={actions}
			/>
			{registryEntry ? (
				<ChangesSection
					session={session}
					entry={registryEntry}
					onOpenNodes={() => onSelectTab("nodes")}
				/>
			) : (
				<LocalNodesPreview
					session={session}
					onOpenNodes={() => onSelectTab("nodes")}
				/>
			)}
		</>
	);
}

/** Side column of the Overview for a linked checkout: the Build and Lint checks. */
export function LocalOverviewChecks({
	session,
}: Readonly<{ session: NodeDebuggerSession }>) {
	const build = useBuildInfo(session);
	return <ChecksSection session={session} build={build.data} />;
}

function PlaceholderNotice({
	id,
	onChoose,
}: Readonly<{ id: string; onChoose: () => void }>) {
	const { t } = useTranslation("common");
	return (
		<section
			aria-label={t("placeholderIdTitle", "Placeholder id")}
			className="flex flex-col gap-3 rounded-xl border border-tertiary/40 bg-tertiary/5 px-4 py-3 sm:flex-row sm:items-center"
		>
			<AlertTriangle className="size-5 shrink-0 text-tertiary" />
			<div className="min-w-0 flex-1 text-sm">
				<p className="font-medium">
					{t(
						"placeholderIdChooseYourOwn",
						"Placeholder id — choose your own before publishing",
					)}
				</p>
				<p className="text-muted-foreground">
					{t(
						"placeholderIdDescription",
						"{{id}} comes from the template. Pick a reverse domain you control so the registry can keep it yours.",
						{ id },
					)}
				</p>
			</div>
			<Button size="sm" onClick={onChoose} className="shrink-0">
				<PenLine />
				{t("chooseAnId", "Choose an id")}
			</Button>
		</section>
	);
}

function IconAction({
	icon: Icon,
	label,
	onClick,
	disabled,
}: Readonly<{
	icon: LucideIcon;
	label: string;
	onClick: () => void;
	disabled?: boolean;
}>) {
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<Button
					variant="ghost"
					size="icon"
					className="size-7 text-muted-foreground"
					aria-label={label}
					disabled={disabled}
					onClick={onClick}
				>
					<Icon className="size-3.5" />
				</Button>
			</TooltipTrigger>
			<TooltipContent>{label}</TooltipContent>
		</Tooltip>
	);
}

function CheckoutSection({
	projectPath,
	version,
	checkout,
	checkoutCount,
	builtAt,
	actions,
}: Readonly<{
	projectPath: string;
	version?: string | null;
	checkout?: MineCheckout;
	checkoutCount: number;
	builtAt?: number;
	actions: CheckoutActions;
}>) {
	const { t } = useTranslation("common");
	const revealLabel = useRevealLabel();
	const language = TEMPLATE_LANGUAGES.find(
		(candidate) => candidate.value === checkout?.language,
	);

	return (
		<WorkspaceSection
			icon={FolderOpen}
			title={t("workspaceCheckout", "Checkout")}
			action={
				<div className="flex shrink-0 items-center gap-0.5">
					<IconAction
						icon={Code2}
						label={t("openInEditor", "Open in editor")}
						onClick={() => actions.openEditor.mutate(projectPath)}
					/>
					<IconAction
						icon={FolderOpen}
						label={revealLabel}
						onClick={() => actions.reveal.mutate(projectPath)}
					/>
					<IconAction
						icon={RefreshCw}
						label={t("reloadIntoCatalog", "Reload into catalog")}
						disabled={actions.reload.isPending}
						onClick={() => actions.reload.mutate(projectPath)}
					/>
				</div>
			}
		>
			<FieldGrid>
				<Field label={t("workspaceFolder", "Folder")}>
					<span className="truncate font-mono text-xs" title={projectPath}>
						{projectPath}
					</span>
				</Field>
				{language && (
					<Field label={t("language", "Language")}>
						<img src={language.img} alt="" className="size-4 rounded-sm" />
						{language.label}
					</Field>
				)}
				<Field label={t("workspaceOnDiskLabel", "On disk")}>
					<span className="font-mono">{version || "—"}</span>
				</Field>
				<Field label={t("workspaceBuilt", "Built")}>
					{builtAt ? (
						<RelativeTime value={builtAt} />
					) : (
						<span className="text-muted-foreground">—</span>
					)}
				</Field>
			</FieldGrid>
			{checkout?.stale && (
				<p className="mt-3 flex items-center gap-1.5 text-xs font-medium text-tertiary">
					<AlertTriangle className="size-3.5 shrink-0" />
					{t(
						"wasmChangedReloadIntoTheCatalog",
						"WASM changed — reload into catalog",
					)}
				</p>
			)}
			{checkoutCount > 1 && (
				<p className="mt-3 text-xs text-muted-foreground">
					{t(
						"workspaceSeveralCheckouts",
						"{{count}} folders share this id. Publish and reload use the newest.",
						{ count: checkoutCount },
					)}
				</p>
			)}
		</WorkspaceSection>
	);
}

function ChecksSection({
	session,
	build,
}: Readonly<{ session: NodeDebuggerSession; build?: BuildArtifactInfo }>) {
	const { t } = useTranslation("common");
	const inspection = session.inspection;
	const checks = useMemo(
		() =>
			overviewChecks({
				build: session.isInspecting
					? null
					: {
							exists: Boolean(
								!session.error &&
									(inspection?.wasmPath || inspection?.widgetBundlePath),
							),
							sizeBytes: build?.sizeBytes,
							builtAt: build?.builtAt,
						},
				lint: inspection
					? {
							errors: session.lintCounts.errors,
							warnings: session.lintCounts.warnings,
						}
					: null,
			}),
		[
			session.isInspecting,
			session.error,
			session.lintCounts,
			inspection,
			build,
		],
	);
	return (
		<WorkspaceSection
			icon={ShieldCheck}
			title={t("workspaceChecks", "Checks")}
			action={<ReinspectButton session={session} />}
		>
			<WorkspaceChecks checks={checks} />
		</WorkspaceSection>
	);
}

function NodeChips({
	nodes,
	sign,
	className,
}: Readonly<{ nodes: DiffedNode[]; sign: string; className: string }>) {
	return (
		<ul className="flex flex-wrap gap-1.5">
			{nodes.map((node) => (
				<li
					key={node.name}
					title={node.name}
					className={`inline-flex items-center gap-1 rounded-md border px-1.5 py-0.5 text-xs ${className}`}
				>
					<span aria-hidden="true" className="font-mono">
						{sign}
					</span>
					{node.label}
				</li>
			))}
		</ul>
	);
}

function ChangesSection({
	session,
	entry,
	onOpenNodes,
}: Readonly<{
	session: NodeDebuggerSession;
	entry: RegistryEntry;
	onOpenNodes: () => void;
}>) {
	const { t } = useTranslation("common");
	const live = liveVersion(asArray(entry.versions))?.version;
	const remoteNodes = asArray(entry.nodes);
	const diff = useMemo(
		() => diffNodes(session.nodes, remoteNodes),
		[session.nodes, remoteNodes],
	);
	const groups = [
		{
			key: "added",
			label: t("workspaceNodesAdded", "Added"),
			nodes: diff.added,
			sign: "+",
			className:
				"border-emerald-500/30 bg-emerald-500/5 text-emerald-700 dark:text-emerald-400",
		},
		{
			key: "removed",
			label: t("workspaceNodesRemoved", "Removed"),
			nodes: diff.removed,
			sign: "−",
			className: "border-destructive/30 bg-destructive/5 text-destructive",
		},
		{
			key: "changed",
			label: t("workspaceNodesDescriptionChanged", "Description changed"),
			nodes: diff.changed,
			sign: "~",
			className: "border-tertiary/40 bg-tertiary/5 text-tertiary",
		},
	].filter((group) => group.nodes.length > 0);

	return (
		<WorkspaceSection
			icon={GitCompare}
			title={
				live
					? t("workspaceChangesSince", "Changes since {{version}}", {
							version: live,
						})
					: t("workspaceChangesAgainstRegistry", "Changes against the registry")
			}
			action={
				<WorkspaceLinkButton onClick={onOpenNodes}>
					{t("workspaceOpenNodesTab", "Open Nodes tab")}
					<ArrowRight className="size-3.5" />
				</WorkspaceLinkButton>
			}
		>
			{session.isInspecting ? (
				<Skeleton className="h-10 w-full" />
			) : !session.inspection ? (
				<p className="text-sm text-muted-foreground">
					{t(
						"workspaceBuildToCompare",
						"Build the package to compare its nodes with the published version.",
					)}
				</p>
			) : !hasNodeChanges(diff) ? (
				<p className="flex items-center gap-2 text-sm">
					<CheckCircle2 className="size-4 shrink-0 text-emerald-600 dark:text-emerald-400" />
					{live
						? t(
								"workspaceNoNodeChangesSince",
								"No node changes since {{version}}",
								{
									version: live,
								},
							)
						: t("workspaceNoNodeChanges", "No node changes")}
				</p>
			) : (
				<div className="space-y-3">
					{groups.map((group) => (
						<div key={group.key} className="space-y-1.5">
							<p className="text-xs font-medium text-muted-foreground">
								{`${group.label} · ${group.nodes.length}`}
							</p>
							<NodeChips
								nodes={group.nodes}
								sign={group.sign}
								className={group.className}
							/>
						</div>
					))}
				</div>
			)}
		</WorkspaceSection>
	);
}

function LocalNodesPreview({
	session,
	onOpenNodes,
}: Readonly<{ session: NodeDebuggerSession; onOpenNodes: () => void }>) {
	const { t } = useTranslation("common");
	const nodes = session.nodes;
	return (
		<WorkspaceSection
			icon={FileCode}
			title={t("nodes", "Nodes")}
			bodyClassName="-mx-4 -mb-3.5"
			action={
				nodes.length > 0 && (
					<WorkspaceLinkButton onClick={onOpenNodes}>
						{t("workspaceOpenNodesTab", "Open Nodes tab")}
						<ArrowRight className="size-3.5" />
					</WorkspaceLinkButton>
				)
			}
		>
			{session.isInspecting ? (
				<div className="border-t border-border/60 px-4 py-3">
					<Skeleton className="h-10 w-full" />
				</div>
			) : nodes.length === 0 ? (
				<p className="border-t border-border/60 px-4 py-3 text-sm text-muted-foreground">
					{t(
						"workspaceNoLocalNodes",
						"No nodes in this build yet. Build the package, then re-inspect it.",
					)}
				</p>
			) : (
				<ul>
					{nodes.slice(0, PREVIEW_NODE_COUNT).map((node) => (
						<li
							key={node.name}
							className="grid h-10 grid-cols-[minmax(6rem,auto)_minmax(0,1fr)] items-center gap-3 border-t border-border/60 px-4 text-sm"
						>
							<span className="truncate font-medium">
								{node.friendly_name || node.name}
							</span>
							<span className="truncate text-muted-foreground">
								{node.description}
							</span>
						</li>
					))}
				</ul>
			)}
		</WorkspaceSection>
	);
}
