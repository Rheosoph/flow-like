"use client";

import { useTranslation } from "@flow-like/locales";
import Link from "next/link";
import { useEffect, useId, useState } from "react";
import { Button } from "../ui/button";
import { Card, CardContent } from "../ui/card";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import {
	LegacyPurchaseHistory,
	type LegacyPurchasePage,
} from "./legacy-purchases";
import { PaymentError, PaymentPage } from "./payment-parts";
import {
	type PurchaseOrder,
	type WithdrawalRequest,
	paymentMoney,
	paymentUrl,
	pendingOrder,
} from "./types";
import {
	usePaymentDistribution,
	usePaymentQuery,
	usePayments,
} from "./use-payments";

export function WithdrawalConfirmation({
	order,
	busy,
	onConfirm,
	onCancel,
}: {
	order: PurchaseOrder;
	busy: boolean;
	onConfirm: (body: WithdrawalRequest) => void;
	onCancel: () => void;
}) {
	const { t, i18n } = useTranslation("payments");
	const id = useId();
	const [consumerName, setConsumerName] = useState("");
	const [confirmationEmail, setConfirmationEmail] = useState("");
	return (
		<form
			className="space-y-3 rounded-lg border p-4"
			onSubmit={(event) => {
				event.preventDefault();
				if (!consumerName.trim() || !confirmationEmail.trim()) return;
				onConfirm({
					confirm: true,
					consumerName: consumerName.trim(),
					confirmationEmail: confirmationEmail.trim(),
				});
			}}
		>
			<p className="font-medium">
				{t("orderReference", "Order {{id}}", { id: order.orderId })}
			</p>
			<p className="text-sm">
				{t(
					"withdrawDeclaration",
					"I withdraw from the purchase identified above.",
				)}
			</p>
			<p className="text-sm text-muted-foreground">
				{t(
					"withdrawConfirmation",
					"Access from this purchase will end when the withdrawal is recorded. You do not need to give a reason.",
				)}
			</p>
			<div className="space-y-2">
				<Label htmlFor={`${id}-name`}>
					{t("withdrawConsumerName", "Your full name")}
				</Label>
				<Input
					id={`${id}-name`}
					name="consumerName"
					autoComplete="name"
					required
					maxLength={200}
					pattern={".*\\S.*"}
					value={consumerName}
					onChange={(event) => setConsumerName(event.target.value)}
					disabled={busy}
				/>
			</div>
			<div className="space-y-2">
				<Label htmlFor={`${id}-email`}>
					{t(
						"withdrawConfirmationEmail",
						"Email for your withdrawal confirmation",
					)}
				</Label>
				<Input
					id={`${id}-email`}
					name="confirmationEmail"
					type="email"
					autoComplete="email"
					required
					maxLength={254}
					aria-describedby={`${id}-email-help`}
					value={confirmationEmail}
					onChange={(event) => setConfirmationEmail(event.target.value)}
					disabled={busy}
				/>
				<p id={`${id}-email-help`} className="text-sm text-muted-foreground">
					{t(
						"withdrawConfirmationEmailHelp",
						"We will send your declaration and its receipt date and time to this address.",
					)}
				</p>
			</div>
			{order.withdrawDeadline && (
				<p className="text-sm text-muted-foreground">
					{t("withdrawDeadline", "Withdrawal deadline: {{date}}", {
						date: new Date(order.withdrawDeadline).toLocaleString(
							i18n.language,
						),
					})}
				</p>
			)}
			<div className="flex flex-wrap gap-3">
				<Button type="submit" variant="destructive" disabled={busy}>
					{t("confirmWithdrawal", "Confirm withdrawal")}
				</Button>
				<Button
					type="button"
					variant="outline"
					disabled={busy}
					onClick={onCancel}
				>
					{t("keepPurchase", "Keep purchase")}
				</Button>
			</div>
		</form>
	);
}

