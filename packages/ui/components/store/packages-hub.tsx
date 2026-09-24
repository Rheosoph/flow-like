"use client";

import { useTranslation } from "@flow-like/locales";
import { Code2, Compass, Library } from "lucide-react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import {
	type ComponentProps,
	type ReactNode,
	type RefObject,
	Suspense,
	useCallback,
	useEffect,
	useLayoutEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { cn } from "../../lib/utils";
import type { GenericFetcher } from "../pages/store/store-package-detail";
import { Button } from "../ui/button";
import type { CompileStatus } from "../ui/package-status-badge";
import { Skeleton } from "../ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../ui/tabs";
import { exploreSearchHref } from "./explore/explore-href";
import { useExploreSupported } from "./explore/use-explore";
import {
	type HubExploreAction,
	type PackagesHubTab,
	hubExploreAction,
	hubHref,
	hubTabFromParam,
	isLegacyHubTab,
} from "./packages-hub-model";
import {
	PACKAGE_GRID_CLASS_NAME,
	PackageCardSkeleton,
	PackageDetailWrapper,
	PackageListContent,
} from "./packages-store-page";

type PackagesHubAuth = ComponentProps<typeof PackageDetailWrapper>["auth"];
type HubSection = (navigation: ReactNode) => ReactNode;
interface HubNavigator {
	navigate: (value: string) => void;
	refocus: RefObject<boolean>;
}

export interface PackagesHubPageProps {
	fetcher: GenericFetcher;
	auth: PackagesHubAuth;
	getPackageStatus?: (packageId: string) => CompileStatus | undefined;
	mine: HubSection;
	library: HubSection;
}

const SKELETON_KEYS = Array.from(
	{ length: 8 },
	(_, index) => `packages-hub-skeleton-${index}`,
);
const HUB_TABS_CLASS_NAME = "min-h-0 w-full flex-1 gap-0";
const TAB_CONTENT_CLASS_NAME =
	"m-0 flex min-h-0 flex-1 flex-col data-[state=inactive]:hidden";
const HUB_NAV_SELECTOR = "[data-packages-hub-nav]";
const ACTIVE_TAB_SELECTOR = '[role="tab"][aria-selected="true"]';

/** A plain `div` like `PackagesHubLayout`: the app shell already renders `<main>`. */
function PackagesHubSkeleton() {
	const { t } = useTranslation();
	return (
		<div
			aria-busy="true"
			data-packages-hub-skeleton
			className="flex min-h-0 min-w-0 w-full flex-1 flex-col overflow-hidden"
		>
			<div className="mx-auto w-full max-w-400 px-4 pt-5 pb-12 sm:px-8 sm:pt-6">
				<output className="sr-only">
					{t("store:loadingPackages", "Loading packages…")}
				</output>
				<div className="flex min-h-11 items-center justify-between gap-3">
					<Skeleton className="h-8 w-36 rounded-lg" />
					<Skeleton className="h-10 w-48 rounded-xl" />
				</div>
				<Skeleton className="mt-2 h-4 w-72 max-w-full rounded" />
				<Skeleton className="mt-4 h-12 w-full rounded-xl" />
				<div
					className={cn(
						PACKAGE_GRID_CLASS_NAME,
						"mt-6 border-t border-border/50 pt-6 sm:mt-7 sm:pt-7",
					)}
				>
					{SKELETON_KEYS.map((key) => (
						<PackageCardSkeleton key={key} />
					))}
				</div>
			</div>
		</div>
	);
}

/** `refocus` is set when the switch starts from the tab list, which the switch remounts with the panel. */
function useHubNavigate(): HubNavigator {
	const router = useRouter();
	const searchParams = useSearchParams();
	const refocus = useRef(false);
	const navigate = useCallback(
		(value: string) => {
			refocus.current = Boolean(
				document.activeElement?.closest(HUB_NAV_SELECTOR),
			);
			router.push(hubHref(searchParams, hubTabFromParam(value)));
		},
		[router, searchParams],
	);
	return useMemo(() => ({ navigate, refocus }), [navigate]);
}

/** The incoming panel's tab list takes over focus when it mounts; the outgoing one is still mounted then. */
function useTakeOverFocus(refocus: RefObject<boolean>) {
	const ref = useRef<HTMLDivElement>(null);
	useLayoutEffect(() => {
		if (!refocus.current) return;
		const active = ref.current?.querySelector<HTMLElement>(ACTIVE_TAB_SELECTOR);
		if (!active) return;
		refocus.current = false;
		active.focus();
	}, [refocus]);
	return ref;
}

