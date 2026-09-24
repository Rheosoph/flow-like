"use client";

import { Button, cn } from "@flow-like/flow-like-ui";
import {
	PackageWorkspace,
	type PackageWorkspaceLocal,
} from "@flow-like/flow-like-ui/components/store/package-workspace/package-workspace";
import { useRegistryPackage } from "@flow-like/flow-like-ui/components/store/package-workspace/use-registry-package";
import {
	type WorkspaceTab,
	workspaceTabFromParam,
} from "@flow-like/flow-like-ui/components/store/package-workspace/workspace-href";
import { workspaceAccess } from "@flow-like/flow-like-ui/components/store/package-workspace/workspace-model";
import { StatePill } from "@flow-like/flow-like-ui/components/store/package-workspace/workspace-parts";
import { isMaintainer } from "@flow-like/flow-like-ui/lib/permission/wasm-package-permission";
import { TEMPLATE_LANGUAGES } from "@flow-like/flow-like-ui/lib/schema/developer";
import { useTranslation } from "@flow-like/locales";
import { useQueryClient } from "@tanstack/react-query";
import { FolderPlus, Loader2 } from "lucide-react";
import Link from "next/link";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { useCallback, useEffect, useMemo } from "react";
import { useAuth } from "react-oidc-context";
import { fetcher } from "../../lib/api";
import { lintCounts, projectRoutes } from "./local-projects";
import { MINE_STATE_TONE, useMineFilterLabels } from "./mine-labels";
import {
	type MineAction,
	type MineEntry,
	type MineState,
	isPlaceholderId,
	mineActionTab,
	workspaceHeaderAction,
} from "./mine-model";
import {
	type CheckoutActions,
	MineActionContent,
	useCheckoutActions,
	useMineActionView,
} from "./mine-package-card";
import { mineQueryKeys } from "./use-mine-packages";
import { type WorkspaceEntry, useWorkspaceEntry } from "./use-workspace-entry";
import { LinkFolderNotice } from "./workspace/link-folder";
import { LocalOverview, LocalOverviewChecks } from "./workspace/local-overview";
import { LocalNodesTab, LocalTestTab } from "./workspace/local-tabs";
import { ManifestEditor } from "./workspace/manifest-editor";
import {
	type NodeDebuggerSession,
	useNodeDebugger,
} from "./workspace/node-debugger";

function useWorkspaceTab() {
	const router = useRouter();
	const pathname = usePathname();
	const searchParams = useSearchParams();
	const select = useCallback(
		(tab: WorkspaceTab) => {
			const params = new URLSearchParams(searchParams.toString());
			if (tab === "overview") params.delete("tab");
			else params.set("tab", tab);
			const query = params.toString();
			router.replace(query ? `${pathname}?${query}` : pathname, {
				scroll: false,
			});
		},
		[pathname, router, searchParams],
	);
	return { active: workspaceTabFromParam(searchParams.get("tab")), select };
}

/** Mine lint for the checkout comes from the workspace's own inspection instead of a second one. */
function useSeedMineLint(
	workspace: WorkspaceEntry,
	session: NodeDebuggerSession,
) {
	const queryClient = useQueryClient();
	const listedPath = workspace.checkout?.path;
	const { requestInspection } = workspace.mine;
	const inspection = session.inspection;
	useEffect(() => {
		if (!listedPath || !inspection) return;
		queryClient.setQueryData(
			mineQueryKeys.lint(listedPath),
			lintCounts(inspection),
		);
		requestInspection(listedPath);
	}, [listedPath, inspection, queryClient, requestInspection]);
}

function MineStatePill({ state }: Readonly<{ state: MineState }>) {
	const labels = useMineFilterLabels();
	const tone = MINE_STATE_TONE[state];
	return (
		<StatePill tone={{ pill: cn("bg-muted/60", tone.text), dot: tone.dot }}>
			{labels[state]}
		</StatePill>
	);
}

const PLACEHOLDER_TONE = {
	pill: "bg-tertiary/10 text-tertiary",
	dot: "bg-tertiary",
};

function PlaceholderStatePill() {
	const { t } = useTranslation("common");
	return (
		<StatePill tone={PLACEHOLDER_TONE}>
			{t("placeholderIdTitle", "Placeholder id")}
		</StatePill>
	);
}

/** True when the shell will show its "This id is taken" banner for this checkout. */
function useIdTakenBanner(
	packageId: string | undefined,
	registry: ReturnType<typeof useRegistryPackage>,
): boolean {
	const permission = registry.entry?.currentUserPermission;
	return useMemo(() => {
		const access = workspaceAccess({
			packageId,
			hasLocal: true,
			auth: registry.authState,
			remote: { status: registry.status, source: registry.source, permission },
		});
		return access.mode === "local" && access.banner === "id_taken";
	}, [
		packageId,
		registry.authState,
		registry.status,
		registry.source,
		permission,
	]);
}

