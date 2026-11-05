"use client";

import { useTranslation } from "@flow-like/locales";
import { useId, useState } from "react";
import { apiErrorMessage } from "../../lib/api-error";
import {
	type MonthlyUsageLimits,
	type UsageBudgetView,
	formatMonthlyLimit,
	parseMonthlyLimit,
} from "../../lib/monthly-usage";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Label } from "../ui/label";

const FIELDS = [
	{ key: "aiCostMicros", scale: 1_000_000, label: "AI spend (EUR)" },
	{ key: "aiCalls", scale: 1, label: "AI calls" },
	{ key: "runtimeMs", scale: 1_000, label: "Cloud runtime (seconds)" },
] as const;

export function formatBudgetAmount(
	value: number,
	resource: keyof MonthlyUsageLimits,
): string {
	if (resource === "aiCostMicros") {
		return new Intl.NumberFormat(undefined, {
			style: "currency",
			currency: "EUR",
			maximumFractionDigits: 6,
		}).format(value / 1_000_000);
	}
	return new Intl.NumberFormat(undefined, {
		maximumFractionDigits: resource === "runtimeMs" ? 3 : 0,
	}).format(resource === "runtimeMs" ? value / 1_000 : value);
}

export function MonthlyUsageLimitsEditor({
	initial,
	onSave,
	ceiling,
	inherit = true,
	label,
}: Readonly<{
	initial: MonthlyUsageLimits;
	onSave: (limits: MonthlyUsageLimits) => Promise<unknown>;
	ceiling?: MonthlyUsageLimits;
	inherit?: boolean;
	label?: string;
}>) {
	const { t } = useTranslation("admin");
	const id = useId();
	const [draft, setDraft] = useState(() =>
		Object.fromEntries(
			FIELDS.map(({ key, scale }) => [
				key,
				formatMonthlyLimit(initial[key], scale),
			]),
		),
	);
	const [pending, setPending] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [saved, setSaved] = useState(false);
	return (
		<form
			className="space-y-3"
			onSubmit={async (event) => {
				event.preventDefault();
				if (pending) return;
				setError(null);
				setSaved(false);
				setPending(true);
				try {
					const limits: MonthlyUsageLimits = {
						aiCostMicros: null,
						aiCalls: null,
						runtimeMs: null,
					};
					for (const { key, scale, label } of FIELDS) {
						const value = parseMonthlyLimit(draft[key] ?? "", scale);
						const max = ceiling?.[key];
						if (value !== null && max != null && value > max) {
							throw new Error(
								t(
									"monthlyUsage.aboveCeiling",
									"{{field}} exceeds the enterprise ceiling. An enterprise administrator must approve a higher limit.",
									{ field: label },
								),
							);
						}
						limits[key] = value;
					}
					await onSave(limits);
					setSaved(true);
				} catch (failure) {
					setError(
						apiErrorMessage(
							failure,
							failure instanceof Error
								? failure.message
								: t(
										"monthlyUsage.saveFailed",
										"The usage limits could not be saved.",
									),
						),
					);
				} finally {
					setPending(false);
				}
			}}
		>
			<fieldset disabled={pending} className="space-y-3">
				{label && <legend className="mb-3 text-sm font-medium">{label}</legend>}
				<div className="grid gap-3 sm:grid-cols-3">
					{FIELDS.map(({ key, label: fieldLabel }) => (
						<div key={key} className="space-y-1.5">
							<Label htmlFor={`${id}-${key}`}>
								{t(`monthlyUsage.${key}`, fieldLabel)}
							</Label>
							<Input
								id={`${id}-${key}`}
								inputMode={key === "aiCalls" ? "numeric" : "decimal"}
								value={draft[key]}
								placeholder={
									inherit
										? t("monthlyUsage.inherit", "Inherit")
										: t("monthlyUsage.noCap", "No cap")
								}
								onChange={(event) => {
									setDraft((current) => ({
										...current,
										[key]: event.target.value,
									}));
									setSaved(false);
								}}
							/>
						</div>
					))}
				</div>
				<p className="text-xs text-muted-foreground">
					{inherit
						? t(
								"monthlyUsage.inheritHint",
								"Leave a field blank to inherit its ceiling. Zero stops new usage for that resource.",
							)
						: t(
								"monthlyUsage.noCapHint",
								"Leave a field blank for no enterprise cap on that resource. Zero stops new usage.",
							)}
				</p>
				<Button type="submit" size="sm">
					{pending
						? t("monthlyUsage.saving", "Saving…")
						: t("monthlyUsage.save", "Save monthly limits")}
				</Button>
			</fieldset>
			{error && (
				<p role="alert" className="text-sm text-destructive">
					{error}
				</p>
			)}
			{saved && (
				<output className="text-sm text-muted-foreground">
					{t("monthlyUsage.saved", "Monthly limits saved.")}
				</output>
			)}
		</form>
	);
}

