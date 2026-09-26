"use client";

import { useTranslation } from "@flow-like/locales";
import { Compass } from "lucide-react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import {
	type ReactNode,
	Suspense,
	useCallback,
	useEffect,
	useRef,
	useState,
} from "react";
import { useAuth } from "react-oidc-context";
import { useAssetImage } from "../../../hooks/use-asset-image";
import { useDeveloperMode } from "../../../hooks/use-developer-mode";
import { useInvoke } from "../../../hooks/use-invoke";
import { GlobalPermission } from "../../../lib/permission/global-permission";
import { cn } from "../../../lib/utils";
import type { IEventMapping } from "../../interfaces/interfaces";
import { EmptyState } from "../../ui/empty-state";
import { BentoGrid, presentSlots } from "./bento-grid";
import { ExploreHeader } from "./explore-header";
import { exploreSearchHref, exploreTypeFromParam } from "./explore-href";
import { ExploreLinksProvider } from "./explore-item-card";
import {
	dismissStorage,
	isExploreUnsupportedError,
	pickExploreView,
} from "./explore-model";
import { ExploreQueryError, ExploreSkeleton } from "./explore-states";
import { ExploreTypeTabs } from "./explore-type-filter";
import type { ExploreAnnouncement, ExploreTypeFilter } from "./explore-types";
import { LegacyExploreFallback } from "./legacy-explore";
import { ExploreRowView, exploreRowHasContent } from "./rows/explore-row";
import { useExplore, useExploreViewer } from "./use-explore";

export interface ExplorePageProps {
	eventConfig?: IEventMapping;
}

/** Scroll region and `@container/explore`, shared by the landing and Browse. A `div`: the app shell owns `<main>`. */
export function ExploreShell({
	children,
	ambient,
}: Readonly<{ children: ReactNode; ambient?: ReactNode }>) {
	return (
		<div className="flex min-h-0 w-full min-w-0 flex-1 flex-col overflow-hidden">
			<div
				data-explore-scroll
				className="relative min-h-0 flex-1 overflow-y-auto overflow-x-hidden [scrollbar-gutter:stable]"
			>
				{ambient}
				<div className="@container/explore relative mx-auto w-full max-w-[1600px] px-4 pb-12 pt-5 sm:px-8 sm:pt-6">
					{children}
				</div>
			</div>
		</div>
	);
}

/** The active spotlight cover, heavily blurred behind the top of the page. */
function AmbientGlow({ cover }: Readonly<{ cover?: string }>) {
	const image = useAssetImage(cover);
	if (!image.canRender) return null;
	return (
		<div
			aria-hidden="true"
			className="pointer-events-none absolute inset-x-0 top-0 h-210 overflow-hidden [mask-image:linear-gradient(to_bottom,black,rgb(0_0_0/0.5)_48%,transparent)]"
		>
			<img
				ref={image.imgRef}
				src={image.src}
				onLoad={image.onLoad}
				onError={image.onError}
				alt=""
				className={cn(
					"absolute -left-[10%] -top-[30%] h-[125%] w-[120%] max-w-none object-cover blur-[96px] saturate-150 transition-opacity duration-700",
					image.loaded ? "opacity-20" : "opacity-0",
				)}
			/>
		</div>
	);
}

/** `?type=` preselects the filter and follows it; the landing ignores it unless package content is on. */
function useTypeParam(): [
	ExploreTypeFilter,
	(type: ExploreTypeFilter) => void,
] {
	const searchParams = useSearchParams();
	const router = useRouter();
	const pathname = usePathname();
	const fromUrl = exploreTypeFromParam(searchParams.get("type"));
	const [type, setTypeState] = useState(fromUrl);

	useEffect(() => {
		setTypeState(fromUrl);
	}, [fromUrl]);

	const setType = useCallback(
		(next: ExploreTypeFilter) => {
			setTypeState(next);
			const params = new URLSearchParams(searchParams.toString());
			if (next === "all") params.delete("type");
			else params.set("type", next);
			const query = params.toString();
			router.replace(query ? `${pathname}?${query}` : pathname, {
				scroll: false,
			});
		},
		[pathname, router, searchParams],
	);
	return [type, setType];
}

/**
 * Read from storage while rendering, so a dismissed announcement is never painted for a frame before the grid
 * re-flows. Keyboard focus follows the control that replaces the pressed one: Restore after a dismiss, the
 * tile's dismiss button after a restore.
 */
function useAnnouncementDismissal(notice?: ExploreAnnouncement | null) {
	const key = notice?.dismissible ? notice.dismissKey : undefined;
	const [changed, setChanged] = useState<Readonly<Record<string, boolean>>>({});
	const [focus, setFocus] = useState<"restore" | "notice">();
	const restoreRef = useRef<HTMLButtonElement>(null);
	const dismissRef = useRef<HTMLButtonElement>(null);
	const dismissed = key
		? (changed[key] ?? dismissStorage.isDismissed(key))
		: false;

	useEffect(() => {
		if (!focus) return;
		(focus === "restore" ? restoreRef : dismissRef).current?.focus();
		setFocus(undefined);
	}, [focus]);

	const setDismissed = useCallback(
		(value: boolean) => {
			if (!key) return;
			if (value) dismissStorage.dismiss(key);
			else dismissStorage.restore(key);
			setChanged((current) => ({ ...current, [key]: value }));
			setFocus(value ? "restore" : "notice");
		},
		[key],
	);
	const dismiss = useCallback(() => setDismissed(true), [setDismissed]);
	const restore = useCallback(() => setDismissed(false), [setDismissed]);

	return { dismissed, dismiss, restore, restoreRef, dismissRef };
}

