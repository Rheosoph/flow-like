"use client";
import { useTranslation } from "@flow-like/locales";
import {
	Button,
	SubscriptionPage,
	useBackend,
	useHub,
	useInvoke,
} from "@flow-like/flow-like-ui";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Loader2 } from "lucide-react";
import { useCallback, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";

export default function SubscriptionPageWrapper() {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const hub = useHub();
	const auth = useAuth();
	const [loading, setLoading] = useState(true);

	const isPremiumEnabled = hub.hub?.features?.premium ?? false;

	const pricing = useInvoke(
		backend.userState.getPricing,
		backend.userState,
		[],
		isPremiumEnabled && auth.isAuthenticated,
	);

	const handleUpgrade = useCallback(
		async (tier: string) => {
			try {
				const response = await backend.userState.createSubscription({
					tier,
					success_url: `${window.location.origin}/subscription?success=true`,
					cancel_url: `${window.location.origin}/subscription?canceled=true`,
				});

				await openUrl(response.checkout_url);
			} catch (error) {
				console.error("Failed to create subscription checkout:", error);
				toast.error("Failed to start checkout process");
			}
		},
		[backend.userState],
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
					<h3>{t('pleaseLogInToViewSubscriptionOptions', 'Please log in to view subscription options.')}</h3>
					<Button onClick={() => auth.signinRedirect()} className="mt-4">
						{t('logIn', 'Log In')}
					</Button>
				</div>
			</main>
		);
	}

	if (pricing.isLoading) {
		return (
			<main className="flex flex-row items-center justify-center w-full flex-1 min-h-0 py-12">
				<Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
			</main>
		);
	}

	if (!pricing.data) {
		return (
			<main className="flex flex-row items-center justify-center w-full flex-1 min-h-0 py-12">
				<div className="text-center p-6">
					<h3 className="text-xl font-semibold mb-2">
						{t('premiumFeaturesNotAvailable', 'Premium Features Not Available')}
					</h3>
					<p className="text-muted-foreground">
						{t('premiumSubscriptionFeaturesAreNotEnabledOnThisInstance', 'Premium subscription features are not enabled on this instance.')}
					</p>
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
			{!pricing.data && (
				<div className="flex flex-row items-center justify-center w-full flex-1 min-h-0 py-12">
					<div className="text-center p-6">
						<h3 className="text-xl font-semibold mb-2">
							{t('failedToLoadPricingInformation', 'Failed to load pricing information.')}
						</h3>
						<p className="text-muted-foreground">{t('pleaseTryAgainLater', 'Please try again later.')}</p>
					</div>
				</div>
			)}
		</main>
	);
}
