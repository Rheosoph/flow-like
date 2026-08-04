"use client";

import { Crown } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import type { IPricingResponse } from "../../../state/backend-state/user-state";
import { Button } from "../../ui/button";
import { Separator } from "../../ui/separator";
import { TIER_ORDER, TierCard } from "../../upgrade/tier-card";
import { EnterpriseContent, TrustRow } from "../../upgrade/upgrade-dialog";

interface SubscriptionPageProps {
	pricing: IPricingResponse;
	onUpgrade: (tier: string) => Promise<void>;
	onManageBilling: () => Promise<void>;
	isPremiumEnabled?: boolean;
}

export function SubscriptionPage({
	pricing,
	onUpgrade,
	onManageBilling,
	isPremiumEnabled = true,
}: Readonly<SubscriptionPageProps>) {
	const [loadingTier, setLoadingTier] = useState<string | null>(null);

	const handleUpgrade = useCallback(
		async (tier: string) => {
			setLoadingTier(tier);
			try {
				await onUpgrade(tier);
			} finally {
				setLoadingTier(null);
			}
		},
		[onUpgrade],
	);

	const sortedTiers = useMemo(() => {
		const entries = Object.entries(pricing.tiers);
		return entries.sort((a, b) => {
			const aIndex = TIER_ORDER.indexOf(a[0]);
			const bIndex = TIER_ORDER.indexOf(b[0]);
			if (aIndex === -1 && bIndex === -1) return 0;
			if (aIndex === -1) return 1;
			if (bIndex === -1) return -1;
			return aIndex - bIndex;
		});
	}, [pricing.tiers]);

	if (!isPremiumEnabled) {
		return (
			<div className="container max-w-4xl mx-auto p-6">
				<div className="text-center py-12">
					<Crown className="h-12 w-12 text-muted-foreground mx-auto mb-4" />
					<h2 className="text-2xl font-bold mb-2">Premium Features Disabled</h2>
					<p className="text-muted-foreground">
						Premium subscription features are not available on this instance.
					</p>
				</div>
			</div>
		);
	}

	if (pricing.conversion?.mode === "enterprise") {
		return (
			<div className="container max-w-3xl mx-auto p-6 py-16">
				<EnterpriseContent
					asDialogTitle={false}
					headline={pricing.conversion.headline ?? undefined}
					contact={{
						name: pricing.conversion.contact_name,
						email: pricing.conversion.contact_email,
						url: pricing.conversion.contact_url,
						message: pricing.conversion.contact_message,
					}}
					contactLabel="Contact Your Admin"
				/>
			</div>
		);
	}

	return (
		<div className="container max-w-6xl mx-auto p-6 space-y-8">
			<div className="text-center space-y-2">
				<h1 className="text-3xl font-bold tracking-tight">
					{pricing.conversion?.headline ?? "Choose your plan"}
				</h1>
				<p className="text-muted-foreground max-w-2xl mx-auto">
					{pricing.conversion?.subheadline ??
						"Unlock more projects, premium AI models and faster cloud executions. Every plan includes the full platform."}
				</p>
			</div>

			<div className="grid gap-6 pt-4 md:grid-cols-2 lg:grid-cols-4">
				{sortedTiers.map(([key, tier]) => (
					<TierCard
						key={key}
						tierKey={key}
						tier={tier}
						currentTier={pricing.current_tier}
						onUpgrade={handleUpgrade}
						onManageBilling={onManageBilling}
						isLoading={loadingTier === key}
					/>
				))}
			</div>

			<TrustRow />

			{pricing.current_tier !== "FREE" && (
				<>
					<Separator />
					<div className="flex flex-col items-center gap-4">
						<p className="text-sm text-muted-foreground">
							Need to update your payment method or cancel your subscription?
						</p>
						<Button variant="outline" onClick={onManageBilling}>
							Manage Billing
						</Button>
					</div>
				</>
			)}
		</div>
	);
}
