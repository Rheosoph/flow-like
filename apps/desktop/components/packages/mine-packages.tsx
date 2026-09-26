"use client";

import {
	Alert,
	AlertDescription,
	Button,
	Input,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Tooltip,
	TooltipContent,
	TooltipTrigger,
	cn,
	useSearch,
} from "@flow-like/flow-like-ui";
import { PackageCard } from "@flow-like/flow-like-ui/components/store/package-card";
import { PackagesHubLayout } from "@flow-like/flow-like-ui/components/store/packages-hub-layout";
import {
	PACKAGE_GRID_CLASS_NAME,
	PackageCardSkeleton,
} from "@flow-like/flow-like-ui/components/store/packages-store-page";
import { TEMPLATE_LANGUAGES } from "@flow-like/flow-like-ui/lib/schema/developer";
import {
	PackageStatus,
	type PackageSummary,
} from "@flow-like/flow-like-ui/lib/schema/wasm";
import { useTranslation } from "@flow-like/locales";
import {
	AlertCircle,
	Archive,
	ArrowRight,
	BookOpen,
	FolderOpen,
	Loader2,
	Plus,
	RefreshCw,
	Search,
	X,
} from "lucide-react";
import Link from "next/link";
import { type ReactNode, useMemo, useRef, useState } from "react";
import { useAuth } from "react-oidc-context";
import { DeveloperSettingsDialog } from "./developer-settings-dialog";
import { projectRoutes } from "./local-projects";
import { MINE_STATE_TONE, useMineFilterLabels } from "./mine-labels";
import {
	MINE_FILTERS,
	type MineFilter,
	type MineSort,
	matchesMineFilter,
	sortMine,
} from "./mine-model";
import { MinePackageCard } from "./mine-package-card";
import { type MineRegistryStatus, useMinePackages } from "./use-mine-packages";

const WASM_NODES_GUIDE_URL =
	"https://docs.flow-like.com/dev/wasm-nodes/overview/";
const FEATURED_TEMPLATE_COUNT = 5;
const SKELETON_KEYS = Array.from(
	{ length: 8 },
	(_, index) => `mine-skeleton-${index}`,
);

function HeaderActions({
	onAddFolder,
	addingFolder,
}: {
	onAddFolder: () => void;
	addingFolder: boolean;
}) {
	const { t } = useTranslation("common");
	return (
		<div className="flex items-center gap-2">
			<Button variant="outline" onClick={onAddFolder} disabled={addingFolder}>
				{addingFolder ? <Loader2 className="animate-spin" /> : <FolderOpen />}
				<span className="hidden sm:inline">{t("addFolder", "Add folder")}</span>
			</Button>
			<Button asChild>
				<Link href={projectRoutes.newPackage()}>
					<Plus />
					<span className="hidden sm:inline">
						{t("newPackage", "New package")}
					</span>
				</Link>
			</Button>
			<DeveloperSettingsDialog />
		</div>
	);
}

const CHIP_CLASS =
	"inline-flex h-8 shrink-0 items-center gap-1.5 rounded-lg border px-2.5 text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

function chipTone(active: boolean): string {
	return active
		? "border-border bg-muted text-foreground"
		: "border-border/60 text-muted-foreground hover:text-foreground";
}

function FilterChips({
	counts,
	filter,
	onChange,
	includeDisabled,
	onIncludeDisabledChange,
}: {
	counts: Record<MineFilter, number>;
	filter: MineFilter;
	onChange: (filter: MineFilter) => void;
	includeDisabled: boolean;
	onIncludeDisabledChange: (includeDisabled: boolean) => void;
}) {
	const { t } = useTranslation("common");
	const labels = useMineFilterLabels();
	const filters =
		includeDisabled || counts.disabled > 0
			? MINE_FILTERS
			: MINE_FILTERS.filter((key) => key !== "disabled");
	return (
		<fieldset
			aria-label={t("filterPackages", "Filter packages")}
			className="m-0 flex min-w-0 flex-[1_1_28rem] flex-wrap items-center gap-1.5 border-0 p-0"
		>
			{filters.map((key) => {
				const active = filter === key;
				const dot =
					key === "all" || key === "issues" ? null : MINE_STATE_TONE[key].dot;
				return (
					<button
						key={key}
						type="button"
						aria-pressed={active}
						onClick={() => onChange(key)}
						className={cn(CHIP_CLASS, chipTone(active))}
					>
						{dot && (
							<span
								aria-hidden="true"
								className={cn("size-1.5 rounded-full", dot)}
							/>
						)}
						{key === "issues" && (
							<AlertCircle
								aria-hidden="true"
								className="size-3.5 text-destructive"
							/>
						)}
						{labels[key]}
						<span
							className={cn(
								"font-mono text-[11px] tabular-nums",
								active ? "text-foreground" : "text-muted-foreground",
							)}
						>
							{counts[key]}
						</span>
					</button>
				);
			})}
			<button
				type="button"
				aria-pressed={includeDisabled}
				onClick={() => onIncludeDisabledChange(!includeDisabled)}
				className={cn(CHIP_CLASS, chipTone(includeDisabled), "border-dashed")}
			>
				<Archive aria-hidden="true" className="size-3.5" />
				{t("showDisabled", "Show disabled")}
			</button>
		</fieldset>
	);
}

