"use client";

import { useTranslation } from "@flow-like/locales";
import { type ReactNode, useId } from "react";
import { useAppCategoryLabel } from "../../../../lib/app-category";
import { APP_TYPE_ORDER, appTypeLabel } from "../../../../lib/app-type";
import { APP_CATEGORY_ORDER } from "../../../../lib/category-meta";
import type { IAppCategory, IAppType } from "../../../../lib/schema/app/app";
import type { WasmPackageCategory } from "../../../../lib/schema/wasm";
import { useExploreLabels } from "../../../store/explore/explore-labels";
import {
	type ExploreCollection,
	type ExploreCollectionRule,
	WASM_PACKAGE_CATEGORIES,
} from "../../../store/explore/explore-types";
import { Input } from "../../../ui/input";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../ui/select";
import { Switch } from "../../../ui/switch";
import { FieldError, Segmented } from "./inspector-fields";

const ANY = "__any";
const RATINGS = ["3", "3.5", "4", "4.5"] as const;

function RuleRow({
	label,
	children,
}: {
	label: string;
	children: (id: string) => ReactNode;
}) {
	const id = useId();
	return (
		<div className="grid grid-cols-[88px_minmax(0,1fr)] items-center gap-2 text-xs">
			<label htmlFor={id} className="text-muted-foreground">
				{label}
			</label>
			{children(id)}
		</div>
	);
}

function OptionSelect({
	id,
	value,
	options,
	onChange,
}: {
	id: string;
	value: string;
	options: readonly { value: string; label: string }[];
	onChange: (value: string) => void;
}) {
	return (
		<Select value={value} onValueChange={onChange}>
			<SelectTrigger
				id={id}
				size="sm"
				className="h-7.5 w-full bg-background text-xs"
			>
				<SelectValue />
			</SelectTrigger>
			<SelectContent>
				{options.map((option) => (
					<SelectItem key={option.value} value={option.value}>
						{option.label}
					</SelectItem>
				))}
			</SelectContent>
		</Select>
	);
}

