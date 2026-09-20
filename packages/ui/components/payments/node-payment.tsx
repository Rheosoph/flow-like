"use client";

import { useTranslation } from "@flow-like/locales";
import { useSearchParams } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { Button } from "../ui/button";
import { Checkbox } from "../ui/checkbox";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import { Textarea } from "../ui/textarea";
import {
	PAYMENT_PROMPT_EVENT,
	PAYMENT_SIMULATION_EVENT,
	type PaymentPromptRef,
	type PaymentSimulation,
	paymentPromptRef,
} from "./payment-events";
import { PaymentError, PaymentPage } from "./payment-parts";
import { paymentMoney, paymentUrl, pendingOrder } from "./types";
import {
	usePaymentDistribution,
	usePaymentQuery,
	usePayments,
} from "./use-payments";

interface NodePaymentView extends PaymentPromptRef {
	platformOwned: boolean;
	status: string;
	reason?: string | null;
	expiresAt: number;
	amountMinor: number;
	currency: string;
	productName: string;
	description: string;
	payeeUserId: string;
	checkoutUrl?: string | null;
	receiptUrl?: string | null;
	payeeDisplayName?: string | null;
	appName?: string | null;
	refundedAmount: number;
	pendingRefundAmount: number;
}

export function NodePaymentCard({
	reference,
}: { reference: PaymentPromptRef }) {
	const { t, i18n } = useTranslation("payments");
	const payments = usePayments();
	const allowed = usePaymentDistribution();
	const query = new URLSearchParams({
		appId: reference.appId,
		runId: reference.runId,
	}).toString();
	const path = `payments/${encodeURIComponent(reference.id)}`;
	const [poll, setPoll] = useState(true);
	const payment = usePaymentQuery<NodePaymentView>(
		`${path}?${query}`,
		true,
		poll,
	);
	const [confirmed, setConfirmed] = useState(false);
	const [busy, setBusy] = useState(false);
	const [report, setReport] = useState(false);
	const [reason, setReason] = useState("");
	const [reported, setReported] = useState(false);
	const [error, setError] = useState<unknown>();
	const [now, setNow] = useState(Date.now());
	const current = payment.data;
	useEffect(() => {
		setPoll(!current || pendingOrder(current.status));
	}, [current]);
	useEffect(() => {
		const timer = window.setInterval(() => setNow(Date.now()), 1000);
		return () => window.clearInterval(timer);
	}, []);
	const expired = !!current && current.expiresAt <= now;
	const terminal = !!current && !pendingOrder(current.status);
	const act = async (action: "checkout" | "decline" | "report") => {
		if (action === "checkout" && (!confirmed || !allowed || expired)) return;
		setBusy(true);
		setError(undefined);
		try {
			await payments.request(
				`${path}/${action}?${query}`,
				"POST",
				action === "report" ? { reason: reason.trim() } : undefined,
			);
			if (action === "report") {
				setReported(true);
				setReport(false);
			}
			await payment.refetch();
		} catch (error) {
			setError(error);
		} finally {
			setBusy(false);
		}
	};
	return (
		<div className="space-y-5">
			<PaymentError error={error || payment.error} />
			{payment.isLoading && <output>{t("loading", "Loading…")}</output>}
			{current && (
				<>
					<div className="space-y-2">
						<h2 className="text-lg font-semibold">{current.productName}</h2>
						<p className="whitespace-pre-wrap text-sm text-muted-foreground">
							{current.description}
						</p>
						<p className="text-2xl font-semibold tabular-nums">
							{paymentMoney(
								current.amountMinor,
								current.currency,
								i18n.language,
							)}
						</p>
					</div>
					<dl className="grid gap-2 text-sm">
						<div>
							<dt className="text-muted-foreground">
								{t("paymentRecipient", "Payment recipient")}
							</dt>
							<dd className="break-all">
								{current.platformOwned
									? t("flowLike", "Flow-Like")
									: current.payeeDisplayName || current.payeeUserId}
							</dd>
						</div>
						<div>
							<dt className="text-muted-foreground">{t("app", "App")}</dt>
							<dd>{current.appName || current.appId}</dd>
						</div>
					</dl>
					<output className="text-sm">
						{String(
							t(`orderState.${current.status}`, {
								defaultValue: current.status.replaceAll("_", " ").toLowerCase(),
							}),
						)}
					</output>
					{!terminal && (
						<p className="text-sm text-muted-foreground">
							{expired
								? t(
										"requestExpired",
										"This payment request has expired. Any payment already processing is being reconciled.",
									)
								: t("requestTimeLeft", "Time remaining: {{seconds}} seconds", {
										seconds: Math.max(
											0,
											Math.floor((current.expiresAt - now) / 1000),
										),
									})}
						</p>
					)}
					{!terminal && !expired && allowed && (
						<>
							<label
								htmlFor={`agree-${reference.id}`}
								className="flex items-start gap-3 text-sm"
							>
								<Checkbox
									id={`agree-${reference.id}`}
									checked={confirmed}
									onCheckedChange={(value) => setConfirmed(value === true)}
								/>
								<span>
									{current.platformOwned
										? t(
												"confirmPlatformNodePayment",
												"I agree to pay the amount above to Flow-Like for the item shown.",
											)
										: t(
												"confirmNodePayment",
												"I agree to pay the amount above to this app's owner for the item shown.",
											)}
								</span>
							</label>
							{paymentUrl(current.checkoutUrl) ? (
								<Button asChild disabled={!confirmed}>
									<a
										href={
											confirmed ? paymentUrl(current.checkoutUrl) : undefined
										}
										target="_blank"
										rel="noopener noreferrer"
										aria-disabled={!confirmed}
										onClick={(event) => {
											if (!confirmed) event.preventDefault();
										}}
									>
										{t("continueCheckout", "Continue checkout")}
									</a>
								</Button>
							) : (
								<Button
									disabled={!confirmed || busy}
									onClick={() => void act("checkout")}
								>
									{t("prepareCheckout", "Prepare checkout")}
								</Button>
							)}
						</>
					)}
					{!allowed && !terminal && (
						<p className="text-sm">
							{t(
								"checkoutUnavailable",
								"Purchasing is unavailable in this app distribution.",
							)}
						</p>
					)}
					{current.refundedAmount > 0 && (
						<p className="text-sm">
							{t("refundedAmount", "Refunded: {{amount}}", {
								amount: paymentMoney(
									current.refundedAmount,
									current.currency,
									i18n.language,
								),
							})}
						</p>
					)}
					<div className="flex flex-wrap gap-3">
						{paymentUrl(current.receiptUrl) && (
							<Button asChild variant="outline">
								<a
									href={paymentUrl(current.receiptUrl)}
									target="_blank"
									rel="noopener noreferrer"
								>
									{t("receipt", "View receipt")}
								</a>
							</Button>
						)}
						{!terminal && (
							<Button
								variant="outline"
								disabled={busy || current.status === "CANCEL_PENDING"}
								onClick={() => void act("decline")}
							>
								{t("declinePayment", "Decline payment")}
							</Button>
						)}
						<Button
							variant="ghost"
							disabled={busy || reported}
							onClick={() => setReport(!report)}
						>
							{t("reportPayment", "Report this request")}
						</Button>
						<Button
							variant="ghost"
							disabled={payment.isFetching}
							onClick={() => void payment.refetch()}
						>
							{t("refreshStatus", "Refresh status")}
						</Button>
					</div>
					{report && (
						<div className="space-y-3">
							<label
								htmlFor={`report-${reference.id}`}
								className="grid gap-2 text-sm"
							>
								{t("reportReason", "Describe the issue")}
								<Textarea
									id={`report-${reference.id}`}
									value={reason}
									maxLength={1000}
									onChange={(event) => setReason(event.target.value)}
								/>
							</label>
							<p className="text-sm text-muted-foreground">
								{t(
									"reportEffect",
									"Reporting also cancels this payment request.",
								)}
							</p>
							<Button
								variant="destructive"
								disabled={busy || !reason.trim()}
								onClick={() => void act("report")}
							>
								{t("sendReport", "Send report and cancel")}
							</Button>
						</div>
					)}
					{reported && (
						<output className="text-sm">
							{t("reportSent", "Your report was sent.")}
						</output>
					)}
				</>
			)}
		</div>
	);
}