function Toolbar({
	query,
	onQueryChange,
	sort,
	onSortChange,
	busy,
	onRefresh,
}: {
	query: string;
	onQueryChange: (query: string) => void;
	sort: MineSort;
	onSortChange: (sort: MineSort) => void;
	busy: boolean;
	onRefresh: () => void;
}) {
	const { t } = useTranslation("common");
	const searchRef = useRef<HTMLInputElement>(null);
	const searchLabel = t(
		"searchByNameIdOrFolder",
		"Search by name, id or folder…",
	);
	return (
		<div className="flex flex-col gap-3 sm:flex-row sm:items-center">
			<div className="relative min-w-0 flex-1">
				<Search
					aria-hidden="true"
					className="pointer-events-none absolute left-4 top-1/2 h-5 w-5 -translate-y-1/2 text-muted-foreground"
				/>
				<Input
					ref={searchRef}
					type="search"
					aria-label={searchLabel}
					aria-controls="mine-package-results"
					placeholder={searchLabel}
					value={query}
					onChange={(event) => onQueryChange(event.target.value)}
					className="h-12 rounded-xl border-border/60 bg-muted/30 pr-12 pl-12 text-sm shadow-none transition-colors focus-visible:bg-background [&::-webkit-search-cancel-button]:appearance-none"
				/>
				{query && (
					<button
						type="button"
						aria-label={t("clearSearch", "Clear search")}
						onClick={() => {
							onQueryChange("");
							searchRef.current?.focus();
						}}
						className="absolute right-1 top-1/2 flex h-10 w-10 -translate-y-1/2 items-center justify-center rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
					>
						<X aria-hidden="true" className="h-4 w-4" />
					</button>
				)}
			</div>
			<div className="flex shrink-0 items-center gap-2">
				<Select
					value={sort}
					onValueChange={(value) => onSortChange(value as MineSort)}
				>
					<SelectTrigger
						aria-label={t("sortPackages", "Sort packages")}
						className="min-h-12 w-44 gap-2 rounded-xl border-border/60 bg-background text-sm"
					>
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="attention">
							{t("needsAttention", "Needs attention")}
						</SelectItem>
						<SelectItem value="name">{t("sortByName", "Name")}</SelectItem>
					</SelectContent>
				</Select>
				<Tooltip>
					<TooltipTrigger asChild>
						<Button
							variant="ghost"
							size="icon"
							className="size-12 rounded-xl text-muted-foreground"
							aria-label={t("refresh", "Refresh")}
							disabled={busy}
							onClick={onRefresh}
						>
							<RefreshCw className={cn(busy && "animate-spin")} />
						</Button>
					</TooltipTrigger>
					<TooltipContent>{t("refresh", "Refresh")}</TooltipContent>
				</Tooltip>
			</div>
		</div>
	);
}

function RegistryHint({
	status,
	onRetry,
	retrying,
}: {
	status: MineRegistryStatus;
	onRetry: () => void;
	retrying: boolean;
}) {
	const { t } = useTranslation("common");
	const auth = useAuth();
	if (status === "signed-out")
		return (
			<span className="flex items-center gap-1.5">
				{t("signInToSeeRegistryStatus", "Sign in to see registry status.")}
				<Button
					variant="link"
					size="sm"
					className="h-auto p-0"
					onClick={() => auth.signinRedirect()}
				>
					{t("signIn", "Sign in")}
				</Button>
			</span>
		);
	if (status === "unavailable")
		return (
			<span className="flex items-center gap-1.5">
				<AlertCircle aria-hidden="true" className="h-3.5 w-3.5 text-tertiary" />
				{t(
					"registryUnreachableShowingLocalPackagesOnly",
					"Registry unreachable — showing local packages only.",
				)}
				<Button
					variant="link"
					size="sm"
					className="h-auto p-0"
					disabled={retrying}
					onClick={onRetry}
				>
					{retrying ? t("retrying", "Retrying…") : t("retry", "Retry")}
				</Button>
			</span>
		);
	if (status === "loading")
		return (
			<span className="flex items-center gap-1.5">
				<Loader2 aria-hidden="true" className="h-3.5 w-3.5 animate-spin" />
				{t("checkingTheRegistry", "Checking the registry…")}
			</span>
		);
	return null;
}

