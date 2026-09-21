"use client";

import { useTranslation } from "@flow-like/locales";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { useState } from "react";
import { useAppPermissions } from "../../hooks/use-app-permissions";
import { RolePermissions } from "../../lib/permission/role-permission";
import { Button } from "../ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "../ui/card";
import { Input } from "../ui/input";
import { Switch } from "../ui/switch";
import { PaymentConsent, PaymentError, PaymentPage } from "./payment-parts";
import {
	type AppPaymentSettings,
	type PaymentReadiness,
	type PaymentTerms,
	amountInput,
	parseEuroAmount,
	paymentMoney,
} from "./types";
import { usePaymentQuery, usePayments } from "./use-payments";

function SettingsForm({
	appId,
	value,
}: { appId: string; value: AppPaymentSettings }) {
	const { t } = useTranslation("payments");
	const payments = usePayments();
	const [enabled, setEnabled] = useState(value.paymentsEnabled);
	const [refunds, setRefunds] = useState(value.refundsFromFlows);
	const [maximum, setMaximum] = useState(amountInput(value.maxPaymentAmount));
	const [daily, setDaily] = useState(amountInput(value.refundsDailyCap));
	const [busy, setBusy] = useState(false);
	const [saved, setSaved] = useState(false);
	const [error, setError] = useState<unknown>();
	const maxMinor = parseEuroAmount(maximum);
	const dailyMinor = parseEuroAmount(daily);
	const valid =
		maxMinor !== null &&
		dailyMinor !== null &&
		maxMinor <= value.platformMaxPaymentAmount &&
		(!enabled || maxMinor >= 50) &&
		(!refunds || dailyMinor >= 50);
	return (
		<form
			className="space-y-5"
			onSubmit={async (event) => {
				event.preventDefault();
				if (!valid) return;
				setBusy(true);
				setSaved(false);
				setError(undefined);
				try {
					await payments.request(
						`apps/${encodeURIComponent(appId)}/payments/settings`,
						"PATCH",
						{
							paymentsEnabled: enabled,
							refundsFromFlows: refunds,
							maxPaymentAmount: maxMinor,
							refundsDailyCap: dailyMinor,
						},
					);
					setSaved(true);
					await payments.refresh();
				} catch (error) {
					setError(error);
				} finally {
					setBusy(false);
				}
			}}
		>
			<PaymentError error={error} />
			<label
				htmlFor={`${appId}-enable`}
				className="flex items-center justify-between gap-4 text-sm"
			>
				<span>
					{t(
						"allowFlowPayments",
						"Allow this app to collect payments from flows",
					)}
				</span>
				<Switch
					id={`${appId}-enable`}
					checked={enabled}
					disabled={
						value.adminBlocked || !payments.config?.node_payments_enabled
					}
					onCheckedChange={setEnabled}
				/>
			</label>
			{value.adminBlocked && (
				<output className="text-sm">
					{t(
						"adminPaused",
						"An administrator has paused payments for this app.",
					)}
				</output>
			)}
			<label htmlFor={`${appId}-maximum`} className="grid gap-2 text-sm">
				{t("maximumPayment", "Maximum amount per payment (EUR)")}
				<Input
					id={`${appId}-maximum`}
					inputMode="decimal"
					value={maximum}
					onChange={(event) => setMaximum(event.target.value)}
					aria-invalid={
						maxMinor === null || maxMinor > value.platformMaxPaymentAmount
					}
				/>
				<span className="text-muted-foreground">
					{t("platformMaximum", "Platform maximum: {{amount}}", {
						amount: paymentMoney(
							value.platformMaxPaymentAmount,
							value.currency,
						),
					})}
				</span>
			</label>
			<label
				htmlFor={`${appId}-refunds`}
				className="flex items-center justify-between gap-4 text-sm"
			>
				<span>{t("allowFlowRefunds", "Allow flows to request refunds")}</span>
				<Switch
					id={`${appId}-refunds`}
					checked={refunds}
					onCheckedChange={setRefunds}
				/>
			</label>
			<label htmlFor={`${appId}-daily`} className="grid gap-2 text-sm">
				{t("dailyRefundLimit", "Daily refund limit (EUR)")}
				<Input
					id={`${appId}-daily`}
					inputMode="decimal"
					value={daily}
					onChange={(event) => setDaily(event.target.value)}
					aria-invalid={dailyMinor === null}
				/>
			</label>
			<Button type="submit" disabled={busy || !valid}>
				{t("saveSettings", "Save payment settings")}
			</Button>
			{saved && (
				<output className="text-sm">
					{t("settingsSaved", "Payment settings saved.")}
				</output>
			)}
		</form>
	);
}

