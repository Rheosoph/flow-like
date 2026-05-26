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
	useStoreData,
} from "@flow-like/flow-like-ui";
import { useRouter, useSearchParams } from "next/navigation";
import { useEffect } from "react";
import { toast } from "sonner";
import { EVENT_CONFIG } from "../../lib/event-config";

export default function Page() {
	const searchParams = useSearchParams();
	const router = useRouter();
	const id = searchParams.get("id") ?? undefined;
	const purchaseStatus = searchParams.get("purchase");
	const {
		appData,
		metaData,
		isMember,
		isPurchasing,
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

	useEffect(() => {
		if (!purchaseStatus) return;

		if (purchaseStatus === "success") {
			toast.success("Purchase successful! You now have access to this app.", {
				duration: 5000,
			});
			refetchAppData();
		} else if (purchaseStatus === "canceled") {
			toast.info("Purchase was canceled. You can try again anytime.");
		}

		const url = new URL(window.location.href);
		url.searchParams.delete("purchase");
		router.replace(url.pathname + url.search, { scroll: false });
	}, [purchaseStatus, refetchAppData, router]);

	if (!id) {
		return (
			<div className="flex-1 flex items-center justify-center p-6">
				<StoreEmptyState
					title="No app selected"
					description="Choose an app from the store to view its details."
				/>
			</div>
		);
	}

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

	if (isError || notFound || !appData || !metaData) {
		return (
			<div className="flex-1 flex items-center justify-center p-6">
				<StoreEmptyState
					title={isError ? "Failed to load app" : "App not found"}
					description={
						isError
							? "Something went wrong. Please try again later."
							: "This app may be private or no longer available."
					}
				/>
			</div>
		);
	}

	return (
		<main className="flex-col flex grow max-h-full overflow-auto min-h-0 w-full">
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
				isPurchasing={isPurchasing}
				onUse={onUse}
				onSettings={onSettings}
				onBuy={onBuy}
				onJoinOrRequest={onJoinOrRequest}
				actionsExtra={
					appData.allow_forking ? (
						<StoreForkButton appId={id} appName={appName} target="online" />
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

				<AppReviewsSection appId={id} onReviewChanged={refetchAppData} />

				<StoreRecommendations />
			</div>
		</main>
	);
}