function TemplateCard({
	language,
}: {
	language: (typeof TEMPLATE_LANGUAGES)[number];
}) {
	const { t } = useTranslation("common");
	const pkg = useMemo<PackageSummary>(
		() => ({
			id: `template-${language.value}`,
			name: t("languageNodePackage", "{{language}} node package", {
				language: language.label,
			}),
			description: language.description,
			latestVersion: "0.1.0",
			downloadCount: 0,
			status: PackageStatus.Active,
			keywords: [],
			verified: false,
			price: 0,
			visibility: "public",
		}),
		[language, t],
	);
	return (
		<PackageCard
			pkg={pkg}
			href={projectRoutes.newPackage(language.value)}
			versionLabel={t("template", "Template")}
			iconSrc={language.img}
			footer={
				<span className="relative mt-auto flex items-center justify-between border-t border-border/60 pt-2.5 text-xs font-semibold text-primary">
					{t("startWithLanguage", "Start with {{language}}", {
						language: language.label,
					})}
					<ArrowRight className="size-4 transition-transform group-hover:translate-x-0.5" />
				</span>
			}
		/>
	);
}

function MoreLanguagesCard({
	languages,
}: {
	languages: readonly (typeof TEMPLATE_LANGUAGES)[number][];
}) {
	const { t } = useTranslation("common");
	return (
		<Link
			href={projectRoutes.newPackage()}
			className="group flex min-h-72 flex-col items-center justify-center gap-4 rounded-xl border border-dashed border-border/60 bg-card/40 p-6 text-center transition-colors hover:border-primary/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
		>
			<div className="flex -space-x-2">
				{languages.slice(0, 5).map((language) => (
					<img
						key={language.value}
						src={language.img}
						alt=""
						className="size-9 rounded-lg border-2 border-background object-cover"
					/>
				))}
			</div>
			<div className="space-y-1">
				<p className="text-sm font-semibold">
					{t("countMoreLanguages", {
						defaultValue_one: "{{count}} more language",
						defaultValue_other: "{{count}} more languages",
						count: languages.length,
					})}
				</p>
				<p className="line-clamp-2 text-xs text-muted-foreground">
					{languages.map((language) => language.label).join(" · ")}
				</p>
			</div>
			<span className="flex items-center gap-1 text-xs font-semibold text-primary">
				{t("seeAllTemplates", "See all templates")}
				<ArrowRight className="size-4 transition-transform group-hover:translate-x-0.5" />
			</span>
		</Link>
	);
}

function EmptyOnboarding({
	onAddFolder,
	addingFolder,
}: {
	onAddFolder: () => void;
	addingFolder: boolean;
}) {
	const { t } = useTranslation("common");
	const featured = TEMPLATE_LANGUAGES.slice(0, FEATURED_TEMPLATE_COUNT);
	const more = TEMPLATE_LANGUAGES.slice(FEATURED_TEMPLATE_COUNT);
	return (
		<section className="space-y-6" aria-labelledby="mine-onboarding-title">
			<div className="flex flex-col gap-4 sm:flex-row sm:items-end sm:justify-between">
				<div className="space-y-1">
					<h2
						id="mine-onboarding-title"
						className="text-xl font-semibold tracking-tight"
					>
						{t("createYourFirstNodePackage", "Create your first node package")}
					</h2>
					<p className="max-w-2xl text-sm text-muted-foreground">
						{t(
							"pickALanguageToStartFromATemplatePackagesCompileToWebassembly",
							"Pick a language to start from a template. Packages compile to WebAssembly, run sandboxed in every flow and can be published to the registry.",
						)}
					</p>
				</div>
				<div className="flex shrink-0 flex-wrap gap-2">
					<Button
						variant="outline"
						onClick={onAddFolder}
						disabled={addingFolder}
					>
						{addingFolder ? (
							<Loader2 className="animate-spin" />
						) : (
							<FolderOpen />
						)}
						{t("addExistingFolder", "Add existing folder")}
					</Button>
					<Button variant="ghost" asChild>
						<a href={WASM_NODES_GUIDE_URL} target="_blank" rel="noreferrer">
							<BookOpen />
							{t("readTheGuide", "Read the guide")}
						</a>
					</Button>
				</div>
			</div>
			<div className={PACKAGE_GRID_CLASS_NAME}>
				{featured.map((language) => (
					<TemplateCard key={language.value} language={language} />
				))}
				{more.length > 0 && <MoreLanguagesCard languages={more} />}
			</div>
		</section>
	);
}

