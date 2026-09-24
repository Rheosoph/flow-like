"use client";
import {
	Button,
	SubscriptionPage,
	useBackend,
	useHub,
	useInvoke,
} from "@flow-like/flow-like-ui";
import { isRecord } from "@flow-like/flow-like-ui/lib/response-shape";
import { useTranslation } from "@flow-like/locales";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Loader2 } from "lucide-react";
import { useCallback } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";

export default function SubscriptionPageWrapper() {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const hub = useHub();
	const auth = useAuth();

	const isPremiumEnabled = hub.hub?.features?.premium ?? false;

	const pricing = useInvoke(
		backend.userState.getPricing,
		backend.userState,
		[],
		isPremiumEnabled && auth.isAuthenticated,
		[auth.user?.profile.sub],
	);

	const handleUpgrade = useCallback(
		async (tier: string, priceId?: string, interval?: "month" | "year") => {
			try {
				const appDomain = hub.hub?.app;
				if (!appDomain)
					throw new Error("The hosted app address is unavailable.");
				const origin = appDomain.startsWith("http")
					? appDomain.replace(/\/+$/, "")
					: `https://${appDomain}`;
				const response = await backend.userState.createSubscription({
					tier,
					price_id: priceId,
					interval,
					success_url: `${origin}/subscription?success=true`,
					cancel_url: `${origin}/subscription?canceled=true`,
				});

				await openUrl(response.checkout_url);
			} catch (error) {
				console.error("Failed to create subscription checkout:", error);
				toast.error("Failed to start checkout process");
			}
		},
		[backend.userState, hub.hub?.app],
	);

	const handleManageBilling = useCallback(async () => {
		try {
			const billingSession = await backend.userState.getBillingSession();

			await openUrl(billingSession.url);
		} catch (error) {
			console.error("Failed to get billing session:", error);
			toast.error("Failed to open billing portal");
		}
	}, [backend.userState]);

	if (!auth.isAuthenticated) {
		return (
			<main className="flex flex-row items-center justify-center w-full flex-1 min-h-0 py-12">
				<div className="text-center p-6 border rounded-lg shadow-lg bg-card">
					<h3>
						{t(
							"pleaseLogInToViewSubscriptionOptions",
							"Please log in to view subscription options.",
						)}
					</h3>
					<Button onClick={() => auth.signinRedirect()} className="mt-4">
						{t("logIn", "Log In")}
					</Button>
				</div>
			</main>
		);
	}

	if (pricing.isLoading) {
		return (
			<main className="flex flex-row items-center justify-center w-full flex-1 min-h-0 py-12">
				<div
					role="status"
					className="flex items-center gap-2 text-sm text-muted-foreground"
				>
					<Loader2 className="h-5 w-5 animate-spin" aria-hidden="true" />
					Loading subscription settings…
				</div>
			</main>
		);
	}

	if (!hub.hub) {
		return (
			<main className="flex w-full flex-1 min-h-0 items-center justify-center py-12">
				<div className="max-w-md space-y-3 px-6 text-center">
					<h1 className="text-xl font-semibold">
						Waiting for your instance settings
					</h1>
					<p className="text-sm text-muted-foreground">
						Your instance settings are not available yet. Check your connection
						and retry if this continues.
					</p>
					<Button variant="outline" onClick={() => hub.refetch()}>
						Retry instance settings
					</Button>
				</div>
			</main>
		);
	}

	if (!isPremiumEnabled) {
		return (
			<main className="flex flex-row items-center justify-center w-full flex-1 min-h-0 py-12">
				<div className="text-center p-6">
					<h3 className="text-xl font-semibold mb-2">
						{t("premiumFeaturesNotAvailable", "Premium Features Not Available")}
					</h3>
					<p className="text-muted-foreground">
						{t(
							"premiumSubscriptionFeaturesAreNotEnabledOnThisInstance",
							"Premium subscription features are not enabled on this instance.",
						)}
					</p>
				</div>
			</main>
		);
	}

	if (!isRecord(pricing.data) || !isRecord(pricing.data.tiers)) {
		return (
			<main className="flex w-full flex-1 min-h-0 items-center justify-center py-12">
				<div className="max-w-md space-y-3 px-6 text-center">
					<h1 className="text-xl font-semibold">
						Subscription settings are temporarily unavailable
					</h1>
					<p className="text-sm text-muted-foreground">
						We couldn't load your pricing information. Check your connection and
						try again.
					</p>
					<Button
						variant="outline"
						disabled={pricing.isFetching}
						onClick={() => pricing.refetch()}
					>
						{pricing.isFetching ? "Retrying…" : "Try again"}
					</Button>
				</div>
			</main>
		);
	}

	return (
		<main className="flex flex-col w-full flex-1 min-h-0 overflow-auto">
			{pricing.data && (
				<SubscriptionPage
					pricing={pricing.data}
					onUpgrade={handleUpgrade}
					onManageBilling={handleManageBilling}
					isPremiumEnabled={isPremiumEnabled}
				/>
			)}
		</main>
	);
}
