"use client";

import { ArrowUpRight } from "lucide-react";
import { useState } from "react";
import { useAuth } from "react-oidc-context";
import { useInvoke } from "../../../hooks/use-invoke";
import { isRecord } from "../../../lib/response-shape";
import { useBackend } from "../../../state/backend-state";
import { Button } from "../../ui/button";
import {
	Sheet,
	SheetContent,
	SheetDescription,
	SheetHeader,
	SheetTitle,
	SheetTrigger,
} from "../../ui/sheet";
import { usageFundingLabel, usageStatusLabel } from "./use-usage-names";

function money(value: number | null | undefined, currency: string) {
	return value === null || value === undefined
		? "Not yet available"
		: new Intl.NumberFormat(undefined, {
				style: "currency",
				currency,
				maximumFractionDigits: 6,
			}).format(value / 1_000_000);
}

function DetailRow({ label, value }: { label: string; value: string }) {
	return (
		<div className="flex items-start justify-between gap-5 py-2.5">
			<dt className="text-muted-foreground">{label}</dt>
			<dd className="text-right tabular-nums break-words">{value}</dd>
		</div>
	);
}

export function UsageOperationDetails({
	id,
	appName,
	modelName,
}: {
	id: string;
	appName?: string;
	modelName?: string;
}) {
	const backend = useBackend();
	const auth = useAuth();
	const [open, setOpen] = useState(false);
	const query = useInvoke(
		backend.userState.getQuotaOperationDetail,
		backend.userState,
		[id],
		open && !!auth.user?.profile.sub,
		[auth.user?.profile.sub],
	);
	const detail = isRecord(query.data) ? query.data : undefined;
	const byteMetered = detail?.meteringBasis === "input_bytes";
	return (
		<Sheet open={open} onOpenChange={setOpen}>
			<SheetTrigger asChild>
				<Button
					variant="ghost"
					size="sm"
					className="mt-1 h-auto px-0 py-1.5 text-xs text-muted-foreground hover:bg-transparent hover:text-foreground"
					aria-label={`Cost and token details${appName ? ` for ${appName}` : ""}`}
				>
					Cost and token details{" "}
					<ArrowUpRight className="size-3" aria-hidden="true" />
				</Button>
			</SheetTrigger>
			<SheetContent className="w-full sm:max-w-lg">
				<SheetHeader className="border-b px-6 pb-5 pt-6 pr-12">
					<SheetTitle className="text-xl">Operation details</SheetTitle>
					<SheetDescription className="min-w-0 [overflow-wrap:anywhere]">
						{appName
							? `${appName}${modelName ? ` · ${modelName}` : ""}`
							: "AI usage, token counts and provider costs for this operation."}
					</SheetDescription>
				</SheetHeader>
				<div className="min-h-0 flex-1 overflow-y-auto px-6 pb-6">
					{query.isLoading ? (
						<p className="py-4 text-sm text-muted-foreground" role="status">
							Loading details…
						</p>
					) : query.isError ? (
						<div className="space-y-3 py-4">
							<p className="text-sm text-muted-foreground">
								These details are temporarily unavailable.
							</p>
							<Button
								size="sm"
								variant="outline"
								onClick={() => query.refetch()}
							>
								Retry details
							</Button>
						</div>
					) : detail ? (
						<div className="space-y-6">
							<div className="flex flex-wrap items-center gap-2 text-xs">
								<span className="rounded-full bg-muted px-2.5 py-1">
									{usageFundingLabel(detail.fundingClass)}
								</span>
								<span className="rounded-full border px-2.5 py-1">
									{usageStatusLabel(detail.status)}
								</span>
							</div>
							{detail.fundingClass === "hosted" ? (
								<>
									<div className="rounded-xl border bg-muted/30 p-4">
										<h3 className="text-sm text-muted-foreground">
											Hosted AI allowance used
										</h3>
										<p className="mt-2 text-2xl font-semibold tracking-tight tabular-nums">
											{money(detail.costMicroEur, "EUR")}
										</p>
										<p className="mt-2 text-xs leading-relaxed text-muted-foreground">
											{detail.costMicroEur == null
												? "Awaiting final usage. Any pending reservation stays visible in your usage overview."
												: detail.estimated
													? "This includes estimates. The amount can change when final usage is confirmed."
													: "This amount counts toward your plan's hosted AI allowance."}
										</p>
									</div>
									<section aria-label="Token and input usage">
										<h3 className="font-medium">
											{byteMetered ? "Input usage" : "Token usage"}
										</h3>
										<dl className="mt-2 divide-y text-sm">
											{byteMetered && (
												<>
													<DetailRow
														label="Input bytes"
														value={
															detail.inputBytes?.toLocaleString() ??
															"Not available"
														}
													/>
													<DetailRow
														label="Reported words (estimate)"
														value={
															detail.providerReportedWords?.toLocaleString() ??
															"Not available"
														}
													/>
												</>
											)}
											{(!byteMetered || detail.inputTokens != null) && (
												<DetailRow
													label="Input tokens"
													value={
														detail.inputTokens?.toLocaleString() ??
														"Not available"
													}
												/>
											)}
											{(!byteMetered || detail.outputTokens != null) && (
												<DetailRow
													label="Output tokens"
													value={
														detail.outputTokens?.toLocaleString() ??
														"Not available"
													}
												/>
											)}
											<DetailRow
												label={
													detail.tokenCountEstimated
														? "Token upper bound (estimate)"
														: "Embedding tokens"
												}
												value={
													detail.embeddingTokens?.toLocaleString() ??
													"Not available"
												}
											/>
										</dl>
										{byteMetered && (
											<p className="mt-2 text-xs leading-relaxed text-muted-foreground">
												This embedding operation is metered by input bytes. The
												word count and token upper bound are estimates.
											</p>
										)}
									</section>
									<section aria-label="Cost breakdown">
										<h3 className="font-medium">
											Cost breakdown{" "}
											<span className="text-xs font-normal text-muted-foreground">
												USD
											</span>
										</h3>
										<dl className="mt-2 divide-y text-sm">
											<DetailRow
												label={
													byteMetered
														? "Internal inference estimate"
														: "Provider inference"
												}
												value={money(detail.providerCostMicroUsd, "USD")}
											/>
											<DetailRow
												label="Provider funding fee"
												value={money(detail.providerFundingCostMicroUsd, "USD")}
											/>
											<DetailRow
												label="AI serving estimate"
												value={money(detail.servingCostMicroUsd, "USD")}
											/>
										</dl>
										<p className="mt-2 text-xs leading-relaxed text-muted-foreground">
											{byteMetered
												? "Inference and serving costs are internal estimates."
												: detail.estimated
													? "Some costs are estimated or awaiting confirmation."
													: "Provider inference is reported; serving compute remains an estimate."}
										</p>
									</section>
								</>
							) : (
								<div className="rounded-xl border bg-muted/30 p-4">
									<h3 className="font-medium">No hosted AI allowance used</h3>
									<p className="mt-2 text-sm leading-relaxed text-muted-foreground">
										{detail.fundingClass === "local"
											? "Local execution and local embeddings are free."
											: detail.fundingClass === "byok" ||
													detail.fundingClass === "customer"
												? "Your model uses no Flow-Like hosted AI allowance. Your provider may charge separately. Cloud orchestration still uses cloud runtime."
												: "This operation uses no Flow-Like hosted AI allowance. Cloud execution uses your runtime allowance."}
									</p>
								</div>
							)}
							<details className="rounded-xl border px-4 py-3 text-xs text-muted-foreground">
								<summary className="cursor-pointer font-medium">
									References and rate information
								</summary>
								<div className="space-y-3 pt-3 break-words [overflow-wrap:anywhere]">
									<p>Operation ID: {id}</p>
									{detail.rateVersion && (
										<p>Rate version: {detail.rateVersion}</p>
									)}
									{detail.usdMicroPerEur != null && (
										<p>
											Conversion: EUR 1 = USD{" "}
											{detail.usdMicroPerEur / 1_000_000}.
										</p>
									)}
									{detail.providerRequestId && (
										<p>Provider reference: {detail.providerRequestId}</p>
									)}
								</div>
							</details>
						</div>
					) : (
						<p className="py-4 text-sm text-muted-foreground">
							No operation details are available yet.
						</p>
					)}
				</div>
			</SheetContent>
		</Sheet>
	);
}
