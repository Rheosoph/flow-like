"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AlertTriangle,
	ArrowRight,
	CheckCircle2,
	ExternalLink,
	FileCode,
	Server,
	ShieldCheck,
	Sparkles,
	Star,
} from "lucide-react";
import Link from "next/link";
import { type ReactNode, useMemo } from "react";
import { readManifestAccess } from "../../../lib/app-package-overview";
import { usePackageCapabilities } from "../../../lib/package-capabilities";
import { asArray } from "../../../lib/response-shape";
import type { PackageMeta, RegistryEntry } from "../../../lib/schema/wasm";
import {
	userAvatarUrl,
	userDisplayName,
	userInitials,
} from "../../../lib/user-display";
import type { GenericFetcher } from "../../pages/store/store-package-detail";
import {
	Avatar,
	AvatarFallback,
	AvatarImage,
	RelativeTime,
	Skeleton,
} from "../../ui";
import { useAccessLabels } from "../app-packages/parts";
import { usePackageUsers } from "./use-workspace-data";
import type { WorkspaceTab } from "./workspace-href";
import {
	type ListingField,
	type WorkspaceBanner,
	listingHealth,
	liveVersion,
	pendingVersion,
	peopleCounts,
} from "./workspace-model";
import {
	Field,
	FieldGrid,
	LiveVersionPill,
	VisibilityLabel,
	WorkspaceLinkButton,
	WorkspaceSection,
} from "./workspace-parts";

const PREVIEW_NODE_COUNT = 6;
const PREVIEW_PEOPLE_COUNT = 3;

export interface OverviewTabProps {
	entry?: RegistryEntry;
	meta?: PackageMeta | null;
	metaLoading?: boolean;
	banner?: WorkspaceBanner;
	main?: ReactNode;
	aside?: ReactNode;
	storeHref?: string;
	fetcher: GenericFetcher;
	auth?: unknown;
	onSelectTab: (tab: WorkspaceTab) => void;
}

export function OverviewTab({
	entry,
	meta,
	metaLoading,
	banner,
	main,
	aside,
	storeHref,
	fetcher,
	auth,
	onSelectTab,
}: Readonly<OverviewTabProps>) {
	return (
		<div className="grid items-start gap-5 lg:grid-cols-[minmax(0,2fr)_minmax(0,1fr)]">
			<div className="flex min-w-0 flex-col gap-4">
				{main}
				{!main && entry && (
					<NodesPreview entry={entry} onOpen={() => onSelectTab("nodes")} />
				)}
			</div>
			<div className="flex min-w-0 flex-col gap-4">
				{aside}
				{entry ? (
					<RegistryCard
						entry={entry}
						storeHref={storeHref}
						fetcher={fetcher}
						auth={auth}
						onManagePeople={() => onSelectTab("access")}
					/>
				) : (
					!banner && <NotPublishedCard />
				)}
				{entry && (
					<ListingHealthCard
						entry={entry}
						meta={meta}
						loading={metaLoading}
						onFinish={() => onSelectTab("listing")}
					/>
				)}
				{entry && <PermissionsCard manifest={entry.manifest} />}
			</div>
		</div>
	);
}