export function PurchaseCard({ order }: { order: PurchaseOrder }) {
	const { t, i18n } = useTranslation("payments");
	const payments = usePayments();
	const checkoutAllowed = usePaymentDistribution();
	const [confirm, setConfirm] = useState(false);
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<unknown>();
	const act = async (
		action: "cancel" | "withdraw",
		body?: WithdrawalRequest,
	) => {
		setBusy(true);
		setError(undefined);
		try {
			await payments.request(
				`user/purchases/${encodeURIComponent(order.orderId)}/${action}`,
				"POST",
				body,
			);
			setConfirm(false);
			await payments.refresh();
		} catch (error) {
			setError(error);
		} finally {
			setBusy(false);
		}
	};
	const checkoutUrl = checkoutAllowed
		? paymentUrl(order.checkoutUrl)
		: undefined;
	return (
		<Card>
			<CardContent className="space-y-4 p-5">
				<div className="flex flex-wrap items-start justify-between gap-3">
					<div>
						<Link
							href={`/store?id=${encodeURIComponent(order.appId)}`}
							className="font-medium underline-offset-4 hover:underline"
						>
							{order.itemName ?? order.appId}
						</Link>
						<p className="mt-1 text-xs text-muted-foreground">
							{t("orderReference", "Order {{id}}", { id: order.orderId })}
						</p>
					</div>
					<p className="font-medium tabular-nums">
						{paymentMoney(order.amount, order.currency, i18n.language)}
					</p>
				</div>
				<output className="text-sm">
					{String(
						t(`orderState.${order.status}`, {
							defaultValue: order.status.replaceAll("_", " ").toLowerCase(),
						}),
					)}
				</output>
				{order.refundedAmount > 0 && (
					<p className="text-sm">
						{t("refundedAmount", "Refunded: {{amount}}", {
							amount: paymentMoney(
								order.refundedAmount,
								order.currency,
								i18n.language,
							),
						})}
					</p>
				)}
				{order.pendingRefundAmount > 0 && (
					<p className="text-sm">
						{t("refundProcessing", "Refund processing: {{amount}}", {
							amount: paymentMoney(
								order.pendingRefundAmount,
								order.currency,
								i18n.language,
							),
						})}
					</p>
				)}
				<PaymentError error={error} />
				<div className="flex flex-wrap gap-3">
					{checkoutUrl && (
						<Button asChild>
							<a href={checkoutUrl} target="_blank" rel="noopener noreferrer">
								{t("continueCheckout", "Continue checkout")}
							</a>
						</Button>
					)}
					{paymentUrl(order.receiptUrl) && (
						<Button asChild variant="outline">
							<a
								href={paymentUrl(order.receiptUrl)}
								target="_blank"
								rel="noopener noreferrer"
							>
								{t("receipt", "View receipt")}
							</a>
						</Button>
					)}
					{pendingOrder(order.status) && (
						<Button
							disabled={busy || order.status === "CANCEL_PENDING"}
							variant="outline"
							onClick={() => void act("cancel")}
						>
							{t("cancelCheckout", "Cancel checkout")}
						</Button>
					)}
					{order.withdrawable && !confirm && (
						<Button disabled={busy} onClick={() => setConfirm(true)}>
							{t("withdrawPurchase", "Withdraw from contract")}
						</Button>
					)}
				</div>
				{confirm && order.withdrawable && (
					<WithdrawalConfirmation
						order={order}
						busy={busy}
						onConfirm={(body) => void act("withdraw", body)}
						onCancel={() => setConfirm(false)}
					/>
				)}
			</CardContent>
		</Card>
	);
}

export function PurchaseLookupResult({ orderId }: { orderId: string }) {
	const { t } = useTranslation("payments");
	const [poll, setPoll] = useState(false);
	const purchase = usePaymentQuery<PurchaseOrder>(
		`user/purchases/${encodeURIComponent(orderId)}`,
		true,
		poll,
	);
	const pending = purchase.data
		? pendingOrder(purchase.data.status) ||
			purchase.data.pendingRefundAmount > 0
		: false;
	useEffect(() => setPoll(pending), [pending]);
	return (
		<div className="space-y-3" aria-live="polite">
			<PaymentError error={purchase.error} />
			{purchase.isLoading && <output>{t("loading", "Loading…")}</output>}
			{purchase.data && (
				<PurchaseCard key={purchase.data.orderId} order={purchase.data} />
			)}
		</div>
	);
}

