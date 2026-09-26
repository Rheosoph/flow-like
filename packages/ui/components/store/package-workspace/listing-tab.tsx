"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CheckCircle2,
	Download,
	Eye,
	ImageIcon,
	type LucideIcon,
	PartyPopper,
	PenLine,
	Star,
	Tag,
	X,
} from "lucide-react";
import {
	type RefObject,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { readManifestAccess } from "../../../lib/app-package-overview";
import {
	type PackageMeta,
	PackageStatus,
	type RegistryEntry,
} from "../../../lib/schema/wasm";
import type { GenericFetcher } from "../../pages/store/store-package-detail";
import {
	Avatar,
	AvatarFallback,
	AvatarImage,
	Button,
	Skeleton,
} from "../../ui";
import { PackageCard, getPackageInitials } from "../package-card";
import { PackageMetaTab } from "../package-meta-tab";
import { PackagePricingCard } from "../package-pricing-card";
import { PublicationRequestCard } from "./publication-cards";
import {
	type ListingField,
	humanizeKey,
	listingHealth,
	storePreviewSummary,
} from "./workspace-model";
import { WorkspaceSection } from "./workspace-parts";

/** Element ids a fix button targets in `PackageMetaTab`; a field only gets a button once its id renders. */
export const LISTING_FIELD_ANCHOR: Record<ListingField, string> = {
	thumbnail: "meta-thumbnail-upload",
	description: "meta-description",
	keywords: "meta-tags",
};

const LISTING_FIELDS = Object.keys(LISTING_FIELD_ANCHOR) as ListingField[];

function fieldAnchor(
	form: HTMLElement | null,
	field: ListingField,
): HTMLElement | null {
	return (
		form?.querySelector<HTMLElement>(`#${LISTING_FIELD_ANCHOR[field]}`) ?? null
	);
}

/** The fields whose anchor is in the rendered form, kept current while the form loads. */
function useAnchoredFields(
	form: RefObject<HTMLElement | null>,
	enabled: boolean,
): ReadonlySet<ListingField> {
	const [anchored, setAnchored] = useState<ReadonlySet<ListingField>>(
		() => new Set(),
	);
	useEffect(() => {
		const node = form.current;
		if (!enabled || !node) return;
		const read = () => {
			const next = LISTING_FIELDS.filter((field) => fieldAnchor(node, field));
			setAnchored((current) =>
				current.size === next.length &&
				next.every((field) => current.has(field))
					? current
					: new Set(next),
			);
		};
		read();
		const observer = new MutationObserver(read);
		observer.observe(node, { childList: true, subtree: true });
		return () => observer.disconnect();
	}, [form, enabled]);
	return anchored;
}

export interface ListingTabProps {
	entry: RegistryEntry;
	meta?: PackageMeta | null;
	metaLoading?: boolean;
	isOwner: boolean;
	published: boolean;
	onDismissPublished: () => void;
	fetcher: GenericFetcher;
	auth?: unknown;
}

export function ListingTab({
	entry,
	meta,
	metaLoading,
	isOwner,
	published,
	onDismissPublished,
	fetcher,
	auth,
}: Readonly<ListingTabProps>) {
	const showPricing =
		isOwner &&
		entry.visibility !== "local" &&
		entry.status !== PackageStatus.Disabled;
	const showPublicationRequest =
		isOwner &&
		entry.visibility === "private" &&
		entry.status === PackageStatus.Active;
	const formRef = useRef<HTMLDivElement>(null);
	const showBanner = published && !metaLoading;
	const anchored = useAnchoredFields(formRef, showBanner);

	const goToField = useCallback((field: ListingField) => {
		const target = fieldAnchor(formRef.current, field);
		if (!target) return;
		target.scrollIntoView({ behavior: "smooth", block: "center" });
		target.focus({ preventScroll: true });
		if (target instanceof HTMLButtonElement) target.click();
	}, []);

	return (
		<div className="flex flex-col gap-4">
			{showBanner && (
				<PublishedBanner
					entry={entry}
					meta={meta}
					anchored={anchored}
					onDismiss={onDismissPublished}
					onFix={goToField}
				/>
			)}
			<div className="grid items-start gap-5 xl:grid-cols-[minmax(0,1fr)_22rem]">
				<div className="flex min-w-0 flex-col gap-4">
					<div ref={formRef} className="min-w-0 scroll-mt-4">
						<PackageMetaTab
							packageId={entry.id}
							fetcher={fetcher}
							auth={auth}
						/>
					</div>
					{showPricing && (
						<PackagePricingCard
							packageId={entry.id}
							price={entry.price ?? 0}
							visibility={entry.visibility}
							fetcher={fetcher}
							auth={auth}
						/>
					)}
					{showPublicationRequest && (
						<PublicationRequestCard
							packageId={entry.id}
							fetcher={fetcher}
							auth={auth}
						/>
					)}
				</div>
				<StorePreview entry={entry} meta={meta} loading={metaLoading} />
			</div>
		</div>
	);
}

function PublishedBanner({
	entry,
	meta,
	anchored,
	onDismiss,
	onFix,
}: Readonly<{
	entry: RegistryEntry;
	meta?: PackageMeta | null;
	anchored: ReadonlySet<ListingField>;
	onDismiss: () => void;
	onFix: (field: ListingField) => void;
}>) {
	const { t } = useTranslation("common");
	const health = useMemo(() => listingHealth(entry, meta), [entry, meta]);
	const version = entry.versions[0]?.version ?? entry.manifest.version;
	const fieldLabels: Record<ListingField, string> = {
		thumbnail: t("workspaceFieldThumbnail", "a thumbnail"),
		description: t("workspaceFieldDescription", "a short description"),
		keywords: t("workspaceFieldKeywords", "keywords"),
	};
	const fixes: Record<ListingField, { label: string; icon: LucideIcon }> = {
		thumbnail: {
			label: t("workspaceAddThumbnail", "Add thumbnail"),
			icon: ImageIcon,
		},
		description: {
			label: t("workspaceAddDescription", "Add description"),
			icon: PenLine,
		},
		keywords: { label: t("workspaceAddKeywords", "Add keywords"), icon: Tag },
	};
	const nextField = health.missing.find((field) => anchored.has(field));
	const fix = nextField ? fixes[nextField] : undefined;

	return (
		<section
			aria-label={t("workspacePublishResult", "Publish result")}
			className="flex flex-wrap items-center gap-3 rounded-xl border border-primary/30 bg-primary/5 py-2.5 pr-2.5 pl-4 sm:flex-nowrap"
		>
			<span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
				{health.complete ? (
					<CheckCircle2 className="size-4" />
				) : (
					<PartyPopper className="size-4" />
				)}
			</span>
			<p className="min-w-0 flex-1 text-sm">
				<span className="font-medium">
					{health.complete
						? t(
								"workspacePublishedListingComplete",
								"Published {{version}} — listing complete.",
								{ version },
							)
						: t(
								"workspacePublishedFinishListing",
								"Published {{version}} — finish your listing.",
								{ version },
							)}
				</span>{" "}
				<span className="text-muted-foreground">
					{health.complete
						? t(
								"workspacePublishedCompleteHint",
								"The store page shows everything Explore needs.",
							)
						: t("workspacePublishedMissing", "Still missing: {{fields}}.", {
								fields: health.missing
									.map((field) => fieldLabels[field])
									.join(", "),
							})}
				</span>
			</p>
			{nextField && fix && (
				<Button
					variant="outline"
					className="order-last w-full sm:order-none sm:w-auto"
					onClick={() => onFix(nextField)}
				>
					<fix.icon className="size-4 text-muted-foreground" />
					{fix.label}
				</Button>
			)}
			<button
				type="button"
				aria-label={t("dismiss", "Dismiss")}
				onClick={onDismiss}
				className="flex size-8 shrink-0 items-center justify-center rounded-md text-muted-foreground outline-none transition-colors hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
			>
				<X className="size-4" />
			</button>
		</section>
	);
}

function PreviewLabel({ children }: Readonly<{ children: string }>) {
	return (
		<div className="font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
			{children}
		</div>
	);
}

function StorePreview({
	entry,
	meta,
	loading,
}: Readonly<{
	entry: RegistryEntry;
	meta?: PackageMeta | null;
	loading?: boolean;
}>) {
	const { t } = useTranslation("common");
	const capabilities = useMemo(
		() => readManifestAccess(entry.manifest)?.capabilityTags,
		[entry.manifest],
	);
	const summary = useMemo(
		() => storePreviewSummary(entry, meta, capabilities),
		[entry, meta, capabilities],
	);
	const category = humanizeKey(
		summary.primaryCategory?.toLowerCase() ??
			summary.secondaryCategory?.toLowerCase(),
	);
	const facts = [category, entry.manifest.license, `v${summary.latestVersion}`]
		.filter(Boolean)
		.join(" · ");
	const rated = (summary.ratingCount ?? 0) > 0;

	return (
		<aside
			aria-label={t("workspaceStorePreview", "Store preview")}
			className="xl:sticky xl:top-4"
		>
			<WorkspaceSection
				icon={Eye}
				title={t("workspaceStorePreview", "Store preview")}
				action={
					<span className="shrink-0 text-xs text-muted-foreground">
						{t("workspaceAsSeenInExplore", "as seen in Explore")}
					</span>
				}
				bodyClassName="flex flex-col gap-3"
			>
				<PreviewLabel>{t("workspacePreviewCard", "Card")}</PreviewLabel>
				{loading ? (
					<Skeleton className="aspect-[4/5] w-full rounded-xl" />
				) : (
					<PackageCard
						pkg={summary}
						href={null}
						className="hover:translate-y-0"
					/>
				)}

				<PreviewLabel>
					{t("workspacePreviewDetailHeader", "Detail header")}
				</PreviewLabel>
				<div className="rounded-lg border border-border/60 bg-background/60 p-3">
					<div className="flex items-center gap-3">
						<Avatar className="size-10 rounded-lg">
							{summary.metadata?.icon ? (
								<AvatarImage
									src={summary.metadata.icon}
									alt=""
									className="object-cover"
								/>
							) : null}
							<AvatarFallback className="rounded-lg font-mono text-xs">
								{getPackageInitials(summary.name)}
							</AvatarFallback>
						</Avatar>
						<div className="min-w-0 flex-1">
							<div className="truncate text-sm font-semibold">
								{summary.name}
							</div>
							<div className="line-clamp-2 text-xs text-muted-foreground">
								{facts}
							</div>
						</div>
						<span
							aria-hidden="true"
							className="inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md bg-primary px-2.5 text-xs font-medium text-primary-foreground"
						>
							<Download className="size-3.5" />
							{t("install", "Install")}
						</span>
					</div>
					{meta?.useCase && (
						<p className="mt-2.5 line-clamp-2 text-xs text-foreground/90">
							{meta.useCase}
						</p>
					)}
					<div className="mt-2 flex flex-wrap items-center gap-x-1.5 text-xs text-muted-foreground">
						<span>
							{t("workspaceInstallCount", {
								defaultValue_one: "{{count}} install",
								defaultValue_other: "{{count}} installs",
								count: summary.downloadCount,
							})}
						</span>
						{rated && (
							<>
								<span>·</span>
								<span className="inline-flex items-center gap-1">
									<Star className="size-3 fill-yellow-500 text-yellow-500" />
									{`${(summary.avgRating ?? 0).toFixed(1)} (${summary.ratingCount})`}
								</span>
							</>
						)}
						<span>·</span>
						<span>
							{t("workspaceNodeCount", {
								defaultValue_one: "{{count}} node",
								defaultValue_other: "{{count}} nodes",
								count: entry.nodes?.length ?? 0,
							})}
						</span>
					</div>
				</div>
				<p className="text-xs text-muted-foreground">
					{t(
						"workspacePreviewHint",
						"Saved listing fields show here right away. Manifest fields refresh with your next publish.",
					)}
				</p>
			</WorkspaceSection>
		</aside>
	);
}
