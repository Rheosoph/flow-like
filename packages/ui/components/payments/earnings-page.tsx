"use client";

import { useTranslation } from "@flow-like/locales";
import { useRef, useState } from "react";
import { Button } from "../ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "../ui/card";
import { Input } from "../ui/input";
import { PaymentError, PaymentPage } from "./payment-parts";
import {
	type PurchaseOrder,
	amountInput,
	parseEuroAmount,
	paymentMoney,
} from "./types";
import { usePaymentQuery, usePayments } from "./use-payments";

interface EarningsEntry {
	platformOwned: boolean;
	id: string;
	sourceType: string;
	sourceId: string;
	status: string;
	amount: number;
	currency: string;
	capturedAmount: number;
	refundedAmount: number;
	reservedRefundAmount: number;
	applicationFeeAmount: number;
	settlement: string;
	orphaned: boolean;
	createdAt: number;
}
interface StripeBalance {
	platformOwned: boolean;
	available: { amount: number; currency: string }[];
	pending: { amount: number; currency: string }[];
	payoutsEnabled: boolean | null;
}

function SaleRefund({ order }: { order: PurchaseOrder }) {
	const { t } = useTranslation("payments");
	const payments = usePayments();
	const [open, setOpen] = useState(false);
	const [amount, setAmount] = useState(
		amountInput(order.refundableRemaining ?? 0),
	);
	const [confirm, setConfirm] = useState(false);
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<unknown>();
	const [submitted, setSubmitted] = useState(false);
	const command = useRef<string | null>(null);
	const minor = parseEuroAmount(amount);
	const valid =
		minor !== null && minor > 0 && minor <= (order.refundableRemaining ?? 0);
	return (
		<div className="space-y-3 border-t pt-3">
			<div className="flex flex-wrap justify-between gap-3">
				<div>
					<p className="text-sm font-medium">{order.itemName ?? order.appId}</p>
					<p className="text-xs text-muted-foreground">{order.orderId}</p>
				</div>
				<p className="text-sm">{paymentMoney(order.amount, order.currency)}</p>
			</div>
			{!open && (order.refundableRemaining ?? 0) > 0 && (
				<Button variant="outline" size="sm" onClick={() => setOpen(true)}>
					{t("refundSale", "Refund sale")}
				</Button>
			)}
			{open && (
				<div className="space-y-3">
					<PaymentError error={error} />
					<label
						htmlFor={`refund-${order.orderId}`}
						className="grid gap-2 text-sm"
					>
						{t("refundAmount", "Refund amount (EUR)")}
						<Input
							id={`refund-${order.orderId}`}
							inputMode="decimal"
							value={amount}
							disabled={busy || command.current !== null}
							onChange={(event) => {
								setAmount(event.target.value);
								setConfirm(false);
							}}
						/>
					</label>
					<p className="text-xs text-muted-foreground">
						{t("refundableAmount", "Available to refund: {{amount}}", {
							amount: paymentMoney(
								order.refundableRemaining ?? 0,
								order.currency,
							),
						})}
					</p>
					{confirm && (
						<p className="text-sm">
							{t(
								"sellerRefundConfirmation",
								"Confirm a refund of {{amount}}? This cannot be undone.",
								{ amount: paymentMoney(minor ?? 0, order.currency) },
							)}
						</p>
					)}
					{submitted ? (
						<output className="text-sm">
							{t(
								"refundSubmitted",
								"Refund requested. Its status will update after Stripe processes it.",
							)}
						</output>
					) : (
						<Button
							variant={confirm ? "destructive" : "outline"}
							disabled={!valid || busy}
							onClick={async () => {
								if (!confirm) {
									setConfirm(true);
									return;
								}
								setBusy(true);
								setError(undefined);
								command.current ??= crypto.randomUUID();
								try {
									await payments.request(
										`user/sales/${encodeURIComponent(order.orderId)}/refund`,
										"POST",
										{
											commandId: command.current,
											amount: minor,
											reason: "requested_by_customer",
										},
									);
									setSubmitted(true);
									await payments.refresh();
								} catch (error) {
									setError(error);
								} finally {
									setBusy(false);
								}
							}}
						>
							{confirm
								? t("confirmRefund", "Confirm refund")
								: t("reviewRefund", "Review refund")}
						</Button>
					)}
				</div>
			)}
		</div>
	);
}

export function EarningsPage() {
	const { identity } = usePayments();
	return <EarningsPageContent key={identity.join(":")} />;
}