export function CollectionRuleEditor({
	rule,
	resolved,
	onChange,
	issueFor,
}: {
	rule: ExploreCollectionRule;
	/** The collection as the current preview resolved it, when the preview shows it. */
	resolved?: ExploreCollection;
	onChange: (rule: ExploreCollectionRule) => void;
	issueFor: (field: string) => string | undefined;
}) {
	const { t } = useTranslation("admin");
	const categoryLabel = useAppCategoryLabel();
	const labels = useExploreLabels();
	const any = { value: ANY, label: t("exploreRuleAny", "Any") };
	const set = (patch: Partial<ExploreCollectionRule>) =>
		onChange({ ...rule, ...patch });
	const limitError = issueFor("rule.limit");
	return (
		<div className="flex flex-col gap-2 rounded-lg border bg-muted/30 p-2.5">
			<Segmented
				size="sm"
				label={t("exploreRuleItemKind", "Rule picks")}
				value={rule.itemKind}
				options={[
					{ value: "app", label: t("exploreRuleApps", "Apps") },
					{ value: "package", label: t("exploreRulePackages", "Packages") },
				]}
				onChange={(itemKind) => set({ itemKind })}
			/>
			{rule.itemKind === "app" ? (
				<>
					<RuleRow label={t("exploreRuleCategory", "Category")}>
						{(id) => (
							<OptionSelect
								id={id}
								value={rule.appCategory ?? ANY}
								options={[
									any,
									...APP_CATEGORY_ORDER.map((category) => ({
										value: category,
										label: categoryLabel(category),
									})),
								]}
								onChange={(value) =>
									set({
										appCategory: value === ANY ? null : (value as IAppCategory),
									})
								}
							/>
						)}
					</RuleRow>
					<RuleRow label={t("exploreRuleAppType", "App type")}>
						{(id) => (
							<OptionSelect
								id={id}
								value={rule.appType ?? ANY}
								options={[
									any,
									...APP_TYPE_ORDER.map((type) => ({
										value: type,
										label: appTypeLabel(type),
									})),
								]}
								onChange={(value) =>
									set({ appType: value === ANY ? null : (value as IAppType) })
								}
							/>
						)}
					</RuleRow>
				</>
			) : (
				<>
					<RuleRow label={t("exploreRuleCategory", "Category")}>
						{(id) => (
							<OptionSelect
								id={id}
								value={rule.packageCategory ?? ANY}
								options={[
									any,
									...WASM_PACKAGE_CATEGORIES.map((category) => ({
										value: category,
										label: labels.packageCategory(category),
									})),
								]}
								onChange={(value) =>
									set({
										packageCategory:
											value === ANY ? null : (value as WasmPackageCategory),
									})
								}
							/>
						)}
					</RuleRow>
					<RuleRow label={t("exploreRuleTrust", "Trust")}>
						{(id) => (
							<div className="flex items-center gap-2">
								<Switch
									id={id}
									checked={rule.verifiedOnly}
									onCheckedChange={(verifiedOnly) => set({ verifiedOnly })}
								/>
								<span className="text-muted-foreground">
									{t("exploreRuleVerifiedOnly", "Verified publishers only")}
								</span>
							</div>
						)}
					</RuleRow>
				</>
			)}
			<RuleRow label={t("exploreRuleRating", "Rating")}>
				{(id) => (
					<OptionSelect
						id={id}
						value={rule.minRating == null ? ANY : String(rule.minRating)}
						options={[
							any,
							...RATINGS.map((rating) => ({
								value: rating,
								label: t("exploreRuleRatingAtLeast", "{{rating}} or more", {
									rating,
								}),
							})),
						]}
						onChange={(value) =>
							set({ minRating: value === ANY ? null : Number(value) })
						}
					/>
				)}
			</RuleRow>
			<RuleRow label={t("exploreRulePrice", "Price")}>
				{(id) => (
					<OptionSelect
						id={id}
						value={rule.price ?? ANY}
						options={[
							any,
							{ value: "free", label: t("exploreRuleFree", "Free") },
							{ value: "paid", label: t("exploreRulePaid", "Paid") },
						]}
						onChange={(value) =>
							set({ price: value === ANY ? null : (value as "free" | "paid") })
						}
					/>
				)}
			</RuleRow>
			<RuleRow label={t("exploreRuleSort", "Sort")}>
				{(id) => (
					<OptionSelect
						id={id}
						value={rule.sort}
						options={[
							{
								value: "installs",
								label: t("exploreRuleSortInstalls", "Most installed"),
							},
							{
								value: "rating",
								label: t("exploreRuleSortRating", "Best rated"),
							},
							{ value: "newest", label: t("exploreRuleSortNewest", "Newest") },
						]}
						onChange={(value) =>
							set({ sort: value as ExploreCollectionRule["sort"] })
						}
					/>
				)}
			</RuleRow>
			<RuleRow label={t("exploreRuleLimit", "Items shown")}>
				{(id) => (
					<Input
						id={id}
						type="number"
						min={2}
						max={12}
						value={Number.isFinite(rule.limit) ? rule.limit : ""}
						aria-invalid={limitError ? true : undefined}
						className="h-7.5 bg-background text-xs"
						onChange={(event) => set({ limit: Number(event.target.value) })}
					/>
				)}
			</RuleRow>
			<FieldError id="rule-limit-error" message={limitError} />
			<FieldError id="rule-rating-error" message={issueFor("rule.minRating")} />
			{resolved && (
				<p className="text-[11.5px] text-muted-foreground">
					{t("exploreRuleMatches", "Shows {{counts}} in this preview", {
						counts: labels.kindCounts(resolved.apps, resolved.packages),
					})}
				</p>
			)}
		</div>
	);
}
