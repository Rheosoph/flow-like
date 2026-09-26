"use client";

import { useTranslation } from "@flow-like/locales";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { Button } from "../ui/button";
import { paymentPromptRef } from "./payment-events";
import { PaymentError, PaymentPage } from "./payment-parts";
import { paymentUrl } from "./types";
import { usePayments } from "./use-payments";

export function PaymentReturnPage() {
	const { t } = useTranslation("payments");
	const params = useSearchParams();
	const payments = usePayments();
	const request = paymentPromptRef({
		id: params.get("id"),
		appId: params.get("appId"),
		runId: params.get("runId"),
	});
	const requestHref = request
		? `/payments/request?${new URLSearchParams({ id: request.id, appId: request.appId, runId: request.runId }).toString()}`
		: undefined;
	const resume = useRef<string | null>(null);
	const captured = useRef(false);
	const [canResume, setCanResume] = useState(false);
	const [busy, setBusy] = useState(false);
	const [link, setLink] = useState<string>();
	const [error, setError] = useState<unknown>();
	useEffect(() => {
		if (captured.current) return;
		captured.current = true;
		resume.current =
			params.get("connect") === "refresh" ? params.get("r") : null;
		setCanResume(!!resume.current);
		const url = new URL(window.location.href);
		url.searchParams.delete("r");
		window.history.replaceState(
			window.history.state,
			"",
			url.pathname + url.search,
		);
		void payments.refresh();
	}, [params, payments.refresh]);
	return (
		<PaymentPage title={t("returnTitle", "Continue in FlowLike")}>
			<p className="text-muted-foreground">
				{t(
					"returnDescription",
					"You can return to the app now. Your payment or account status is confirmed by the server and may still be processing.",
				)}
			</p>
			<PaymentError error={error} />
			{payments.auth.isAuthenticated ? (
				<div className="flex flex-wrap gap-3">
					{requestHref && (
						<Button asChild>
							<Link href={requestHref}>
								{t("viewPaymentRequest", "View payment request")}
							</Link>
						</Button>
					)}
					<Button asChild>
						<Link href="/account/purchases">
							{t("viewPurchases", "View your purchases")}
						</Link>
					</Button>
					<Button asChild variant="outline">
						<Link href="/account/payouts">
							{t("manageAccount", "Manage your payment account")}
						</Link>
					</Button>
					{canResume && (
						<Button
							disabled={busy}
							variant="outline"
							onClick={async () => {
								setBusy(true);
								setError(undefined);
								try {
									const response = await payments.request<{ url: string }>(
										"user/payments/connect/resume",
										"POST",
										{ resumeToken: resume.current },
									);
									setLink(paymentUrl(response.url));
									setCanResume(false);
									resume.current = null;
								} catch (error) {
									setError(error);
								} finally {
									setBusy(false);
								}
							}}
						>
							{t("resumeSetup", "Resume Stripe setup")}
						</Button>
					)}
				</div>
			) : (
				<div className="space-y-3">
					<p className="text-sm">
						{t(
							"returnSignIn",
							"Sign in to view your purchases or continue payment account setup.",
						)}
					</p>
					<Button
						onClick={() =>
							void payments.auth.signinRedirect({
								url_state:
									requestHref ??
									(params.get("connect")
										? "/account/payouts"
										: "/account/purchases"),
							})
						}
					>
						{t("signIn", "Sign in")}
					</Button>
				</div>
			)}
			{link && (
				<Button asChild>
					<a href={link} target="_blank" rel="noopener noreferrer">
						{t("continueStripe", "Continue in Stripe")}
					</a>
				</Button>
			)}
		</PaymentPage>
	);
}