function PurchaseLookup() {
	const { t } = useTranslation("payments");
	const id = useId();
	const [reference, setReference] = useState("");
	const [selection, setSelection] = useState<{
		orderId: string;
		attempt: number;
	}>();
	return (
		<section
			className="space-y-3 rounded-lg border p-4"
			aria-labelledby={`${id}-title`}
		>
			<h2 id={`${id}-title`} className="font-medium">
				{t("findPurchase", "Find a purchase")}
			</h2>
			<p id={`${id}-help`} className="text-sm text-muted-foreground">
				{t(
					"findPurchaseHelp",
					"Enter the order reference from your confirmation email to open any purchase, including older orders.",
				)}
			</p>
			<form
				className="flex flex-wrap items-end gap-3"
				onSubmit={(event) => {
					event.preventDefault();
					const orderId = reference.trim();
					if (orderId)
						setSelection((previous) => ({
							orderId,
							attempt: (previous?.attempt ?? 0) + 1,
						}));
				}}
			>
				<div className="min-w-48 flex-1 space-y-2">
					<Label htmlFor={`${id}-reference`}>
						{t("purchaseOrderReference", "Order reference")}
					</Label>
					<Input
						id={`${id}-reference`}
						name="orderReference"
						required
						maxLength={200}
						aria-describedby={`${id}-help`}
						value={reference}
						onChange={(event) => setReference(event.target.value)}
					/>
				</div>
				<Button type="submit" variant="outline" disabled={!reference.trim()}>
					{t("openPurchase", "Open purchase")}
				</Button>
			</form>
			{selection && (
				<PurchaseLookupResult
					key={`${selection.orderId}:${selection.attempt}`}
					orderId={selection.orderId}
				/>
			)}
		</section>
	);
}

export function PurchasesPage() {
	const { t } = useTranslation("payments");
	const payments = usePayments();
	const [poll, setPoll] = useState(false);
	const purchases = usePaymentQuery<{
		orders: PurchaseOrder[];
		legacyPurchases?: LegacyPurchasePage;
	}>("user/purchases", true, poll);
	const pending =
		purchases.data?.orders.some(
			(order) => pendingOrder(order.status) || order.pendingRefundAmount > 0,
		) ?? false;
	useEffect(() => {
		setPoll(pending);
	}, [pending]);
	return (
		<PaymentPage title={t("purchases", "Purchases")}>
			<p className="text-muted-foreground">
				{t(
					"purchasesDescription",
					"Check payment status, open receipts, and manage eligible withdrawals. Payment confirmation may take a moment after checkout.",
				)}
			</p>
			<PurchaseLookup key={payments.identity.join(":")} />
			<PaymentError error={purchases.error} />
			<Button
				variant="outline"
				disabled={purchases.isFetching}
				onClick={() => void purchases.refetch()}
			>
				{t("refreshStatus", "Refresh status")}
			</Button>
			{purchases.isLoading && <output>{t("loading", "Loading…")}</output>}
			{purchases.data?.orders.length === 0 && (
				<p className="rounded-lg border p-6 text-sm text-muted-foreground">
					{t("noPurchases", "No marketplace purchases yet.")}
				</p>
			)}
			<Link
				href="/account/payouts"
				className="text-sm underline underline-offset-4"
			>
				{t("manageAccount", "Manage your payment account")}
			</Link>
			<div className="space-y-4">
				{purchases.data?.orders.map((order) => (
					<PurchaseCard key={order.orderId} order={order} />
				))}
			</div>
			<LegacyPurchaseHistory
				key={payments.identity.join(":")}
				initial={purchases.data?.legacyPurchases}
			/>
		</PaymentPage>
	);
}
