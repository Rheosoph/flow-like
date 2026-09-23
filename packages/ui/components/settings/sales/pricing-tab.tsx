"use client";

import { useTranslation } from "@flow-like/locales";
import { CopyIcon, PercentIcon, PlusIcon, TagIcon } from "lucide-react";
import type { IDiscount } from "../../../state/backend-state/sales-state";
import { SellerTermsCard } from "../../payments/app-payment-settings";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../../ui/card";
import { Switch } from "../../ui/switch";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "../../ui/table";
import { formatCurrency } from "./sales-format";
import { EmptyCard, SectionHeading } from "./sales-parts";

function DiscountStatus({ discount }: { discount: IDiscount }) {
	const { t } = useTranslation("settings");
	if (discount.isValid)
		return <Badge variant="default">{t("active", "Active")}</Badge>;
	if (discount.isActive)
		return <Badge variant="secondary">{t("expired", "Expired")}</Badge>;
	return <Badge variant="outline">{t("disabled", "Disabled")}</Badge>;
}

function DiscountsTable({
	discounts,
	onEdit,
	onToggle,
	onDelete,
	onCopy,
}: {
	discounts: IDiscount[];
	onEdit: (discount: IDiscount) => void;
	onToggle: (discountId: string) => void;
	onDelete: (discountId: string) => void;
	onCopy: (code: string) => void;
}) {
	const { t } = useTranslation("settings");
	return (
		<Card>
			<Table>
				<TableHeader>
					<TableRow>
						<TableHead>{t("code", "Code")}</TableHead>
						<TableHead>{t("discountName", "Name")}</TableHead>
						<TableHead>{t("discount", "Discount")}</TableHead>
						<TableHead>{t("uses", "Uses")}</TableHead>
						<TableHead>{t("status", "Status")}</TableHead>
						<TableHead className="text-right">
							{t("actions", "Actions")}
						</TableHead>
					</TableRow>
				</TableHeader>
				<TableBody>
					{discounts.map((discount) => (
						<TableRow key={discount.id}>
							<TableCell>
								<div className="flex items-center gap-2">
									<code className="bg-muted px-2 py-1 rounded text-sm">
										{discount.code}
									</code>
									<Button
										variant="ghost"
										size="icon"
										className="h-6 w-6"
										aria-label={t("copyCode", "Copy code")}
										onClick={() => onCopy(discount.code)}
									>
										<CopyIcon className="h-3 w-3" />
									</Button>
								</div>
							</TableCell>
							<TableCell>{discount.name}</TableCell>
							<TableCell>
								{discount.discountType === "Percentage" ? (
									<span className="flex items-center gap-1">
										<PercentIcon className="h-3 w-3" />
										{`${discount.discountValue}%`}
									</span>
								) : (
									formatCurrency(discount.discountValue)
								)}
							</TableCell>
							<TableCell>
								{discount.usedCount}
								{discount.maxUses && ` / ${discount.maxUses}`}
							</TableCell>
							<TableCell>
								<div className="flex items-center gap-2">
									<Switch
										checked={discount.isActive}
										onCheckedChange={() => onToggle(discount.id)}
									/>
									<DiscountStatus discount={discount} />
								</div>
							</TableCell>
							<TableCell className="text-right">
								<div className="flex justify-end gap-2">
									<Button
										variant="ghost"
										size="sm"
										onClick={() => onEdit(discount)}
									>
										{t("edit", "Edit")}
									</Button>
									<Button
										variant="ghost"
										size="sm"
										className="text-destructive"
										onClick={() => onDelete(discount.id)}
									>
										{t("delete", "Delete")}
									</Button>
								</div>
							</TableCell>
						</TableRow>
					))}
				</TableBody>
			</Table>
		</Card>
	);
}

function PriceCard({
	price,
	onEditPrice,
}: {
	price: number;
	onEditPrice: () => void;
}) {
	const { t } = useTranslation("settings");
	return (
		<Card>
			<CardHeader>
				<CardTitle>{t("storePrice", "Store price")}</CardTitle>
				<CardDescription>
					{t(
						"storePriceDescription",
						"What a buyer pays once to use this app. At zero the app is free.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent className="flex flex-wrap items-center justify-between gap-4">
				<span className="text-3xl font-bold">
					{price > 0 ? formatCurrency(price) : t("free", "Free")}
				</span>
				<Button variant="outline" onClick={onEditPrice}>
					{t("updatePrice", "Update Price")}
				</Button>
			</CardContent>
		</Card>
	);
}

export function PricingTab({
	appId,
	storeListed,
	price,
	discounts,
	onEditPrice,
	onCreateDiscount,
	onEditDiscount,
	onToggleDiscount,
	onDeleteDiscount,
	onCopyCode,
}: {
	appId: string;
	storeListed: boolean;
	price: number | null;
	discounts: IDiscount[];
	onEditPrice: () => void;
	onCreateDiscount: () => void;
	onEditDiscount: (discount: IDiscount) => void;
	onToggleDiscount: (discountId: string) => void;
	onDeleteDiscount: (discountId: string) => void;
	onCopyCode: (code: string) => void;
}) {
	const { t } = useTranslation("settings");

	return (
		<div className="space-y-6">
			<SellerTermsCard appId={appId} />

			{!storeListed || price === null ? (
				<EmptyCard
					icon={TagIcon}
					message={t(
						"storePricingNeedsPublicApp",
						"Store pricing needs a public app. Make the app public to set a price and offer discount codes.",
					)}
				/>
			) : (
				<>
					<PriceCard price={price} onEditPrice={onEditPrice} />

					<div className="space-y-4">
						<SectionHeading
							title={t("discountCodes", "Discount Codes")}
							description={t(
								"managePromotionalDiscountsForYourApp",
								"Manage promotional discounts for your app",
							)}
							action={
								<Button onClick={onCreateDiscount}>
									<PlusIcon className="h-4 w-4 mr-2" />
									{t("addDiscount", "Add Discount")}
								</Button>
							}
						/>

						{discounts.length === 0 ? (
							<EmptyCard
								icon={TagIcon}
								message={t("noDiscountsCreatedYet", "No discounts created yet")}
							>
								<Button
									variant="outline"
									className="mt-4"
									onClick={onCreateDiscount}
								>
									{t("createYourFirstDiscount", "Create your first discount")}
								</Button>
							</EmptyCard>
						) : (
							<DiscountsTable
								discounts={discounts}
								onEdit={onEditDiscount}
								onToggle={onToggleDiscount}
								onDelete={onDeleteDiscount}
								onCopy={onCopyCode}
							/>
						)}
					</div>
				</>
			)}
		</div>
	);
}
