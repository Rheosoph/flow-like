"use client";

import { useTranslation } from "@flow-like/locales";
import Link from "next/link";
import { useState } from "react";
import { Button } from "../ui/button";
import { Card, CardContent } from "../ui/card";
import { PaymentError } from "./payment-parts";
import { paymentMoney, paymentUrl } from "./types";
import { usePaymentQuery, usePayments } from "./use-payments";

interface LegacyPurchase {
	id: string;
	itemId: string;
	itemKind: "APP" | "PACKAGE";
	itemName: string;
	status: string;
	amount: number;
	currency: string;
	createdAt: number;
	receiptAvailable: boolean;
}
export interface LegacyPurchasePage {
	items: LegacyPurchase[];
	nextCursor?: string | null;
}

function LegacyPurchaseCard({ purchase }: { purchase: LegacyPurchase }) {
	const { t, i18n } = useTranslation("payments");
	const payments = usePayments();
	const [busy, setBusy] = useState(false);
	const [loaded, setLoaded] = useState(false);
	const [url, setUrl] = useState<string>();
	const [error, setError] = useState<unknown>();
	return (
		<Card>
			<CardContent className="space-y-3 p-5">
				<div className="flex flex-wrap items-start justify-between gap-3">
					<div>
						<Link
							href={`${purchase.itemKind === "APP" ? "/store" : "/store/packages"}?id=${encodeURIComponent(purchase.itemId)}`}
							className="font-medium underline-offset-4 hover:underline"
						>
							{purchase.itemName}
						</Link>
						<p className="mt-1 text-xs text-muted-foreground">
							{t("legacyPurchase", "Legacy platform purchase")} ·{" "}
							{new Date(purchase.createdAt).toLocaleDateString(i18n.language)}
						</p>
					</div>
					<p className="font-medium tabular-nums">
						{paymentMoney(purchase.amount, purchase.currency, i18n.language)}
					</p>
				</div>
				<p className="text-sm">
					{String(
						t(`orderState.${purchase.status.toUpperCase()}`, {
							defaultValue: purchase.status.toLowerCase().replaceAll("_", " "),
						}),
					)}
				</p>
				<PaymentError error={error} />
				{url ? (
					<Button asChild variant="outline">
						<a href={url} target="_blank" rel="noopener noreferrer">
							{t("receipt", "View receipt")}
						</a>
					</Button>
				) : (
					purchase.receiptAvailable && (
						<Button
							variant="outline"
							disabled={busy || loaded}
							onClick={async () => {
								setBusy(true);
								setError(undefined);
								try {
									const receipt = await payments.request<{
										url: string | null;
									}>(
										`user/payments/legacy-purchases/${purchase.itemKind}/${encodeURIComponent(purchase.id)}/receipt`,
									);
									setUrl(paymentUrl(receipt.url));
									setLoaded(true);
								} catch (error) {
									setError(error);
								} finally {
									setBusy(false);
								}
							}}
						>
							{busy
								? t("loading", "Loading…")
								: t("loadReceipt", "Load receipt")}
						</Button>
					)
				)}
				{loaded && !url && (
					<output className="block text-sm text-muted-foreground">
						{t(
							"receiptUnavailable",
							"A Stripe receipt is not available for this purchase.",
						)}
					</output>
				)}
			</CardContent>
		</Card>
	);
}

export function LegacyPurchaseHistory({
	initial,
}: { initial?: LegacyPurchasePage }) {
	const { t } = useTranslation("payments");
	const [cursor, setCursor] = useState<string>();
	const [previous, setPrevious] = useState<LegacyPurchase[]>([]);
	const page = usePaymentQuery<LegacyPurchasePage>(
		`user/payments/legacy-purchases?cursor=${encodeURIComponent(cursor ?? "")}`,
		!!cursor,
	);
	const current = cursor ? page.data : initial;
	if (!initial?.items.length) return null;
	return (
		<section className="space-y-4">
			<h2 className="text-lg font-semibold">
				{t("legacyHistory", "Earlier purchases")}
			</h2>
			<p className="text-sm text-muted-foreground">
				{t(
					"legacyHistoryDescription",
					"These purchases used the earlier platform checkout. Their original records and available receipts remain accessible.",
				)}
			</p>
			<PaymentError error={page.error} />
			{[...previous, ...(current?.items ?? [])].map((purchase) => (
				<LegacyPurchaseCard
					key={`${purchase.itemKind}:${purchase.id}`}
					purchase={purchase}
				/>
			))}
			{current?.nextCursor && (
				<Button
					variant="outline"
					disabled={page.isFetching}
					onClick={() => {
						setPrevious((items) => [...items, ...current.items]);
						setCursor(current.nextCursor ?? undefined);
					}}
				>
					{t("loadMore", "Load more")}
				</Button>
			)}
			{page.isLoading && cursor && <output>{t("loading", "Loading…")}</output>}
		</section>
	);
}