function NodesPreview({
	entry,
	onOpen,
}: Readonly<{ entry: RegistryEntry; onOpen: () => void }>) {
	const { t } = useTranslation("common");
	const nodes = asArray(entry.nodes);
	return (
		<WorkspaceSection
			icon={FileCode}
			title={t("nodes", "Nodes")}
			bodyClassName="-mx-4 -mb-3.5"
			action={
				nodes.length > 0 && (
					<WorkspaceLinkButton onClick={onOpen}>
						{t("workspaceOpenNodesTab", "Open Nodes tab")}
						<ArrowRight className="size-3.5" />
					</WorkspaceLinkButton>
				)
			}
		>
			{nodes.length === 0 ? (
				<p className="border-t border-border/60 px-4 py-3 text-sm text-muted-foreground">
					{t(
						"workspaceNoRegistryNodes",
						"The registry lists no nodes for this package yet.",
					)}
				</p>
			) : (
				<ul>
					{nodes.slice(0, PREVIEW_NODE_COUNT).map((node) => (
						<li
							key={node.id}
							className="grid h-10 grid-cols-[minmax(6rem,auto)_minmax(0,1fr)] items-center gap-3 border-t border-border/60 px-4 text-sm"
						>
							<span className="truncate font-medium">
								{node.friendlyName || node.name}
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

function RegistryCard({
	entry,
	storeHref,
	fetcher,
	auth,
	onManagePeople,
}: Readonly<{
	entry: RegistryEntry;
	storeHref?: string;
	fetcher: GenericFetcher;
	auth?: unknown;
	onManagePeople: () => void;
}>) {
	const { t } = useTranslation("common");
	const versions = asArray(entry.versions);
	const live = liveVersion(versions);
	const pending = pendingVersion(versions);
	const rated = (entry.ratingCount ?? 0) > 0;

	return (
		<WorkspaceSection
			icon={Server}
			title={t("workspaceRegistry", "Registry")}
			action={
				storeHref && (
					<Link
						href={storeHref}
						className="inline-flex shrink-0 items-center gap-1 rounded-sm text-xs font-medium text-primary outline-none hover:text-primary/80 focus-visible:ring-2 focus-visible:ring-ring"
					>
						{t("viewStorePage", "View store page")}
						<ExternalLink className="size-3.5" />
					</Link>
				)
			}
		>
			<FieldGrid>
				<Field label={t("live", "Live")}>
					{live ? (
						<LiveVersionPill version={live.version} />
					) : (
						<span className="text-muted-foreground">
							{t("workspaceNotLiveYet", "Not live yet")}
						</span>
					)}
					<VisibilityLabel
						visibility={entry.visibility}
						className="text-muted-foreground"
					/>
				</Field>
				{pending && (
					<Field label={t("inReview", "In review")}>
						<span className="font-mono">{pending.version}</span>
					</Field>
				)}
				<Field label={t("installs", "Installs")}>
					<span className="tabular-nums">
						{(entry.downloadCount ?? 0).toLocaleString()}
					</span>
				</Field>
				<Field label={t("rating", "Rating")}>
					{rated ? (
						<span className="flex items-center gap-1.5 tabular-nums">
							<Star className="size-3.5 fill-yellow-500 text-yellow-500" />
							{(entry.avgRating ?? 0).toFixed(1)}
							<span className="text-muted-foreground">{`(${entry.ratingCount})`}</span>
						</span>
					) : (
						<span className="text-muted-foreground">
							{t("workspaceNoRatingsYet", "No ratings yet")}
						</span>
					)}
				</Field>
				<Field label={t("workspacePublished", "Published")}>
					{live ? (
						<RelativeTime value={live.publishedAt} />
					) : (
						<span className="text-muted-foreground">—</span>
					)}
				</Field>
				<Field label={t("workspacePeople", "People")}>
					<PeopleSummary
						packageId={entry.id}
						fetcher={fetcher}
						auth={auth}
						onManage={onManagePeople}
					/>
				</Field>
			</FieldGrid>
		</WorkspaceSection>
	);
}

function PeopleSummary({
	packageId,
	fetcher,
	auth,
	onManage,
}: Readonly<{
	packageId: string;
	fetcher: GenericFetcher;
	auth?: unknown;
	onManage: () => void;
}>) {
	const { t } = useTranslation("common");
	const users = usePackageUsers(packageId, fetcher, auth);
	const people = asArray(users.data);
	const counts = peopleCounts(people);

	if (users.isLoading) return <Skeleton className="h-5 w-32" />;
	if (users.isError) {
		return (
			<span className="text-muted-foreground">
				{t("workspacePeopleUnavailable", "Couldn't load people")}
			</span>
		);
	}

	const roles = [
		counts.owners > 0 &&
			t("workspaceOwnerCount", {
				defaultValue_one: "{{count}} Owner",
				defaultValue_other: "{{count}} Owners",
				count: counts.owners,
			}),
		counts.maintainers > 0 &&
			t("workspaceMaintainerCount", {
				defaultValue_one: "{{count}} Maintainer",
				defaultValue_other: "{{count}} Maintainers",
				count: counts.maintainers,
			}),
		counts.users > 0 &&
			t("workspaceUserCount", {
				defaultValue_one: "{{count}} User",
				defaultValue_other: "{{count}} Users",
				count: counts.users,
			}),
		counts.buyers > 0 &&
			t("workspaceBuyerCount", {
				defaultValue_one: "{{count}} Buyer",
				defaultValue_other: "{{count}} Buyers",
				count: counts.buyers,
			}),
	].filter((role): role is string => typeof role === "string");

	return (
		<button
			type="button"
			onClick={onManage}
			className="flex min-w-0 items-center gap-2 rounded-sm text-left outline-none hover:text-primary focus-visible:ring-2 focus-visible:ring-ring"
		>
			<span className="flex shrink-0">
				{people.slice(0, PREVIEW_PEOPLE_COUNT).map((person, index) => {
					const avatar = userAvatarUrl(person);
					return (
						<Avatar
							key={person.id}
							className={
								index === 0
									? "size-5.5 border-2 border-card"
									: "-ml-1.5 size-5.5 border-2 border-card"
							}
						>
							{avatar ? (
								<AvatarImage src={avatar} alt={userDisplayName(person)} />
							) : null}
							<AvatarFallback className="text-[9px] font-semibold">
								{userInitials(person)}
							</AvatarFallback>
						</Avatar>
					);
				})}
			</span>
			<span className="truncate">
				{roles.length === 0
					? t("workspaceNoPeople", "Nobody yet")
					: roles.join(" · ")}
			</span>
		</button>
	);
}

function NotPublishedCard() {
	const { t } = useTranslation("common");
	return (
		<WorkspaceSection icon={Server} title={t("workspaceRegistry", "Registry")}>
			<p className="text-sm text-muted-foreground">
				{t(
					"workspaceNotPublishedYetDescription",
					"Not published yet. Publish to get a store page, installs and reviews.",
				)}
			</p>
		</WorkspaceSection>
	);
}

function ListingHealthCard({
	entry,
	meta,
	loading,
	onFinish,
}: Readonly<{
	entry: RegistryEntry;
	meta?: PackageMeta | null;
	loading?: boolean;
	onFinish: () => void;
}>) {
	const { t } = useTranslation("common");
	const health = useMemo(() => listingHealth(entry, meta), [entry, meta]);
	const copy: Record<ListingField, { title: string; hint: string }> = {
		thumbnail: {
			title: t("workspaceThumbnailMissing", "Thumbnail missing"),
			hint: t(
				"workspaceThumbnailMissingHint",
				"Explore shows a plain tile until you add one.",
			),
		},
		description: {
			title: t("workspaceDescriptionMissing", "Short description missing"),
			hint: t(
				"workspaceDescriptionMissingHint",
				"Cards show no summary under the name.",
			),
		},
		keywords: {
			title: t("workspaceKeywordsMissing", "No keywords"),
			hint: t(
				"workspaceKeywordsMissingHint",
				"Search can only match the package name.",
			),
		},
	};

	return (
		<WorkspaceSection
			icon={Sparkles}
			title={t("workspaceListingHealth", "Listing health")}
			action={
				!loading &&
				!health.complete && (
					<WorkspaceLinkButton onClick={onFinish}>
						{t("workspaceFinishListing", "Finish listing")}
						<ArrowRight className="size-3.5" />
					</WorkspaceLinkButton>
				)
			}
		>
			{loading ? (
				<div className="space-y-2">
					<Skeleton className="h-4 w-40" />
					<Skeleton className="h-3 w-56" />
				</div>
			) : health.complete ? (
				<p className="flex items-center gap-2 text-sm">
					<CheckCircle2 className="size-4 shrink-0 text-emerald-600 dark:text-emerald-400" />
					{t("workspaceListingComplete", "Listing complete")}
				</p>
			) : (
				<ul className="flex flex-col gap-2.5 text-sm">
					{health.missing.map((field) => (
						<li key={field} className="flex items-start gap-2">
							<AlertTriangle className="mt-0.5 size-4 shrink-0 text-tertiary" />
							<div className="min-w-0">
								<div>{copy[field].title}</div>
								<div className="text-xs text-muted-foreground">
									{copy[field].hint}
								</div>
							</div>
						</li>
					))}
				</ul>
			)}
		</WorkspaceSection>
	);
}

function PermissionsCard({
	manifest,
}: Readonly<{ manifest: RegistryEntry["manifest"] }>) {
	const { t } = useTranslation("common");
	const access = useMemo(() => readManifestAccess(manifest), [manifest]);
	const capabilities = usePackageCapabilities(access?.capabilityTags);
	const accessLabels = useAccessLabels();

	return (
		<WorkspaceSection
			icon={ShieldCheck}
			title={t("workspacePermissionsAndLimits", "Permissions & limits")}
		>
			<FieldGrid>
				<Field label={t("permissions", "Permissions")}>
					{capabilities.length === 0 ? (
						<span>
							{t("workspaceNoPermissions", "None")}
							<span className="text-muted-foreground">
								{` · ${t("workspacePureCompute", "pure compute")}`}
							</span>
						</span>
					) : (
						<span className="flex min-w-0 flex-wrap gap-1">
							{capabilities.map((capability) => (
								<span
									key={capability.key}
									title={capability.label}
									className={
										capability.severity === "elevated"
											? "rounded border border-primary/35 bg-primary/10 px-1.5 py-0.5 font-mono text-[11px] text-primary"
											: "rounded border border-border/60 bg-muted/40 px-1.5 py-0.5 font-mono text-[11px] text-muted-foreground"
									}
								>
									{capability.key}
								</span>
							))}
						</span>
					)}
				</Field>
				<Field label={t("memory", "Memory")}>
					{accessLabels.memory(access?.memory) ?? "—"}
				</Field>
				<Field label={t("timeout", "Timeout")}>
					{accessLabels.timeout(access?.timeout) ?? "—"}
				</Field>
				{(access?.allowedHosts.length ?? 0) > 0 && (
					<Field label={t("workspaceHosts", "Hosts")}>
						<span className="truncate font-mono text-xs">
							{access?.allowedHosts.join(", ")}
						</span>
					</Field>
				)}
			</FieldGrid>
		</WorkspaceSection>
	);
}
