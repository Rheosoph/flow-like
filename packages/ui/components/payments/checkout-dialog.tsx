"use client";

import { useTranslation } from "@flow-like/locales";
import Link from "next/link";
import { useEffect, useRef, useState } from "react";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import { PaymentConsent, PaymentError } from "./payment-parts";
import { PurchaseCard } from "./purchases-page";
import {
	type PaymentTerms,
	type PurchaseOrder,
	paymentMoney,
	pendingOrder,
} from "./types";
import {
	usePaymentDistribution,
	usePaymentQuery,
	usePayments,
} from "./use-payments";

export type MarketplaceItemKind = NonNullable<PurchaseOrder["itemKind"]>;

type MarketplaceCheckoutItem =
	| { itemKind?: "APP"; appId: string; appName: string }
	| { itemKind: "PACKAGE"; itemId: string; itemName: string };

type MarketplaceCheckoutDialogProps = MarketplaceCheckoutItem & {
	amount: number;
	open: boolean;
	onOpenChange: (open: boolean) => void;
	onPurchased: () => void | Promise<void>;
};

export function marketplaceCheckoutPath(
	itemKind: MarketplaceItemKind,
	itemId: string,
): string {
	const id = encodeURIComponent(itemId);
	return itemKind === "PACKAGE"
		? `registry/package/${id}/marketplace/checkout`
		: `apps/${id}/marketplace/checkout`;
}

function checkoutItem(item: MarketplaceCheckoutItem) {
	return item.itemKind === "PACKAGE"
		? { kind: item.itemKind, id: item.itemId, name: item.itemName }
		: { kind: "APP" as const, id: item.appId, name: item.appName };
}

function CheckoutDialogContent(props: MarketplaceCheckoutDialogProps) {
	const { amount, open, onOpenChange, onPurchased } = props;
	const item = checkoutItem(props);
	const { t, i18n } = useTranslation("payments");
	const payments = usePayments();
	const allowed = usePaymentDistribution();
	const [terms, setTerms] = useState<PaymentTerms | null>(null);
	const [created, setCreated] = useState<PurchaseOrder>();
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<unknown>();
	const [poll, setPoll] = useState(true);
	const order = usePaymentQuery<PurchaseOrder>(
		`user/purchases/${encodeURIComponent(created?.orderId ?? "")}`,
		open && !!created,
		poll,
	);
	const current = order.data ?? created;
	const notified = useRef<string | null>(null);
	useEffect(() => {
		setPoll(!current || pendingOrder(current.status));
		if (
			current &&
			["PAID", "COMPLETED", "FULFILLED"].includes(current.status) &&
			notified.current !== current.orderId
		) {
			notified.current = current.orderId;
			void onPurchased();
		}
	}, [current, onPurchased]);
	const create = async () => {
		if (!terms || !allowed) return;
		setBusy(true);
		setError(undefined);
		try {
			setCreated(
				await payments.request<PurchaseOrder>(
					marketplaceCheckoutPath(item.kind, item.id),
					"POST",
					{
						termsVersion: terms.version,
						termsAccepted: true,
						locale: terms.locale,
						withdrawalWaiver: false,
					},
				),
			);
		} catch (error) {
			setError(error);
		} finally {
			setBusy(false);
		}
	};
	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-xl">
				<DialogHeader>
					<DialogTitle>
						{t("buyApp", "Buy {{name}}", { name: item.name })}
					</DialogTitle>
					<DialogDescription>
						{item.kind === "PACKAGE"
							? t(
									"packageCheckoutDescription",
									"Review the purchase terms, then continue to Stripe. You can install the package once the server confirms payment.",
								)
							: t(
									"checkoutDescription",
									"Review the purchase terms, then continue to Stripe. Access is added after the server confirms payment.",
								)}
					</DialogDescription>
				</DialogHeader>
				<PaymentError error={error || order.error} />
				{current ? (
					<PurchaseCard order={current} />
				) : allowed && payments.config?.marketplace_enabled ? (
					<div className="space-y-5">
						<p className="text-xl font-semibold tabular-nums">
							{paymentMoney(amount, "eur", i18n.language)}
						</p>
						<PaymentConsent
							kind="PURCHASE_TERMS"
							value={terms}
							onChange={setTerms}
						/>
						<Button
							className="w-full"
							disabled={!terms || busy}
							onClick={() => void create()}
						>
							{busy
								? t("preparingCheckout", "Preparing checkout…")
								: t("prepareCheckout", "Prepare checkout")}
						</Button>
					</div>
				) : (
					<p>
						{t(
							"checkoutUnavailable",
							"Purchasing is unavailable in this app distribution.",
						)}
					</p>
				)}
				<Link
					className="text-sm underline underline-offset-4"
					href="/account/purchases"
				>
					{t("viewPurchases", "View your purchases")}
				</Link>
			</DialogContent>
		</Dialog>
	);
}

export function MarketplaceCheckoutDialog(
	props: MarketplaceCheckoutDialogProps,
) {
	const { identity } = usePayments();
	return <CheckoutDialogContent key={identity.join(":")} {...props} />;
}
