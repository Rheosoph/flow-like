"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import type {
	ICreateDiscountRequest,
	IDiscount,
} from "../../../state/backend-state/sales-state";
import { EuroAmountInput } from "../../payments/euro-amount-input";
import { Button } from "../../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { Textarea } from "../../ui/textarea";
import { formatCurrency } from "./sales-format";

export function DiscountDialog({
	open,
	onOpenChange,
	onSave,
	discount,
}: {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	onSave: (discount: ICreateDiscountRequest) => Promise<void>;
	discount?: IDiscount;
}) {
	const { t } = useTranslation("settings");
	const [form, setForm] = useState<ICreateDiscountRequest>({
		code: "",
		name: "",
		description: "",
		discountType: "percentage",
		discountValue: 10,
		maxUses: undefined,
		minPurchaseAmount: undefined,
		startsAt: undefined,
		expiresAt: undefined,
	});
	const [saving, setSaving] = useState(false);

	// biome-ignore lint/correctness/useExhaustiveDependencies: Reset form when dialog opens/closes
	useEffect(() => {
		if (discount) {
			setForm({
				code: discount.code,
				name: discount.name,
				description: discount.description || "",
				discountType:
					discount.discountType === "Percentage"
						? "percentage"
						: "fixed_amount",
				discountValue: discount.discountValue,
				maxUses: discount.maxUses || undefined,
				minPurchaseAmount: discount.minPurchaseAmount || undefined,
				startsAt: discount.startsAt,
				expiresAt: discount.expiresAt || undefined,
			});
		} else {
			setForm({
				code: "",
				name: "",
				description: "",
				discountType: "percentage",
				discountValue: 10,
				maxUses: undefined,
				minPurchaseAmount: undefined,
				startsAt: undefined,
				expiresAt: undefined,
			});
		}
	}, [discount, open]);

	const handleSave = useCallback(async () => {
		setSaving(true);
		try {
			await onSave(form);
			onOpenChange(false);
		} catch (error) {
			toast.error(
				error instanceof Error
					? error.message
					: t("failedToSaveDiscount", "Failed to save discount"),
			);
		} finally {
			setSaving(false);
		}
	}, [form, onSave, onOpenChange, t]);

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="max-w-md">
				<DialogHeader>
					<DialogTitle>
						{discount
							? t("editDiscount", "Edit Discount")
							: t("createDiscount", "Create Discount")}
					</DialogTitle>
					<DialogDescription>
						{discount
							? t("updateTheDiscountDetails", "Update the discount details")
							: "Create a new discount code for your app"}
					</DialogDescription>
				</DialogHeader>

				<div className="grid gap-4 py-4">
					<div className="grid gap-2">
						<Label htmlFor="code">{t("code", "Code")}</Label>
						<Input
							id="code"
							placeholder="LAUNCH20"
							value={form.code}
							onChange={(e) =>
								setForm({ ...form, code: e.target.value.toUpperCase() })
							}
						/>
					</div>

					<div className="grid gap-2">
						<Label htmlFor="name">Name</Label>
						<Input
							id="name"
							placeholder={t("launchDiscount", "Launch Discount")}
							value={form.name}
							onChange={(e) => setForm({ ...form, name: e.target.value })}
						/>
					</div>

					<div className="grid gap-2">
						<Label htmlFor="description">
							{t("description", "Description")}
						</Label>
						<Textarea
							id="description"
							placeholder={t("specialLaunchOffer", "Special launch offer...")}
							value={form.description}
							onChange={(e) =>
								setForm({ ...form, description: e.target.value })
							}
						/>
					</div>

					<div className="grid grid-cols-2 gap-4">
						<div className="grid gap-2">
							<Label>Type</Label>
							<Select
								value={form.discountType}
								onValueChange={(v) =>
									setForm({
										...form,
										discountType: v as "percentage" | "fixed_amount",
									})
								}
							>
								<SelectTrigger>
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									<SelectItem value="percentage">
										{t("percentage", "Percentage")}
									</SelectItem>
									<SelectItem value="fixed_amount">
										{t("fixedAmount", "Fixed Amount")}
									</SelectItem>
								</SelectContent>
							</Select>
						</div>

						<div className="grid gap-2">
							<Label htmlFor="value">
								{t("value", "Value")}{" "}
								{form.discountType === "percentage" ? "(%)" : "(cents)"}
							</Label>
							<Input
								id="value"
								type="number"
								min={0}
								max={form.discountType === "percentage" ? 100 : undefined}
								value={form.discountValue}
								onChange={(e) =>
									setForm({
										...form,
										discountValue: Number.parseInt(e.target.value) || 0,
									})
								}
							/>
						</div>
					</div>

					<div className="grid grid-cols-2 gap-4">
						<div className="grid gap-2">
							<Label htmlFor="maxUses">
								{t("maxUsesOptional", "Max Uses (optional)")}
							</Label>
							<Input
								id="maxUses"
								type="number"
								min={1}
								placeholder="Unlimited"
								value={form.maxUses || ""}
								onChange={(e) =>
									setForm({
										...form,
										maxUses: e.target.value
											? Number.parseInt(e.target.value)
											: undefined,
									})
								}
							/>
						</div>

						<div className="grid gap-2">
							<Label htmlFor="minAmount">
								{t("minAmountCents", "Min Amount (cents)")}
							</Label>
							<Input
								id="minAmount"
								type="number"
								min={0}
								placeholder={t("noMinimum", "No minimum")}
								value={form.minPurchaseAmount || ""}
								onChange={(e) =>
									setForm({
										...form,
										minPurchaseAmount: e.target.value
											? Number.parseInt(e.target.value)
											: undefined,
									})
								}
							/>
						</div>
					</div>
				</div>

				<DialogFooter>
					<Button variant="outline" onClick={() => onOpenChange(false)}>
						{t("cancel", "Cancel")}
					</Button>
					<Button
						onClick={handleSave}
						disabled={saving || !form.code || !form.name}
					>
						{saving ? "Saving..." : discount ? "Update" : "Create"}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

export function PriceEditorDialog({
	open,
	onOpenChange,
	currentPrice,
	onSave,
}: {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	currentPrice: number;
	onSave: (price: number) => Promise<void>;
}) {
	const { t } = useTranslation("settings");
	const [price, setPrice] = useState(currentPrice);
	const [priceValid, setPriceValid] = useState(true);
	const [saving, setSaving] = useState(false);

	// biome-ignore lint/correctness/useExhaustiveDependencies: Reset price when dialog opens/closes
	useEffect(() => {
		setPrice(currentPrice);
	}, [currentPrice, open]);

	const handleSave = useCallback(async () => {
		setSaving(true);
		try {
			await onSave(price);
			onOpenChange(false);
			toast.success("Price updated successfully");
		} catch (error) {
			toast.error(
				error instanceof Error
					? error.message
					: t("failedToUpdatePrice", "Failed to update price"),
			);
		} finally {
			setSaving(false);
		}
	}, [price, onSave, onOpenChange, t]);

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="max-w-sm">
				<DialogHeader>
					<DialogTitle>{t("updatePrice", "Update Price")}</DialogTitle>
					<DialogDescription>
						{t("setEuroPrice", "Set a new price for your app in euros.")}
					</DialogDescription>
				</DialogHeader>

				<div className="grid gap-4 py-4">
					<div className="grid gap-2">
						<Label htmlFor="price">{t("priceEur", "Price (EUR)")}</Label>
						<div className="flex items-center gap-2">
							<EuroAmountInput
								id="price"
								value={price}
								onChange={setPrice}
								onValidityChange={setPriceValid}
							/>
							<span className="text-muted-foreground whitespace-nowrap">
								= {formatCurrency(price)}
							</span>
						</div>
					</div>
				</div>

				<DialogFooter>
					<Button variant="outline" onClick={() => onOpenChange(false)}>
						{t("cancel", "Cancel")}
					</Button>
					<Button onClick={handleSave} disabled={saving || !priceValid}>
						{saving ? "Saving..." : t("updatePrice", "Update Price")}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