/** Keyed by hub, profile and account, so a cached permission never carries over to another identity. */
function useCanEditLayout(): boolean {
	const { backend, apiOrigin, profileId, signedIn } = useExploreViewer();
	const sub = useAuth()?.user?.profile?.sub;
	const info = useInvoke(
		backend.userState.getInfo,
		backend.userState,
		[],
		signedIn === true,
		[apiOrigin, profileId, sub],
	);
	return (
		signedIn === true &&
		!info.isError &&
		new GlobalPermission(info.data?.permission ?? 0).hasPermission(
			GlobalPermission.WriteLandingPage,
		)
	);
}

export function ExplorePage({ eventConfig }: Readonly<ExplorePageProps>) {
	return (
		<Suspense
			fallback={
				<ExploreShell>
					<ExploreSkeleton />
				</ExploreShell>
			}
		>
			<ExploreLinksProvider eventConfig={eventConfig}>
				<ExploreLanding eventConfig={eventConfig} />
			</ExploreLinksProvider>
		</Suspense>
	);
}

function ExploreLanding({ eventConfig }: Readonly<ExplorePageProps>) {
	const [cover, setCover] = useState<string>();
	const explore = useExplore();
	if (!explore.data && isExploreUnsupportedError(explore.error)) {
		return <LegacyExploreFallback eventConfig={eventConfig} />;
	}
	return (
		<ExploreShell ambient={<AmbientGlow cover={cover} />}>
			<ExploreLandingContent explore={explore} onCoverChange={setCover} />
		</ExploreShell>
	);
}

function ExploreLandingContent({
	explore,
	onCoverChange,
}: Readonly<{
	explore: ReturnType<typeof useExplore>;
	onCoverChange: (cover: string | undefined) => void;
}>) {
	const { t } = useTranslation("store");
	const router = useRouter();
	const { developerMode } = useDeveloperMode();
	const [type, setType] = useTypeParam();
	const canEditLayout = useCanEditLayout();
	const page = explore.data;
	const dev = !!page?.viewer.dev && developerMode;
	const activeType: ExploreTypeFilter = dev ? type : "all";
	const view = page ? pickExploreView(page, activeType) : undefined;
	const notice = useAnnouncementDismissal(view?.grid.notice);
	const hasHero = (view?.grid.hero?.slides.length ?? 0) > 0;

	useEffect(() => {
		if (!hasHero) onCoverChange(undefined);
	}, [hasHero, onCoverChange]);

	if (!page || !view) {
		if (explore.isError) {
			return (
				<div className="flex flex-col gap-5">
					<h1 className="text-3xl font-semibold tracking-tight">
						{t("explore", "Explore")}
					</h1>
					<ExploreQueryError
						error={explore.error}
						onRetry={() => void explore.refetch()}
						retrying={explore.isFetching}
						fallbackLink={{
							label: t("exploreOpenBrowse", "Browse everything"),
							href: exploreSearchHref(),
						}}
					/>
				</div>
			);
		}
		return <ExploreSkeleton />;
	}

	const context = { mix: dev ? activeType : ("apps" as const), dev };
	const empty =
		presentSlots(view.grid, notice.dismissed).length === 0 &&
		!view.rows.some((row) => exploreRowHasContent(row, context));

	return (
		<div className="flex flex-col gap-10" data-explore-view={activeType}>
			<div className="flex flex-col gap-5">
				<ExploreHeader
					dev={dev}
					type={activeType}
					canEditLayout={canEditLayout}
					onRestoreAnnouncement={notice.dismissed ? notice.restore : undefined}
					restoreRef={notice.restoreRef}
					typeFilter={
						dev ? (
							<ExploreTypeTabs
								value={activeType}
								counts={page.typeCounts}
								onChange={setType}
							/>
						) : undefined
					}
				/>
				<BentoGrid
					grid={view.grid}
					type={activeType}
					noticeDismissed={notice.dismissed}
					onDismissNotice={notice.dismiss}
					dismissNoticeRef={notice.dismissRef}
					onCoverChange={onCoverChange}
				/>
			</div>
			{view.rows.map((row) => (
				<ExploreRowView
					key={
						row.kind === "rail"
							? row.rail.placementId || row.rail.rail
							: row.collection.placementId
					}
					row={row}
					context={context}
				/>
			))}
			{empty && (
				<div className="flex justify-center py-10">
					<EmptyState
						icons={[Compass]}
						title={t("exploreEmptyTitle", "Nothing to explore yet")}
						description={t(
							"exploreEmptyBody",
							"This hub has not published anything to Explore. Search everything that is public instead.",
						)}
						action={{
							label: t("exploreOpenBrowse", "Browse everything"),
							onClick: () => router.push(exploreSearchHref()),
						}}
					/>
				</div>
			)}
		</div>
	);
}
