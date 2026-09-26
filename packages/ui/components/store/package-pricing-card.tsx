"use client";

import { useTranslation } from "@flow-like/locales";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Loader2, Tag } from "lucide-react";
import Link from "next/link";
import { useEffect, useId, useState } from "react";
import { useInvoke } from "../../hooks/use-invoke";
import { useBackend } from "../../state/backend-state";
import type { GenericFetcher } from "../pages/store/store-package-detail";
import { SellerTermsConsentCard } from "../payments/app-payment-settings";
import { EuroAmountInput } from "../payments/euro-amount-input";
import { PaymentError } from "../payments/payment-parts";
import { type ConnectAccount, paymentMoney } from "../payments/types";
import { usePaymentQuery, usePayments } from "../payments/use-payments";
import {
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	Label,
} from "../ui";

const packagePath = (packageId: string, route: string) =>
	`registry/package/${encodeURIComponent(packageId)}/${route}`;

function SellerAccountNotice({ account }: { account: ConnectAccount }) {
	const { t } = useTranslation("payments");
	if (account.platformOwned) {
		return (
			<p className="rounded-lg border p-4 text-sm">
				{t(
					"platformPackageReceives",
					"You are a Flow-Like administrator. Sales of this package go to Flow-Like's Stripe account.",
				)}
			</p>
		);
	}
	return (
		<div className="space-y-2 rounded-lg border p-4 text-sm">
			<p>
				{account.canSell
					? t(
							"packageSellerReady",
							"Your payment account can receive sales of this package.",
						)
					: t(
							"packageSellerSetupNeeded",
							"Complete your payment account setup before you set a price for this package.",
						)}
			</p>
			<Link
				className="inline-block underline underline-offset-4"
				href="/account/payouts"
			>
				{t("manageAccount", "Manage your payment account")}
			</Link>
		</div>
	);
}

/**
 * The owner's price for a registry package (EUR cents, 0 = free). With the
 * marketplace on, selling also needs the seller terms and a payout account.
 */
export function PackagePricingCard({
	packageId,
	price,
	visibility,
	fetcher,
	auth,
}: {
	packageId: string;
	price: number;
	visibility?: string;
	fetcher: GenericFetcher;
	auth?: unknown;
}) {
	const { t, i18n } = useTranslation("payments");
	const inputId = useId();
	const backend = useBackend();
	const queryClient = useQueryClient();
	const payments = usePayments();
	const profile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const [draft, setDraft] = useState(price);
	const [draftValid, setDraftValid] = useState(true);
	useEffect(() => setDraft(price), [price]);

	const marketplace = payments.config?.marketplace_enabled === true;
	const minimum = payments.config?.marketplace_min_amount ?? 0;
	const maximum = payments.config?.max_payment_amount ?? 0;
	const selling = marketplace && (draft > 0 || price > 0);
	const account = usePaymentQuery<ConnectAccount>(
		"user/payments/connect",
		selling,
	);
	const inRange =
		draft === 0 ||
		((minimum <= 0 || draft >= minimum) && (maximum <= 0 || draft <= maximum));

	const save = useMutation({
		mutationFn: async (next: number) => {
			if (!profile.data) throw new Error("Profile not loaded");
			return fetcher<{ price: number }>(
				profile.data.hub_profile,
				packagePath(packageId, "price"),
				{
					method: "PATCH",
					body: JSON.stringify({ price: next }),
					headers: { "Content-Type": "application/json" },
				},
				auth,
			);
		},
		onSuccess: () =>
			queryClient.invalidateQueries({
				queryKey: ["registry-package", packageId],
			}),
	});

	const money = (minor: number) => paymentMoney(minor, "eur", i18n.language);

	return (
		<>
			<Card>
				<CardHeader>
					<CardTitle className="flex items-center gap-2 text-base">
						<Tag className="h-4 w-4" />
						{t("packagePrice", "Package price")}
					</CardTitle>
					<CardDescription>
						{t(
							"packagePriceDescription",
							"What a buyer pays once to install this package. At zero the package is free.",
						)}
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					<p className="text-2xl font-semibold tabular-nums">
						{price > 0 ? money(price) : t("free", "Free")}
					</p>
					{visibility === "public_request_access" && (
						<p className="text-sm text-muted-foreground">
							{t(
								"packagePriceRequiresOpenListing",
								"Packages that need access approval cannot be sold. Make the listing public to set a price.",
							)}
						</p>
					)}
					<PaymentError error={save.error} />
					<form
						className="grid gap-2"
						onSubmit={(event) => {
							event.preventDefault();
							if (draftValid && inRange && draft !== price) save.mutate(draft);
						}}
					>
						<Label htmlFor={inputId}>{t("priceEur", "Price (EUR)")}</Label>
						<div className="flex flex-wrap items-start gap-3">
							<div className="min-w-40 flex-1">
								<EuroAmountInput
									id={inputId}
									value={draft}
									disabled={save.isPending}
									onChange={(minor) => {
										setDraft(minor);
										if (!save.isIdle) save.reset();
									}}
									onValidityChange={setDraftValid}
								/>
							</div>
							<Button
								type="submit"
								disabled={
									save.isPending || !draftValid || !inRange || draft === price
								}
							>
								{save.isPending && (
									<Loader2 className="mr-2 h-4 w-4 animate-spin" />
								)}
								{t("savePrice", "Save price")}
							</Button>
						</div>
						{minimum > 0 && maximum > 0 && (
							<p
								className={
									inRange
										? "text-xs text-muted-foreground"
										: "text-xs text-destructive"
								}
							>
								{t(
									"paidPriceRange",
									"Paid prices range from {{minimum}} to {{maximum}}.",
									{ minimum: money(minimum), maximum: money(maximum) },
								)}
							</p>
						)}
					</form>
					{save.isSuccess && (
						<output className="text-sm">
							{t("priceSaved", "Price saved.")}
						</output>
					)}
					{selling && <PaymentError error={account.error} />}
					{selling && account.data && (
						<SellerAccountNotice account={account.data} />
					)}
				</CardContent>
			</Card>
			{selling && (
				<SellerTermsConsentCard
					termsPath={packagePath(packageId, "marketplace/terms")}
					platformOwned={account.data?.platformOwned}
					description={t(
						"packageSellerTermsBeforePrice",
						"Accept the current seller terms before you sell this package.",
					)}
				/>
			)}
		</>
	);
}
