"use client";

import {
	AboutSection,
	AppReviewsSection,
	HeroSkeleton,
	StoreEmptyState,
	StoreForkButton,
	StoreHero,
	StoreRecommendations,
	TextEditor,
	exploreHref,
	legacyAppsExploreTarget,
	useStoreData,
} from "@flow-like/flow-like-ui";
import { MarketplaceCheckoutDialog } from "@flow-like/flow-like-ui/components/payments/checkout-dialog";
import { EVENT_CONFIG } from "@flow-like/flow-like-ui/lib/event-config";
import { isRecord } from "@flow-like/flow-like-ui/lib/response-shape";
import { useTranslation } from "@flow-like/locales";
import { useRouter, useSearchParams } from "next/navigation";
import { useEffect, useRef } from "react";
import { toast } from "sonner";
import { useApplyForkBundle } from "../../lib/use-apply-fork-bundle";

export default function Page() {
	const { t } = useTranslation("common");
	const searchParams = useSearchParams();
	const router = useRouter();
	const id = searchParams.get("id") ?? undefined;
	const purchaseStatus =
		searchParams.get("checkout") ?? searchParams.get("purchase");
	const applyForkBundle = useApplyForkBundle();
	const {
		appData,
		metaData,
		isMember,
		isPurchasing,
		checkoutOpen,
		setCheckoutOpen,
		purchasingAllowed,
		isLoading,
		isError,
		notFound,
		hasThumbnail,
		coverUrl,
		iconUrl,
		appName,
		priceLabel,
		canUseApp,
		onUse,
		onSettings,
		onBuy,
		onJoinOrRequest,
		refetchAppData,
	} = useStoreData(id, router, EVENT_CONFIG);
	const handledPurchaseRef = useRef<string | null>(null);

	useEffect(() => {
		if (id) return;
		router.replace(
			searchParams.get("sort")
				? legacyAppsExploreTarget(searchParams)
				: exploreHref(),
		);
	}, [id, searchParams, router]);

	useEffect(() => {
		if (!purchaseStatus) {
			handledPurchaseRef.current = null;
			return;
		}
		if (handledPurchaseRef.current === purchaseStatus) return;
		handledPurchaseRef.current = purchaseStatus;

		if (purchaseStatus === "success" || purchaseStatus === "submitted") {
			toast.info(
				"Checkout returned. Payment is confirmed by the server; access may still be processing.",
			);
			void refetchAppData();
		} else if (purchaseStatus === "canceled") {
			toast.info(
				"Checkout was closed. Check Purchases for its current status.",
			);
		}

		const url = new URL(window.location.href);
		url.searchParams.delete("purchase");
		url.searchParams.delete("checkout");
		router.replace(url.pathname + url.search, { scroll: false });
	}, [purchaseStatus, refetchAppData, router]);

	if (!id) return null;

	if (isLoading) {
		return (
			<main className="flex-col flex grow max-h-full overflow-auto min-h-0 w-full">
				<HeroSkeleton />
				<div className="max-w-5xl mx-auto px-6 md:px-10 pt-8 space-y-4 w-full">
					<div className="h-4 w-3/4 rounded-full bg-muted/20" />
					<div className="h-4 w-1/2 rounded-full bg-muted/20" />
				</div>
			</main>
		);
	}

	if (isError || notFound || !isRecord(appData) || !isRecord(metaData)) {
		return (
			<div className="flex-1 flex items-center justify-center p-6">
				<StoreEmptyState
					title={
						isError
							? t("failedToLoadApp", "Failed to load app")
							: t("appNotFound", "App not found")
					}
					description={
						isError
							? t(
									"somethingWentWrongPleaseTryAgainLater",
									"Something went wrong. Please try again later.",
								)
							: t(
									"thisAppMayBePrivateOrNoLongerAvailable",
									"This app may be private or no longer available.",
								)
					}
				/>
			</div>
		);
	}

	return (
		<main
			key={id}
			className="flex-col flex grow max-h-full overflow-auto min-h-0 w-full"
		>
			<MarketplaceCheckoutDialog
				key={`checkout:${id}`}
				appId={id}
				appName={appName}
				amount={appData.price ?? 0}
				open={checkoutOpen}
				onOpenChange={setCheckoutOpen}
				onPurchased={refetchAppData}
			/>
			<StoreHero
				appId={id}
				hasThumbnail={hasThumbnail}
				coverUrl={coverUrl}
				iconUrl={iconUrl}
				appName={appName}
				priceLabel={priceLabel}
				category={appData.primary_category ?? "Other"}
				isMember={isMember}
				ratingCount={appData.rating_count}
				avgRating={appData.avg_rating ?? 0}
				visibility={appData.visibility}
				authors={appData.authors}
				canUseApp={canUseApp}
				price={appData.price ?? 0}
				purchasingAllowed={purchasingAllowed}
				isPurchasing={isPurchasing}
				onUse={onUse}
				onSettings={onSettings}
				onBuy={onBuy}
				onJoinOrRequest={onJoinOrRequest}
				actionsExtra={
					appData.allow_forking ? (
						<StoreForkButton
							appId={id}
							appName={appName}
							target="offline"
							targets={["offline", "online"]}
							onForkStarted={applyForkBundle}
							hideUnlessAvailable
						/>
					) : null
				}
			/>

			<div className="max-w-5xl mx-auto w-full px-6 md:px-10 pt-8 pb-12 space-y-10">
				<AboutSection app={appData} meta={metaData} />

				{metaData?.long_description && (
					<div className="leading-relaxed">
						<TextEditor initialContent={metaData.long_description} isMarkdown />
					</div>
				)}

				{typeof id === "string" && id.trim().length > 0 && (
					<AppReviewsSection appId={id} onReviewChanged={refetchAppData} />
				)}

				<StoreRecommendations excludeAppId={id} />
			</div>
		</main>
	);
}