export function NodePaymentPrompt() {
	const { t } = useTranslation("payments");
	const [queue, setQueue] = useState<PaymentPromptRef[]>([]);
	const seen = useRef(new Set<string>());
	const [simulation, setSimulation] = useState<PaymentSimulation | null>(null);
	useEffect(() => {
		const receive = (event: Event) => {
			const reference = paymentPromptRef((event as CustomEvent).detail);
			if (!reference || seen.current.has(reference.id)) return;
			seen.current.add(reference.id);
			setQueue((items) => [...items, reference].slice(0, 16));
		};
		const simulate = (event: Event) =>
			setSimulation((event as CustomEvent<PaymentSimulation>).detail);
		window.addEventListener(PAYMENT_PROMPT_EVENT, receive);
		window.addEventListener(PAYMENT_SIMULATION_EVENT, simulate);
		return () => {
			window.removeEventListener(PAYMENT_PROMPT_EVENT, receive);
			window.removeEventListener(PAYMENT_SIMULATION_EVENT, simulate);
		};
	}, []);
	const current = queue[0];
	return (
		<Dialog
			open={!!current || !!simulation}
			onOpenChange={(open) => {
				if (!open) {
					if (current) setQueue((items) => items.slice(1));
					else setSimulation(null);
				}
			}}
		>
			<DialogContent className="max-h-[90vh] overflow-y-auto">
				<DialogHeader>
					<DialogTitle>
						{t("paymentRequested", "Payment requested")}
					</DialogTitle>
					<DialogDescription>
						{t(
							"paymentRequestDescription",
							"The running flow is waiting for your decision. Review the recipient and amount before continuing.",
						)}
					</DialogDescription>
				</DialogHeader>
				{current && <NodePaymentCard key={current.id} reference={current} />}
				{!current && simulation && (
					<div className="space-y-3 rounded-lg border p-4">
						<h2 className="font-semibold">
							{t("simulationTitle", "Payment simulation")}
						</h2>
						<p className="text-sm">
							{t(
								"simulationDescription",
								"This local Board Test uses a simulated result. No payment was collected.",
							)}
						</p>
						<p>{simulation.productName}</p>
						<p>{paymentMoney(simulation.amountMinor, simulation.currency)}</p>
						<p className="text-sm">
							{t("simulationResult", "Simulated outcome: {{status}}", {
								status: simulation.status,
							})}
						</p>
					</div>
				)}
			</DialogContent>
		</Dialog>
	);
}

export function NodePaymentPage() {
	const { t } = useTranslation("payments");
	const params = useSearchParams();
	const reference = paymentPromptRef({
		id: params.get("id"),
		appId: params.get("appId"),
		runId: params.get("runId"),
	});
	return (
		<PaymentPage title={t("paymentRequested", "Payment requested")}>
			{reference ? (
				<NodePaymentCard key={reference.id} reference={reference} />
			) : (
				<p>{t("invalidPaymentLink", "This payment link is incomplete.")}</p>
			)}
		</PaymentPage>
	);
}
