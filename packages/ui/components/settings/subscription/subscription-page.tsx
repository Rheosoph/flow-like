"use client";

import { ArrowUpRight, Building2, Crown } from "lucide-react";
import { Suspense, useCallback, useMemo, useRef, useState } from "react";
import { useSearchParams } from "next/navigation";
import { useAuth } from "react-oidc-context";
import { useInvoke } from "../../../hooks/use-invoke";
import { formatQuotaDate } from "../../../lib/quota";
import {
	formatRuntimeSeconds,
	getRuntimeSample,
} from "../../../lib/runtime-estimate";
import { useBackend } from "../../../state/backend-state";
import type { IPricingResponse } from "../../../state/backend-state/user-state";
import { Button } from "../../ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../../ui/tabs";
import {
	TIER_COLORS,
	TIER_ICONS,
	TIER_ORDER,
	TierCard,
} from "../../upgrade/tier-card";
import { TierComparison } from "../../upgrade/tier-comparison";
import { RuntimeCalculator } from "../../upgrade/runtime-calculator";
import {
	EnterpriseContent,
	FreeUseTips,
	TrustRow,
} from "../../upgrade/upgrade-dialog";
import { UsageOverview } from "./usage-overview";

interface SubscriptionPageProps {
	pricing: IPricingResponse;
	onUpgrade: (
		tier: string,
		priceId?: string,
		interval?: "month" | "year",
	) => Promise<void>;
	onManageBilling: () => Promise<void>;
	isPremiumEnabled?: boolean;
}