function HubNavigation({
	exploreHref,
	exploreActive = false,
	refocus,
}: {
	exploreHref: string;
	exploreActive?: boolean;
	refocus: RefObject<boolean>;
}) {
	const { t } = useTranslation();
	const ref = useTakeOverFocus(refocus);
	const exploreLabel = t("explorePackages", "Explore packages");
	return (
		<div
			ref={ref}
			className="flex min-w-0 items-center gap-2"
			data-packages-hub-nav
		>
			<TabsList
				aria-label={t("packageViews", "Package views")}
				className="h-10 rounded-xl border border-border/60 bg-muted/30 p-1"
			>
				<TabsTrigger value="mine" className="gap-1 rounded-lg px-2 sm:px-3">
					<Code2 aria-hidden="true" className="hidden h-3.5 w-3.5 sm:block" />
					{t("mine", "Mine")}
				</TabsTrigger>
				<TabsTrigger value="library" className="gap-1 rounded-lg px-2 sm:px-3">
					<Library aria-hidden="true" className="hidden h-3.5 w-3.5 sm:block" />
					{t("library", "Library")}
				</TabsTrigger>
			</TabsList>
			<Button
				asChild
				variant={exploreActive ? "secondary" : "outline"}
				className="h-10 rounded-xl px-3"
			>
				<Link
					href={exploreHref}
					aria-current={exploreActive ? "page" : undefined}
					title={exploreLabel}
				>
					<Compass aria-hidden="true" />
					<span className="sr-only sm:not-sr-only">{exploreLabel}</span>
				</Link>
			</Button>
		</div>
	);
}

/**
 * Signing in or switching language re-keys the support query, which goes pending again. The settled answer stands
 * meanwhile, so the legacy list keeps its search, filters and page instead of dropping back to the skeleton.
 */
function useHubExploreAction(): HubExploreAction {
	const live = hubExploreAction(useExploreSupported());
	const [settled, setSettled] = useState<HubExploreAction>("wait");
	if (live !== "wait" && live !== settled) setSettled(live);
	return live === "wait" ? settled : live;
}

/** Explore lives in Bento Browse; a hub without `/store/explore` keeps the old package list here. */
function HubExplore({
	fetcher,
	auth,
	hub,
}: {
	fetcher: GenericFetcher;
	auth: PackagesHubAuth;
	hub: HubNavigator;
}) {
	const router = useRouter();
	const searchParams = useSearchParams();
	const action = useHubExploreAction();

	useEffect(() => {
		if (action === "redirect") {
			router.replace(exploreSearchHref({ type: "packages" }));
		}
	}, [action, router]);

	if (action !== "legacy") return <PackagesHubSkeleton />;

	return (
		<Tabs
			value="explore"
			onValueChange={hub.navigate}
			activationMode="manual"
			className={HUB_TABS_CLASS_NAME}
		>
			<PackageListContent
				fetcher={fetcher}
				auth={auth}
				navigation={
					<HubNavigation
						exploreHref={hubHref(searchParams, "explore")}
						exploreActive
						refocus={hub.refocus}
					/>
				}
			/>
		</Tabs>
	);
}

function HubTabs({
	tab,
	mine,
	library,
	hub,
}: {
	tab: Exclude<PackagesHubTab, "explore">;
	mine: HubSection;
	library: HubSection;
	hub: HubNavigator;
}) {
	const router = useRouter();
	const searchParams = useSearchParams();
	const tabParam = searchParams.get("tab");

	useEffect(() => {
		if (isLegacyHubTab(tabParam)) {
			router.replace(hubHref(searchParams, tab));
		}
	}, [router, searchParams, tab, tabParam]);

	const navigation = (
		<HubNavigation
			exploreHref={exploreSearchHref({ type: "packages" })}
			refocus={hub.refocus}
		/>
	);

	return (
		<Tabs
			value={tab}
			onValueChange={hub.navigate}
			activationMode="manual"
			className={HUB_TABS_CLASS_NAME}
		>
			<TabsContent value="mine" className={TAB_CONTENT_CLASS_NAME}>
				{mine(navigation)}
			</TabsContent>
			<TabsContent value="library" className={TAB_CONTENT_CLASS_NAME}>
				{library(navigation)}
			</TabsContent>
		</Tabs>
	);
}

function PackagesHubContent({
	fetcher,
	auth,
	getPackageStatus,
	mine,
	library,
}: PackagesHubPageProps) {
	const searchParams = useSearchParams();
	const hub = useHubNavigate();

	if (searchParams.get("id")) {
		return (
			<PackageDetailWrapper
				fetcher={fetcher}
				auth={auth}
				getPackageStatus={getPackageStatus}
			/>
		);
	}

	const tab = hubTabFromParam(searchParams.get("tab"));
	if (tab === "explore") {
		return <HubExplore fetcher={fetcher} auth={auth} hub={hub} />;
	}
	return <HubTabs tab={tab} mine={mine} library={library} hub={hub} />;
}

/** `/store/packages`: `?id=` is the store detail, `?tab=mine|library` the hub, anything else Explore. */
export function PackagesHubPage(props: PackagesHubPageProps) {
	return (
		<Suspense fallback={<PackagesHubSkeleton />}>
			<PackagesHubContent {...props} />
		</Suspense>
	);
}