function NoMatches({ onShowAll }: { onShowAll: () => void }) {
	const { t } = useTranslation("common");
	return (
		<div className="flex items-center gap-3 rounded-xl border border-dashed border-border/60 p-4 text-sm text-muted-foreground">
			<Search aria-hidden="true" className="h-4 w-4 shrink-0" />
			<span className="flex-1">
				{t(
					"noPackagesMatchThisFilterAndSearch",
					"No packages match this filter and search.",
				)}
			</span>
			<Button variant="secondary" size="sm" onClick={onShowAll}>
				{t("showAll", "Show all")}
			</Button>
		</div>
	);
}

export function MinePackages({ navigation }: { navigation: ReactNode }) {
	const { t } = useTranslation("common");
	const [includeDisabled, setIncludeDisabled] = useState(false);
	const mine = useMinePackages({ includeDisabled });
	const [query, setQuery] = useState("");
	const [filter, setFilter] = useState<MineFilter>("all");
	const [sort, setSort] = useState<MineSort>("attention");

	const changeIncludeDisabled = (next: boolean) => {
		setIncludeDisabled(next);
		if (!next && filter === "disabled") setFilter("all");
	};

	const { entries, counts } = mine.model;
	const visible = useMemo(
		() =>
			sortMine(entries, sort).filter((entry) =>
				matchesMineFilter(entry, filter),
			),
		[entries, sort, filter],
	);
	const results = useSearch(visible, query, {
		fields: ["name", "packageId", "searchText"],
		boost: { name: 3, packageId: 2 },
	});

	const addFolder = () => mine.addFolder(undefined);
	const registrySettled = mine.registryStatus !== "loading";
	const isLoading =
		mine.isLoading || (entries.length === 0 && !registrySettled);
	const isEmpty = !isLoading && !mine.error && entries.length === 0;

	return (
		<PackagesHubLayout
			subtitle={t(
				"yourNodePackagesOnThisMachineAndInTheRegistry",
				"Your node packages — on this machine and in the registry.",
			)}
			actions={
				<HeaderActions
					onAddFolder={addFolder}
					addingFolder={mine.isAddingFolder}
				/>
			}
			navigation={navigation}
			toolbar={
				isEmpty ? null : (
					<Toolbar
						query={query}
						onQueryChange={setQuery}
						sort={sort}
						onSortChange={setSort}
						busy={mine.isRefreshing}
						onRefresh={mine.refresh}
					/>
				)
			}
			filters={
				(entries.length > 0 || includeDisabled) && (
					<div className="flex min-h-10 flex-wrap items-center gap-3">
						<FilterChips
							counts={counts}
							filter={filter}
							onChange={setFilter}
							includeDisabled={includeDisabled}
							onIncludeDisabledChange={changeIncludeDisabled}
						/>
					</div>
				)
			}
		>
			<section
				id="mine-package-results"
				className="space-y-5"
				aria-busy={isLoading}
			>
				<output
					className="flex min-h-5 flex-wrap gap-4 text-sm text-muted-foreground"
					aria-live="polite"
				>
					{!isLoading && entries.length > 0 && (
						<span>
							{t("countPackages", {
								defaultValue_one: "{{count}} package",
								defaultValue_other: "{{count}} packages",
								count: results.length,
							})}
						</span>
					)}
					<RegistryHint
						status={mine.registryStatus}
						onRetry={mine.retryRegistry}
						retrying={mine.isRetryingRegistry}
					/>
				</output>
				{Boolean(mine.error) && (
					<Alert variant="destructive" className="rounded-xl">
						<AlertCircle className="h-4 w-4" />
						<AlertDescription className="flex flex-wrap items-center justify-between gap-3">
							{t(
								"localPackagesCouldNotBeLoaded",
								"Local packages could not be loaded.",
							)}
							<Button variant="outline" size="sm" onClick={mine.refresh}>
								{t("retry", "Retry")}
							</Button>
						</AlertDescription>
					</Alert>
				)}
				{isLoading ? (
					<div className={PACKAGE_GRID_CLASS_NAME}>
						{SKELETON_KEYS.map((key) => (
							<PackageCardSkeleton key={key} />
						))}
					</div>
				) : isEmpty ? (
					<EmptyOnboarding
						onAddFolder={addFolder}
						addingFolder={mine.isAddingFolder}
					/>
				) : results.length === 0 ? (
					<NoMatches
						onShowAll={() => {
							setFilter("all");
							setQuery("");
						}}
					/>
				) : (
					<div className={PACKAGE_GRID_CLASS_NAME}>
						{results.map((entry) => (
							<MinePackageCard
								key={entry.key}
								entry={entry}
								onInspect={mine.requestInspection}
								onRemove={mine.removeProject}
								onLinkFolder={addFolder}
								onProjectChanged={mine.invalidateProject}
								linkingFolder={mine.isAddingFolder}
							/>
						))}
					</div>
				)}
			</section>
		</PackagesHubLayout>
	);
}