function EarningsPageContent() {
	const { t, i18n } = useTranslation("payments");
	const balance = usePaymentQuery<StripeBalance>("user/payments/balance");
	const [before, setBefore] = useState<number>();
	const [previous, setPrevious] = useState<EarningsEntry[]>([]);
	const earnings = usePaymentQuery<{
		platformOwned: boolean;
		entries: EarningsEntry[];
		nextBefore?: number;
	}>(`user/payments/earnings${before ? `?before=${before}` : ""}`);
	const sales = usePaymentQuery<{ orders: PurchaseOrder[] }>("user/sales");
	return (
		<PaymentPage title={t("earnings", "Earnings and refunds")}>
			<p className="text-muted-foreground">
				{earnings.data?.platformOwned
					? t(
							"platformEarningsDescription",
							"Review payments collected by Flow-Like and manage marketplace refunds. Each payment retains its original recipient; any earlier connected account payments are labeled separately.",
						)
					: t(
							"earningsDescription",
							"Review payments credited to your account and manage refunds for your marketplace sales.",
						)}
			</p>
			<PaymentError error={earnings.error || sales.error} />
			<Card>
				<CardHeader>
					<CardTitle>
						{balance.data?.platformOwned
							? t("platformBalance", "Flow-Like platform balance")
							: t("stripeBalance", "Stripe account balance")}
					</CardTitle>
				</CardHeader>
				<CardContent className="space-y-3">
					<p className="text-sm text-muted-foreground">
						{balance.data?.platformOwned
							? t(
									"platformBalanceScope",
									"This is Flow-Like's entire Stripe balance, including other apps and billing activity. It is not a personal payout balance.",
								)
							: t(
									"balanceScope",
									"This balance belongs to your Stripe account and can include activity outside FlowLike.",
								)}
					</p>
					<PaymentError error={balance.error} />
					{balance.data && (
						<div className="grid gap-4 sm:grid-cols-2">
							{(["available", "pending"] as const).map((kind) => (
								<div key={kind}>
									<p className="text-sm text-muted-foreground">
										{kind === "available"
											? t("availableBalance", "Available")
											: t("pendingBalance", "Pending")}
									</p>
									{balance.data?.[kind]?.map((item) => (
										<p
											key={item.currency}
											className="text-xl font-semibold tabular-nums"
										>
											{paymentMoney(item.amount, item.currency, i18n.language)}
										</p>
									))}
								</div>
							))}
						</div>
					)}
				</CardContent>
			</Card>
			<Card>
				<CardHeader>
					<CardTitle>{t("paymentActivity", "Payment activity")}</CardTitle>
				</CardHeader>
				<CardContent className="space-y-4">
					{[...previous, ...(earnings.data?.entries ?? [])].map((entry) => (
						<div
							key={entry.id}
							className="flex flex-wrap justify-between gap-3 border-b pb-3 text-sm"
						>
							<div>
								<p className="font-medium">
									{entry.sourceType === "REQUEST"
										? t("flowPayment", "Flow payment")
										: t("marketplaceSale", "Marketplace sale")}
								</p>
								<p className="text-xs text-muted-foreground">
									{new Date(entry.createdAt).toLocaleString(i18n.language)}
								</p>
								<p>{entry.status.toLowerCase().replaceAll("_", " ")}</p>
								<p className="text-xs text-muted-foreground">
									{entry.platformOwned
										? t("platformRecipient", "Recipient: Flow-Like")
										: t(
												"connectedRecipient",
												"Recipient: your connected Stripe account",
											)}
								</p>
							</div>
							<div className="text-right">
								<p>
									{paymentMoney(
										entry.capturedAmount || entry.amount,
										entry.currency,
										i18n.language,
									)}
								</p>
								<p className="text-xs text-muted-foreground">
									{t("platformFee", "Platform fee: {{amount}}", {
										amount: paymentMoney(
											entry.applicationFeeAmount,
											entry.currency,
											i18n.language,
										),
									})}
								</p>
								{entry.refundedAmount > 0 && (
									<p className="text-xs">
										{t("refundedAmount", "Refunded: {{amount}}", {
											amount: paymentMoney(
												entry.refundedAmount,
												entry.currency,
												i18n.language,
											),
										})}
									</p>
								)}
							</div>
						</div>
					))}
					{earnings.data?.entries?.length === 0 && previous.length === 0 && (
						<p className="text-sm text-muted-foreground">
							{t("noEarnings", "No payments yet.")}
						</p>
					)}
					{earnings.data?.nextBefore && earnings.data.entries?.length >= 50 && (
						<Button
							variant="outline"
							disabled={earnings.isFetching}
							onClick={() => {
								const page = earnings.data;
								if (!page) return;
								setPrevious((items) => [...items, ...page.entries]);
								setBefore(page.nextBefore);
							}}
						>
							{t("loadMore", "Load more")}
						</Button>
					)}
				</CardContent>
			</Card>
			<Card>
				<CardHeader>
					<CardTitle>{t("saleRefunds", "Marketplace refunds")}</CardTitle>
				</CardHeader>
				<CardContent className="space-y-4">
					{sales.data?.orders?.map((order) => (
						<div key={order.orderId} className="space-y-2">
							<p className="text-xs text-muted-foreground">
								{order.platformOwned
									? t("platformRecipient", "Recipient: Flow-Like")
									: t(
											"connectedRecipient",
											"Recipient: your connected Stripe account",
										)}
							</p>
							<SaleRefund order={order} />
						</div>
					))}
					{sales.data?.orders?.length === 0 && (
						<p className="text-sm text-muted-foreground">
							{t("noSales", "No marketplace sales yet.")}
						</p>
					)}
				</CardContent>
			</Card>
		</PaymentPage>
	);
}
