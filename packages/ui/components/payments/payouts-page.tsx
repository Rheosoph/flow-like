"use client";

import { useTranslation } from "@flow-like/locales";
import Link from "next/link";
import { useState } from "react";
import { Button } from "../ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "../ui/card";
import { PaymentConsent, PaymentError, PaymentPage } from "./payment-parts";
import { type ConnectAccount, type PaymentTerms, paymentUrl } from "./types";
import { usePaymentQuery, usePayments } from "./use-payments";

export function PayoutsPage() {
	const { identity } = usePayments();
	return <PayoutsPageContent key={identity.join(":")} />;
}

function PayoutsPageContent() {
	const { t } = useTranslation("payments");
	const payments = usePayments();
	const account = usePaymentQuery<ConnectAccount>("user/payments/connect");
	const countries = usePaymentQuery<{ countries: string[] }>(
		"user/payments/connect/countries",
		payments.config?.onboarding_enabled === true,
	);
	const [country, setCountry] = useState("");
	const [terms, setTerms] = useState<PaymentTerms | null>(null);
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<unknown>();
	const [link, setLink] = useState<string>();
	const [confirmDisconnect, setConfirmDisconnect] = useState(false);
	const current = account.data;
	const act = async (operation: "onboarding" | "disconnect" | "reconnect") => {
		setBusy(true);
		setError(undefined);
		setLink(undefined);
		try {
			const result = await payments.request<{ url?: string }>(
				`user/payments/connect/${operation}`,
				"POST",
				operation === "onboarding"
					? {
							country: current?.country || country,
							termsVersion: terms?.version,
							locale: terms?.locale,
						}
					: undefined,
			);
			if (result.url) setLink(paymentUrl(result.url));
			setConfirmDisconnect(false);
			await payments.refresh();
		} catch (error) {
			setError(error);
		} finally {
			setBusy(false);
		}
	};
	return (
		<PaymentPage title={t("payouts", "Payments and payouts")}>
			<p className="text-muted-foreground">
				{current?.platformOwned
					? t(
							"platformPaymentsDescription",
							"Your apps collect marketplace and flow payments in Flow-Like's Stripe account. Proceeds belong to Flow-Like. Connected account setup is unavailable for platform administrators.",
						)
					: t(
							"payoutsDescription",
							"Connect a Stripe account to receive payments for your apps. Stripe collects your business and payout details.",
						)}
			</p>
			<PaymentError error={error || account.error} />
			{account.isLoading && <output>{t("loading", "Loading…")}</output>}
			{current && (
				<Card>
					<CardHeader>
						<CardTitle>{t("accountStatus", "Account status")}</CardTitle>
					</CardHeader>
					<CardContent className="space-y-4">
						<dl className="grid gap-3 text-sm sm:grid-cols-2">
							<div>
								<dt className="text-muted-foreground">
									{t("status", "Status")}
								</dt>
								<dd className="font-medium">
									{String(
										t(`accountState.${current.state}`, {
											defaultValue: current.state.replaceAll("_", " "),
										}),
									)}
								</dd>
							</div>
							<div>
								<dt className="text-muted-foreground">
									{t("paymentCollection", "Payment collection")}
								</dt>
								<dd>
									{current.canAcceptPayments
										? t("available", "Available")
										: t("unavailable", "Unavailable")}
								</dd>
							</div>
							{!current.platformOwned && (
								<div>
									<dt className="text-muted-foreground">
										{t("bankPayouts", "Bank payouts")}
									</dt>
									<dd>
										{current.payoutsEnabled
											? t("enabled", "Enabled")
											: t("notEnabled", "Not enabled")}
									</dd>
								</div>
							)}
							{current.country && (
								<div>
									<dt className="text-muted-foreground">
										{t("country", "Country")}
									</dt>
									<dd>{current.country}</dd>
								</div>
							)}
						</dl>
						{current.business?.name && <p>{current.business.name}</p>}
						{current.requirements?.currently_due?.length ? (
							<div>
								<p className="text-sm font-medium">
									{t("detailsNeeded", "Details to complete in Stripe")}
								</p>
								<ul className="mt-2 list-inside list-disc text-sm text-muted-foreground">
									{current.requirements.currently_due.map((item) => (
										<li key={item}>{item.replaceAll("_", " ")}</li>
									))}
								</ul>
							</div>
						) : null}
						{current.requirements?.disabled_reason && (
							<p className="text-sm">
								{current.requirements.disabled_reason.replaceAll("_", " ")}
							</p>
						)}
						<div className="flex flex-wrap gap-3">
							<Button
								variant="outline"
								disabled={account.isFetching}
								onClick={() => void account.refetch()}
							>
								{t("refreshStatus", "Refresh status")}
							</Button>
							{paymentUrl(current.dashboardUrl) && (
								<Button asChild variant="outline">
									<a
										href={paymentUrl(current.dashboardUrl)}
										target="_blank"
										rel="noopener noreferrer"
									>
										{t("stripeDashboard", "Open Stripe dashboard")}
									</a>
								</Button>
							)}
						</div>
					</CardContent>
				</Card>
			)}
			{link && !current?.platformOwned && (
				<div aria-live="polite" className="space-y-3 rounded-lg border p-4">
					<p>
						{t(
							"onboardingReady",
							"Your Stripe setup link is ready. Open it to continue, then return here and refresh your status.",
						)}
					</p>
					<Button asChild>
						<a href={link} target="_blank" rel="noopener noreferrer">
							{t("continueStripe", "Continue in Stripe")}
						</a>
					</Button>
				</div>
			)}
			{payments.config?.onboarding_enabled &&
				current &&
				!current.platformOwned &&
				!current.canSell &&
				current.state !== "disconnected" && (
					<Card>
						<CardHeader>
							<CardTitle>
								{t("completeSetup", "Complete payment setup")}
							</CardTitle>
						</CardHeader>
						<CardContent className="space-y-4">
							{!current.country && (
								<label className="grid gap-2 text-sm">
									{t("businessCountry", "Business country")}
									<select
										className="h-10 rounded-md border bg-background px-3"
										value={country}
										onChange={(event) => setCountry(event.target.value)}
									>
										<option value="">
											{t("chooseCountry", "Choose a country")}
										</option>
										{countries.data?.countries.map((code) => (
											<option key={code} value={code}>
												{new Intl.DisplayNames(["en"], { type: "region" }).of(
													code,
												) ?? code}
											</option>
										))}
									</select>
								</label>
							)}
							<PaymentError error={countries.error} />
							<PaymentConsent
								kind="PAYMENTS_OWNER_TERMS"
								value={terms}
								onChange={setTerms}
							/>
							<Button
								disabled={busy || !terms || !(current.country || country)}
								onClick={() => void act("onboarding")}
							>
								{t("setUpStripe", "Set up payments with Stripe")}
							</Button>
						</CardContent>
					</Card>
				)}
			{!current?.platformOwned && !payments.config?.onboarding_enabled && (
				<p className="text-sm text-muted-foreground">
					{t(
						"onboardingPaused",
						"New payment account setup is currently unavailable. Existing account details remain accessible.",
					)}
				</p>
			)}
			{current &&
				!current.platformOwned &&
				![
					"not_started",
					"unconnected",
					"not_connected",
					"disconnected",
				].includes(current.state) && (
					<Card>
						<CardHeader>
							<CardTitle>{t("disconnect", "Disconnect payments")}</CardTitle>
						</CardHeader>
						<CardContent className="space-y-3">
							<p className="text-sm text-muted-foreground">
								{t(
									"disconnectExplanation",
									"Disconnecting stops new payments. Existing orders and refunds remain available. Your Stripe account stays open.",
								)}
							</p>
							{confirmDisconnect ? (
								<div className="flex gap-3">
									<Button
										variant="destructive"
										disabled={busy}
										onClick={() => void act("disconnect")}
									>
										{t("confirmDisconnect", "Confirm disconnect")}
									</Button>
									<Button
										variant="outline"
										onClick={() => setConfirmDisconnect(false)}
									>
										{t("keepConnected", "Keep connected")}
									</Button>
								</div>
							) : (
								<Button
									variant="outline"
									onClick={() => setConfirmDisconnect(true)}
								>
									{t("disconnect", "Disconnect payments")}
								</Button>
							)}
						</CardContent>
					</Card>
				)}
			{!current?.platformOwned &&
				current?.state === "disconnected" &&
				payments.config?.onboarding_enabled && (
					<Button disabled={busy} onClick={() => void act("reconnect")}>
						{t("reconnect", "Reconnect existing account")}
					</Button>
				)}
			<Link
				href="/account/earnings"
				className="block text-sm underline underline-offset-4"
			>
				{t("earnings", "Earnings and refunds")}
			</Link>
			<Link
				className="text-sm underline underline-offset-4"
				href="/account/purchases"
			>
				{t("viewPurchases", "View your purchases")}
			</Link>
		</PaymentPage>
	);
}
