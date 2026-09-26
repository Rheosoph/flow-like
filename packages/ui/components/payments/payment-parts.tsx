"use client";

import { useTranslation } from "@flow-like/locales";
import type { ReactNode } from "react";
import { useEffect, useId } from "react";
import { ApiResponseError, apiErrorMessage } from "../../lib/api-error";
import { Button } from "../ui/button";
import { Checkbox } from "../ui/checkbox";
import type { PaymentTerms } from "./types";
import { usePaymentQuery, usePayments } from "./use-payments";

export function PaymentPage({
	title,
	children,
}: { title: string; children: ReactNode }) {
	return (
		<main className="min-h-0 flex-1 overflow-auto">
			<div className="mx-auto w-full max-w-4xl space-y-6 p-6 md:p-10">
				<h1 className="text-2xl font-semibold tracking-tight">{title}</h1>
				{children}
			</div>
		</main>
	);
}

export function PaymentError({ error }: { error: unknown }) {
	const { t } = useTranslation("payments");
	const { auth } = usePayments();
	if (!error) return null;
	const reauth =
		error instanceof ApiResponseError && error.code === "REAUTH_REQUIRED";
	return (
		<div
			role="alert"
			className="rounded-lg border border-destructive/30 bg-destructive/5 p-4 text-sm"
		>
			<p>
				{apiErrorMessage(
					error,
					t("requestFailed", "The request could not be completed. Try again."),
				)}
			</p>
			{reauth && (
				<Button
					className="mt-3"
					onClick={() =>
						void auth.signinRedirect({
							prompt: "login",
							max_age: 0,
							url_state: window.location.pathname + window.location.search,
						})
					}
				>
					{t("signInAgain", "Sign in again")}
				</Button>
			)}
		</div>
	);
}

export function PaymentConsent({
	kind,
	value,
	onChange,
}: {
	kind: string;
	value: PaymentTerms | null;
	onChange: (terms: PaymentTerms | null) => void;
}) {
	const { t, i18n } = useTranslation("payments");
	const locale = i18n.resolvedLanguage ?? "en";
	const checkboxId = useId();
	const terms = usePaymentQuery<PaymentTerms>(
		`payments/terms?kind=${encodeURIComponent(kind)}&locale=${encodeURIComponent(locale)}`,
	);
	useEffect(() => {
		if (
			value &&
			(terms.data?.hash !== value.hash ||
				terms.data?.version !== value.version ||
				terms.data?.locale !== value.locale)
		)
			onChange(null);
	}, [terms.data, value, onChange]);
	return (
		<div className="space-y-3">
			<PaymentError error={terms.error} />
			{terms.isLoading && (
				<output className="text-sm text-muted-foreground">
					{t("loadingTerms", "Loading terms…")}
				</output>
			)}
			{terms.data && (
				<>
					<textarea
						className="max-h-64 w-full resize-y rounded-lg border bg-background p-4 text-sm"
						aria-label={t("agreementText", "Agreement text")}
						readOnly
						rows={8}
						value={terms.data.text}
					/>
					<label
						htmlFor={checkboxId}
						className="flex cursor-pointer items-start gap-3 text-sm"
					>
						<Checkbox
							id={checkboxId}
							checked={value?.hash === terms.data.hash}
							onCheckedChange={(checked) =>
								onChange(checked === true ? (terms.data ?? null) : null)
							}
						/>
						<span>
							{t("acceptTerms", "I have read and accept these terms.")}
						</span>
					</label>
				</>
			)}
		</div>
	);
}