function SubscriptionPageContent({
	pricing,
	onUpgrade,
	onManageBilling,
	isPremiumEnabled = true,
}: Readonly<SubscriptionPageProps>) {
	const backend = useBackend();
	const auth = useAuth();
	const searchParams = useSearchParams();
	const tab = searchParams?.get("tab") === "usage" ? "usage" : "plan";
	const usageTab = useRef<HTMLButtonElement>(null);
	const [billingInterval, setBillingInterval] = useState<"month" | "year">(
		"month",
	);
	const [loadingTier, setLoadingTier] = useState<string | null>(null);
	const [billingLoading, setBillingLoading] = useState(false);
	const usage = useInvoke(
		backend.userState.getQuotaUsage,
		backend.userState,
		[],
		isPremiumEnabled &&
			auth.isAuthenticated &&
			pricing.conversion?.mode !== "enterprise",
		[auth.user?.profile.sub],
	);

	const changeTab = (value: string) => {
		const url = new URL(window.location.href);
		if (value === "usage") url.searchParams.set("tab", "usage");
		else url.searchParams.delete("tab");
		// Next copies its router state and updates useSearchParams for this call.
		window.history.replaceState(null, "", url);
	};
	const handleUpgrade = useCallback(
		async (tier: string, priceId?: string, interval?: "month" | "year") => {
			setLoadingTier(tier);
			try {
				await onUpgrade(tier, priceId, interval);
			} finally {
				setLoadingTier(null);
			}
		},
		[onUpgrade],
	);
	const handleManageBilling = useCallback(async () => {
		setBillingLoading(true);
		try {
			await onManageBilling();
		} finally {
			setBillingLoading(false);
		}
	}, [onManageBilling]);
	const sortedTiers = useMemo(
		() =>
			Object.entries(pricing.tiers).sort(([a], [b]) => {
				const aIndex = TIER_ORDER.indexOf(a);
				const bIndex = TIER_ORDER.indexOf(b);
				return (
					(aIndex < 0 ? TIER_ORDER.length : aIndex) -
					(bIndex < 0 ? TIER_ORDER.length : bIndex)
				);
			}),
		[pricing.tiers],
	);
	const current = pricing.tiers[pricing.current_tier];
	const runtimeSample = useMemo(
		() => getRuntimeSample(usage.data),
		[usage.data],
	);
	const currentName =
		current?.display_name ?? current?.name ?? pricing.current_tier;
	const hasBilling =
		pricing.current_tier !== "FREE" && pricing.current_tier !== "ENTERPRISE";

	if (!isPremiumEnabled)
		return (
			<div className="mx-auto max-w-3xl px-6 py-16 text-center">
				<Crown className="mx-auto mb-4 h-10 w-10 text-muted-foreground" />
				<h1 className="text-2xl font-semibold">
					Subscriptions are unavailable on this instance
				</h1>
				<p className="mt-2 text-muted-foreground">
					Contact your administrator for account and capacity options.
				</p>
			</div>
		);
	if (pricing.conversion?.mode === "enterprise")
		return (
			<div className="container mx-auto max-w-3xl p-6 py-16">
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

	return (
		<div className="container mx-auto max-w-6xl px-4 py-6 sm:px-6 sm:py-8 space-y-6">
			<header>
				<h1 className="text-2xl font-semibold tracking-tight sm:text-3xl">
					Your plan and usage
				</h1>
				<p className="mt-2 text-sm text-muted-foreground">
					Manage your subscription and see where your cloud allowance goes.
				</p>
			</header>
			<section
				aria-label="Current plan"
				className="flex flex-wrap items-center justify-between gap-5 rounded-2xl border bg-card p-5 sm:p-6"
			>
				<div className="flex min-w-0 items-center gap-4">
					<div
						className={`flex h-12 w-12 shrink-0 items-center justify-center rounded-xl ${TIER_COLORS[pricing.current_tier] ?? TIER_COLORS.FREE}`}
					>
						{TIER_ICONS[pricing.current_tier] ?? TIER_ICONS.FREE}
					</div>
					<div>
						<p className="text-xs font-medium text-muted-foreground">
							Current plan
						</p>
						<h2 className="mt-0.5 text-xl font-semibold">{currentName}</h2>
						<p className="mt-1 text-xs text-muted-foreground">
							{usage.data
								? `Monthly allowances renew ${formatQuotaDate(usage.data.periodEnd)}.`
								: "Local execution stays free on every plan."}
						</p>
					</div>
				</div>
				<div className="flex flex-wrap gap-2">
					{tab !== "usage" && (
						<Button
							variant="ghost"
							onClick={() => {
								changeTab("usage");
								usageTab.current?.focus();
							}}
						>
							View usage <ArrowUpRight className="h-4 w-4" />
						</Button>
					)}
					{hasBilling && (
						<Button
							variant="outline"
							disabled={billingLoading}
							onClick={handleManageBilling}
						>
							{billingLoading ? "Opening billing…" : "Manage billing"}
						</Button>
					)}
				</div>
			</section>
			<Tabs value={tab} onValueChange={changeTab} className="gap-6">
				<TabsList aria-label="Subscription settings" className="h-11">
					<TabsTrigger value="plan" className="px-5">
						Plan &amp; billing
					</TabsTrigger>
					<TabsTrigger ref={usageTab} value="usage" className="px-5">
						Usage
					</TabsTrigger>
				</TabsList>
				<TabsContent value="plan" className="space-y-6">
					<div className="flex flex-wrap items-end justify-between gap-4">
						<div className="max-w-xl">
							<h2 className="text-xl font-semibold tracking-tight">
								{pricing.conversion?.headline ?? "Choose your plan"}
							</h2>
							<p className="mt-1.5 text-sm text-muted-foreground">
								More cloud runtime, hosted AI and storage as your apps grow.
							</p>
						</div>
						<div
							className="flex rounded-lg border bg-muted/40 p-1"
							role="group"
							aria-label="Billing interval"
						>
							<Button
								size="sm"
								variant={billingInterval === "month" ? "default" : "ghost"}
								aria-pressed={billingInterval === "month"}
								onClick={() => setBillingInterval("month")}
							>
								Monthly
							</Button>
							<Button
								size="sm"
								variant={billingInterval === "year" ? "default" : "ghost"}
								aria-pressed={billingInterval === "year"}
								onClick={() => setBillingInterval("year")}
							>
								Annual
							</Button>
						</div>
					</div>
					<div className="grid grid-cols-1 gap-4 pt-2 md:grid-cols-2 xl:grid-cols-4">
						{sortedTiers
							.filter(([key]) => key !== "ENTERPRISE")
							.map(([key, tier]) => (
								<TierCard
									key={key}
									tierKey={key}
									tier={{
										...tier,
										price:
											tier.prices?.find(
												(price) =>
													price.interval === billingInterval &&
													price.currency === "eur",
											) ??
											(billingInterval === "month" ? tier.price : undefined),
									}}
									currentTier={pricing.current_tier}
									onUpgrade={handleUpgrade}
									onManageBilling={handleManageBilling}
									isLoading={loadingTier === key}
									showDetails={false}
									runtimeAverageMs={runtimeSample?.averageRuntimeMs}
								/>
							))}
					</div>
					<p className="text-xs leading-relaxed text-muted-foreground">
						{billingInterval === "year"
							? "Annual prices cover a full year. "
							: "Prices are per month. "}
						Cloud runtime and hosted AI allowances renew monthly. Storage
						measures current usage. Applicable taxes appear at checkout.
					</p>
					{runtimeSample && (
						<p className="text-xs text-muted-foreground">
							Run estimates use your recent average of{" "}
							{formatRuntimeSeconds(runtimeSample.averageRuntimeMs)} seconds per
							cloud run and each plan's cloud-start limit. Hosted AI and other
							limits still apply.
						</p>
					)}
					<RuntimeCalculator
						sample={runtimeSample}
						plans={sortedTiers
							.filter(([key]) => key !== "ENTERPRISE")
							.map(([id, tier]) => ({
								id,
								name: tier.display_name ?? tier.name ?? id,
								runtimeMs: tier.max_runtime_ms,
								cloudStarts: tier.max_remote_executions,
							}))}
					/>
					<TierComparison tiers={pricing.tiers} />
					<FreeUseTips />
					{pricing.tiers.ENTERPRISE && (
						<div className="flex flex-wrap items-center justify-between gap-4 rounded-xl border p-5">
							<div className="flex items-start gap-3">
								<Building2 className="mt-0.5 h-5 w-5 shrink-0 text-muted-foreground" />
								<div>
									<h3 className="font-medium">
										Need a plan for your organization?
									</h3>
									<p className="mt-1 text-sm text-muted-foreground">
										Enterprise capacity, deployment and support by agreement.
									</p>
								</div>
							</div>
							<Button asChild variant="outline">
								<a href={pricing.tiers.ENTERPRISE.contact_url}>
									Talk to sales <ArrowUpRight className="h-4 w-4" />
								</a>
							</Button>
						</div>
					)}
					<TrustRow />
					{hasBilling && (
						<p className="border-t pt-5 text-xs text-muted-foreground">
							Use Manage billing above to change your payment method, view
							invoices or cancel your subscription.
						</p>
					)}
				</TabsContent>
				<TabsContent value="usage">
					<UsageOverview />
				</TabsContent>
			</Tabs>
		</div>
	);
}

export function SubscriptionPage(props: Readonly<SubscriptionPageProps>) {
	return (
		<Suspense
			fallback={
				<p className="p-6 text-sm text-muted-foreground" role="status">
					Loading subscription settings…
				</p>
			}
		>
			<SubscriptionPageContent {...props} />
		</Suspense>
	);
}