export function AppPaymentsPage() {
	const { t } = useTranslation("payments");
	const appId = useSearchParams().get("id") ?? "";
	const permissions = useAppPermissions(appId);
	const canManage = permissions.can(RolePermissions.Owner);
	const payments = usePayments();
	const settings = usePaymentQuery<AppPaymentSettings>(
		`apps/${encodeURIComponent(appId)}/payments/settings`,
		!!appId && canManage,
	);
	const readiness = usePaymentQuery<PaymentReadiness>(
		`apps/${encodeURIComponent(appId)}/payments/readiness`,
		!!appId,
	);
	const [terms, setTerms] = useState<PaymentTerms | null>(null);
	const [busy, setBusy] = useState(false);
	const [accepted, setAccepted] = useState(false);
	const [error, setError] = useState<unknown>();
	return (
		<PaymentPage title={t("appPayments", "App payments")}>
			<p className="text-muted-foreground">
				{readiness.data?.platformOwned
					? t(
							"platformAppReceives",
							"This app's owner is a Flow-Like administrator. Marketplace and flow payments go to Flow-Like's Stripe account, with no transfer to a personal account.",
						)
					: t(
							"ownerReceives",
							"Payments go to the app owner's connected Stripe account. Transferring ownership pauses new payments until the new owner completes setup.",
						)}
			</p>
			<PaymentError error={settings.error || readiness.error || error} />
			{readiness.data && (
				<p className="rounded-lg border p-4 text-sm">
					{readiness.data.platformOwned
						? readiness.data.canAcceptPayments
							? t(
									"platformReadyToCollect",
									"Flow-Like can collect payments for this app.",
								)
							: t(
									"platformPaymentsUnavailable",
									"Payments are currently unavailable for this app.",
								)
						: readiness.data.canAcceptPayments
							? t("readyToCollect", "The owner's account can accept payments.")
							: t(
									"setupNeeded",
									"The owner must complete payment setup before this app can collect payments.",
								)}
				</p>
			)}
			{!canManage ? (
				<p>
					{t("ownerOnly", "Only the app owner can change payment settings.")}
				</p>
			) : (
				<>
					{readiness.data?.platformOwned === false && (
						<Link
							className="inline-block text-sm underline underline-offset-4"
							href="/account/payouts"
						>
							{t("manageAccount", "Manage your payment account")}
						</Link>
					)}
					{settings.data && (
						<Card>
							<CardHeader>
								<CardTitle>{t("flowPayments", "Flow payments")}</CardTitle>
							</CardHeader>
							<CardContent>
								<SettingsForm appId={appId} value={settings.data} />
							</CardContent>
						</Card>
					)}
					{payments.config?.marketplace_enabled &&
						readiness.data?.platformOwned === false && (
							<Card>
								<CardHeader>
									<CardTitle>
										{t("marketplaceSales", "Marketplace sales")}
									</CardTitle>
								</CardHeader>
								<CardContent className="space-y-4">
									<p className="text-sm text-muted-foreground">
										{t(
											"sellerTermsDescription",
											"Accept the current seller terms before offering this app for sale. Listing price and visibility remain in the app settings.",
										)}
									</p>
									<PaymentConsent
										kind="SELLER_TERMS"
										value={terms}
										onChange={setTerms}
									/>
									<Button
										disabled={!terms || busy}
										onClick={async () => {
											setBusy(true);
											setError(undefined);
											try {
												await payments.request(
													`apps/${encodeURIComponent(appId)}/marketplace/terms`,
													"POST",
													{
														termsVersion: terms?.version,
														termsAccepted: true,
														locale: terms?.locale,
													},
												);
												setAccepted(true);
												await payments.refresh();
											} catch (error) {
												setError(error);
											} finally {
												setBusy(false);
											}
										}}
									>
										{t("acceptSellerTerms", "Accept seller terms")}
									</Button>
									{accepted && (
										<output className="text-sm">
											{t("sellerTermsAccepted", "Seller terms accepted.")}
										</output>
									)}
								</CardContent>
							</Card>
						)}
				</>
			)}
		</PaymentPage>
	);
}