function WorkspacePrimaryAction({
	entry,
	action,
	actions,
	onSelectTab,
}: Readonly<{
	entry: MineEntry;
	action: MineAction;
	actions: CheckoutActions;
	onSelectTab: (tab: WorkspaceTab) => void;
}>) {
	const view = useMineActionView(entry, action);
	if (!view) return null;
	if (action.kind === "reload") {
		return (
			<Button
				variant="secondary"
				disabled={actions.reload.isPending}
				onClick={() => actions.reload.mutate(action.path)}
			>
				<MineActionContent view={view} busy={actions.reload.isPending} />
			</Button>
		);
	}
	if (action.kind === "publish") {
		return (
			<Button asChild variant={view.emphasis ? "default" : "secondary"}>
				<Link href={projectRoutes.publish(action.path)}>
					<MineActionContent view={view} />
				</Link>
			</Button>
		);
	}
	const tab = mineActionTab(action);
	return (
		<Button variant="secondary" onClick={() => tab && onSelectTab(tab)}>
			<MineActionContent view={view} />
		</Button>
	);
}

function AddFolderAction({
	workspace,
	checkoutPath,
}: Readonly<{ workspace: WorkspaceEntry; checkoutPath: string }>) {
	const { t } = useTranslation("common");
	const { mine } = workspace;
	return (
		<Button
			variant="secondary"
			disabled={mine.isAddingFolder}
			onClick={() => mine.addFolder(checkoutPath)}
		>
			{mine.isAddingFolder ? (
				<Loader2 className="animate-spin" />
			) : (
				<FolderPlus />
			)}
			{t("addToPackages", "Add to packages")}
		</Button>
	);
}

function folderName(path: string): string {
	return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

export interface DesktopPackageWorkspaceProps {
	id?: string;
	project?: string;
}

/**
 * The shared package workspace with this machine's checkout plugged in: the
 * debugger's nodes and lint, the node runner and widget tester, the manifest
 * editor, and an Overview with the checkout, its checks and the node changes
 * since the live version. Without a checkout, Test and Manifest offer "Link
 * folder…".
 */
export function DesktopPackageWorkspace({
	id,
	project,
}: Readonly<DesktopPackageWorkspaceProps>) {
	const auth = useAuth();
	const workspace = useWorkspaceEntry({ id, project });
	const { entry, checkout, checkoutPath, manifest, packageId } = workspace;
	const session = useNodeDebugger({ projectPath: checkoutPath });
	const actions = useCheckoutActions(workspace.mine.invalidateProject);
	const { active: activeTab, select: selectTab } = useWorkspaceTab();
	useSeedMineLint(workspace, session);

	const registry = useRegistryPackage(packageId, fetcher, auth, {
		localFallback: false,
	});
	const maintainedEntry =
		registry.source === "registry" &&
		registry.entry &&
		isMaintainer(registry.entry.currentUserPermission ?? 0)
			? registry.entry
			: undefined;
	const idKept = !!entry?.registry || !!maintainedEntry;
	const placeholder = !idKept && isPlaceholderId(packageId);
	const idTakenBanner = useIdTakenBanner(packageId, registry);
	const headerAction =
		entry && workspace.listed && checkoutPath
			? workspaceHeaderAction(entry, checkoutPath, activeTab)
			: null;

	const language = TEMPLATE_LANGUAGES.find(
		(candidate) => candidate.value === (checkout?.language ?? entry?.language),
	);

	const local: PackageWorkspaceLocal | undefined = checkoutPath
		? {
				header: {
					name:
						entry?.name || manifest?.name?.trim() || folderName(checkoutPath),
					id: packageId,
					iconSrc: language?.img,
					version: manifest?.version ?? checkout?.version ?? undefined,
					badges: language && (
						<span className="inline-flex h-5.5 shrink-0 items-center rounded-full border border-border/70 px-2 text-xs text-muted-foreground">
							{language.label}
						</span>
					),
					state: placeholder ? (
						<PlaceholderStatePill />
					) : entry?.registry ? (
						<MineStatePill state={entry.state} />
					) : undefined,
					actions: !workspace.listed && (
						<AddFolderAction
							workspace={workspace}
							checkoutPath={checkoutPath}
						/>
					),
				},
				primaryAction:
					entry && headerAction ? (
						<WorkspacePrimaryAction
							entry={entry}
							action={headerAction}
							actions={actions}
							onSelectTab={selectTab}
						/>
					) : undefined,
				overview: {
					main: (
						<LocalOverview
							projectPath={checkoutPath}
							packageId={packageId}
							placeholder={placeholder && !idTakenBanner}
							version={manifest?.version ?? checkout?.version}
							checkout={checkout}
							checkoutCount={entry?.checkouts.length ?? 1}
							session={session}
							registryEntry={maintainedEntry}
							actions={actions}
							onSelectTab={selectTab}
						/>
					),
					aside: <LocalOverviewChecks session={session} />,
				},
				nodes: (
					<LocalNodesTab session={session} onRun={() => selectTab("test")} />
				),
				nodeCount: session.inspection ? session.nodes.length : undefined,
				test: <LocalTestTab session={session} projectPath={checkoutPath} />,
				manifest: (
					<ManifestEditor
						key={checkoutPath}
						projectPath={checkoutPath}
						flagPlaceholderId={!idKept}
					/>
				),
			}
		: undefined;

	return (
		<PackageWorkspace
			packageId={packageId}
			fetcher={fetcher}
			auth={auth}
			local={local}
			localPending={workspace.pending}
			desktop={{
				linkFolder: <LinkFolderNotice packageId={packageId} />,
				canInstall: true,
			}}
		/>
	);
}