export function UsageBudgetSummary({
	view,
}: Readonly<{ view: UsageBudgetView }>) {
	const { t } = useTranslation("admin");
	const ceiling = view.adminLimits ?? view.defaultLimits;
	return (
		<div className="space-y-3">
			<p className="text-xs text-muted-foreground">
				{t(
					"monthlyUsage.period",
					"Calendar month in UTC: {{start}} to {{end}} (exclusive).",
					{
						start: view.periodStart.slice(0, 10),
						end: view.periodEnd.slice(0, 10),
					},
				)}
			</p>
			<p className="text-xs text-muted-foreground">
				{t(
					"monthlyUsage.hostedOnly",
					"Hosted AI usage excludes local models and calls made directly with your own provider keys.",
				)}
			</p>
			<div className="grid gap-3 sm:grid-cols-3">
				{FIELDS.map(({ key, label }) => (
					<div key={key} className="rounded-lg border p-3">
						<p className="text-xs text-muted-foreground">
							{t(`monthlyUsage.${key}`, label)}
						</p>
						<p className="mt-1 text-lg font-semibold tabular-nums">
							{formatBudgetAmount(view.used[key], key)}
							<span className="text-xs font-normal text-muted-foreground">
								{" / "}
								{view.effectiveLimits[key] === null
									? t("monthlyUsage.noCap", "No cap")
									: formatBudgetAmount(view.effectiveLimits[key], key)}
							</span>
						</p>
						<p className="text-xs text-muted-foreground">
							{t("monthlyUsage.reserved", "{{amount}} reserved", {
								amount: formatBudgetAmount(view.reserved[key], key),
							})}
						</p>
						<p className="mt-2 text-xs text-muted-foreground">
							{view.adminLimits
								? t(
										"monthlyUsage.approvedCeiling",
										"Approved ceiling: {{amount}}",
										{
											amount:
												ceiling[key] === null
													? t("monthlyUsage.noCap", "No cap")
													: formatBudgetAmount(ceiling[key], key),
										},
									)
								: t(
										view.scope === "bot_group"
											? "monthlyUsage.inheritedBotCeiling"
											: "monthlyUsage.defaultCeiling",
										view.scope === "bot_group"
											? "Inherited bot ceiling: {{amount}}"
											: "Enterprise default: {{amount}}",
										{
											amount:
												ceiling[key] === null
													? t("monthlyUsage.noCap", "No cap")
													: formatBudgetAmount(ceiling[key], key),
										},
									)}
						</p>
					</div>
				))}
			</div>
			<p className="text-xs text-muted-foreground">
				{t(
					"monthlyUsage.tokens",
					"{{input}} input, {{output}} output, {{embedding}} embedding tokens",
					{
						input: view.inputTokens.toLocaleString(),
						output: view.outputTokens.toLocaleString(),
						embedding: view.embeddingTokens.toLocaleString(),
					},
				)}
			</p>
			{view.models.length > 0 && (
				<details className="rounded-lg border p-3 text-sm">
					<summary className="cursor-pointer font-medium">
						{t("monthlyUsage.models", "Model usage")}
					</summary>
					<p className="my-2 text-xs text-muted-foreground">
						{t(
							"monthlyUsage.modelCurrency",
							"Reported model costs below are in USD. Totals may be incomplete while requests settle or prices are unavailable. Monthly budgets above are in EUR and include hosted serving costs.",
						)}
					</p>
					<div className="overflow-x-auto">
						<table className="w-full text-left text-xs">
							<thead>
								<tr className="border-b [&>th]:py-2 [&>th]:pr-3">
									<th>{t("monthlyUsage.model", "Model")}</th>
									<th>{t("monthlyUsage.requests", "Requests")}</th>
									<th>
										{t(
											"monthlyUsage.tokenHeading",
											"Tokens (in / out / embedding)",
										)}
									</th>
									<th>
										{t("monthlyUsage.providerCost", "Reported cost (USD)")}
									</th>
								</tr>
							</thead>
							<tbody>
								{view.models.map((model) => (
									<tr
										key={`${model.provider}:${model.modelId}`}
										className="border-b last:border-0 [&>td]:py-2 [&>td]:pr-3"
									>
										<td className="max-w-64 break-words">
											{model.modelId ??
												t("monthlyUsage.unknownModel", "Unspecified model")}
											<span className="block text-muted-foreground">
												{model.provider}
											</span>
										</td>
										<td>{model.calls.toLocaleString()}</td>
										<td>
											{model.inputTokens.toLocaleString()} /{" "}
											{model.outputTokens.toLocaleString()} /{" "}
											{model.embeddingTokens.toLocaleString()}
										</td>
										<td>
											{new Intl.NumberFormat(undefined, {
												style: "currency",
												currency: "USD",
												maximumFractionDigits: 6,
											}).format(model.costMicroDollars / 1_000_000)}
										</td>
									</tr>
								))}
							</tbody>
						</table>
					</div>
				</details>
			)}
		</div>
	);
}
