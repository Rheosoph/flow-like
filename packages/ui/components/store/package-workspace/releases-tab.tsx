"use client";

import { useTranslation } from "@flow-like/locales";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Download, History, Loader2, Rocket } from "lucide-react";
import type { ReactNode } from "react";
import { toast } from "sonner";
import { getErrorMessage } from "../../../lib/error-message";
import { asArray } from "../../../lib/response-shape";
import {
	PackageStatus,
	type PackageVersion,
	type RegistryEntry,
} from "../../../lib/schema/wasm";
import { useBackend } from "../../../state/backend-state";
import type { GenericFetcher } from "../../pages/store/store-package-detail";
import { Badge, Button, RelativeTime } from "../../ui";
import { PackageDangerZoneCard, PackageRestoreCard } from "./lifecycle-cards";
import { PublicationReviewCard } from "./publication-cards";
import { liveVersion, pendingVersion } from "./workspace-model";
import { CountBadge, WorkspaceSection } from "./workspace-parts";

export interface ReleasesTabProps {
	entry: RegistryEntry;
	isOwner: boolean;
	canInstall: boolean;
	publishAction?: ReactNode;
	diskVersion?: string;
	fetcher: GenericFetcher;
	auth?: unknown;
	onDeleted: () => void;
}

export function ReleasesTab({
	entry,
	isOwner,
	canInstall,
	publishAction,
	diskVersion,
	fetcher,
	auth,
	onDeleted,
}: Readonly<ReleasesTabProps>) {
	const { t } = useTranslation("common");
	const versions = asArray(entry.versions);
	const live = liveVersion(versions);
	const showPublicationAudit =
		entry.status !== PackageStatus.Active || !!pendingVersion(versions);
	const packageName = entry.manifest.name || entry.id;

	return (
		<div className="grid items-start gap-5 lg:grid-cols-[minmax(0,2fr)_minmax(0,1fr)]">
			<div className="flex min-w-0 flex-col gap-4">
				{publishAction && (
					<WorkspaceSection
						icon={Rocket}
						title={t("workspaceNextRelease", "Next release")}
						action={publishAction}
					>
						<p className="text-sm text-muted-foreground">
							{diskVersion
								? t(
										"workspaceNextReleaseFromDisk",
										"Version {{version}} is on disk. Publishing sends it to review before it goes live.",
										{ version: diskVersion },
									)
								: t(
										"workspaceNextReleaseGeneric",
										"Publishing sends a new version to review before it goes live.",
									)}
						</p>
					</WorkspaceSection>
				)}
				<VersionsSection
					packageId={entry.id}
					versions={versions}
					liveVersion={live?.version}
					canInstall={canInstall}
				/>
			</div>
			<div className="flex min-w-0 flex-col gap-4">
				{showPublicationAudit && (
					<PublicationReviewCard
						packageId={entry.id}
						status={entry.status}
						fetcher={fetcher}
						auth={auth}
					/>
				)}
				{isOwner && entry.status === PackageStatus.Disabled && (
					<PackageRestoreCard
						packageId={entry.id}
						fetcher={fetcher}
						auth={auth}
					/>
				)}
				{isOwner && entry.status !== PackageStatus.Disabled && (
					<PackageDangerZoneCard
						packageId={entry.id}
						packageName={packageName}
						fetcher={fetcher}
						auth={auth}
						onDeleted={onDeleted}
					/>
				)}
			</div>
		</div>
	);
}

function isInstallable(version: PackageVersion): boolean {
	return (
		!version.yanked &&
		version.status !== PackageStatus.Rejected &&
		version.status !== PackageStatus.Disabled
	);
}

function VersionsSection({
	packageId,
	versions,
	liveVersion,
	canInstall,
}: Readonly<{
	packageId: string;
	versions: PackageVersion[];
	liveVersion?: string;
	canInstall: boolean;
}>) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const queryClient = useQueryClient();

	const installed = useQuery({
		queryKey: ["installed-package", packageId],
		queryFn: () => backend.registryState.getInstalledVersion(packageId),
		enabled: canInstall,
	});

	const install = useMutation({
		mutationFn: (version: string) =>
			backend.registryState.installPackage(packageId, version),
		onSuccess: () => {
			toast.success(
				t("workspaceInstalledForTesting", "Installed on this machine"),
			);
			queryClient.invalidateQueries({
				queryKey: ["installed-package", packageId],
			});
		},
		onError: (error: unknown) =>
			toast.error(
				t("workspaceInstallFailed", "Install failed: {{message}}", {
					message: getErrorMessage(error),
				}),
			),
	});

	return (
		<WorkspaceSection
			icon={History}
			title={
				<span className="flex items-center gap-2">
					{t("versions", "Versions")}
					<CountBadge value={versions.length} />
				</span>
			}
			bodyClassName="-mx-4 -mb-3.5"
		>
			{versions.length === 0 ? (
				<p className="border-t border-border/60 px-4 py-3 text-sm text-muted-foreground">
					{t("workspaceNoVersions", "No versions published yet.")}
				</p>
			) : (
				<ul>
					{versions.map((version) => {
						const isPending = version.status === PackageStatus.PendingReview;
						const isInstalled = installed.data === version.version;
						const installingThis =
							install.isPending && install.variables === version.version;
						return (
							<li
								key={version.version}
								className="flex flex-wrap items-center gap-x-3 gap-y-1 border-t border-border/60 px-4 py-2.5"
							>
								<code className="font-mono text-sm">{version.version}</code>
								<VersionBadges
									version={version}
									isLive={version.version === liveVersion}
								/>
								<span className="flex-1" />
								<RelativeTime
									className="text-xs text-muted-foreground"
									value={version.publishedAt}
								/>
								{canInstall && isInstallable(version) && (
									<Button
										size="sm"
										variant={isInstalled ? "secondary" : "outline"}
										disabled={isInstalled || install.isPending}
										onClick={() => install.mutate(version.version)}
									>
										{installingThis ? (
											<Loader2 className="size-3.5 animate-spin" />
										) : isInstalled ? (
											<Check className="size-3.5" />
										) : (
											<Download className="size-3.5" />
										)}
										{isInstalled
											? t("installed", "Installed")
											: isPending
												? t("workspaceInstallForTesting", "Install for testing")
												: t("install", "Install")}
									</Button>
								)}
								{version.releaseNotes && (
									<p className="basis-full whitespace-pre-wrap text-xs text-muted-foreground">
										{version.releaseNotes}
									</p>
								)}
							</li>
						);
					})}
				</ul>
			)}
		</WorkspaceSection>
	);
}

function VersionBadges({
	version,
	isLive,
}: Readonly<{ version: PackageVersion; isLive: boolean }>) {
	const { t } = useTranslation("common");
	return (
		<>
			{isLive && <Badge variant="secondary">{t("live", "Live")}</Badge>}
			{version.status === PackageStatus.PendingReview && (
				<Badge variant="outline">{t("inReview", "In review")}</Badge>
			)}
			{version.status === PackageStatus.Rejected && (
				<Badge variant="destructive">{t("rejected", "Rejected")}</Badge>
			)}
			{version.status === PackageStatus.Disabled && (
				<Badge variant="secondary">{t("disabled", "Disabled")}</Badge>
			)}
			{version.yanked && (
				<Badge variant="destructive">{t("workspaceYanked", "Yanked")}</Badge>
			)}
		</>
	);
}
