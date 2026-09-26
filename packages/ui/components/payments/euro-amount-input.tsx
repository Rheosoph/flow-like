"use client";

import { useTranslation } from "@flow-like/locales";
import { useEffect, useState } from "react";
import { Input } from "../ui/input";
import { amountInput, parseEuroAmount } from "./types";

export function EuroAmountInput({
	id,
	value,
	disabled,
	onChange,
	onValidityChange,
}: {
	id?: string;
	value: number | null | undefined;
	disabled?: boolean;
	onChange: (minor: number) => void;
	onValidityChange?: (valid: boolean) => void;
}) {
	const { t } = useTranslation("payments");
	const [draft, setDraft] = useState(amountInput(value ?? 0));
	const [focused, setFocused] = useState(false);
	const minor = draft.trim() === "" ? 0 : parseEuroAmount(draft);
	useEffect(() => {
		if (!focused) setDraft(amountInput(value ?? 0));
	}, [value, focused]);
	useEffect(() => {
		onValidityChange?.(minor !== null && minor <= 4_294_967_295);
	}, [minor, onValidityChange]);
	return (
		<div className="space-y-2">
			<Input
				id={id}
				inputMode="decimal"
				placeholder="0.00"
				disabled={disabled}
				value={draft}
				aria-invalid={minor === null || minor > 4_294_967_295}
				onFocus={() => setFocused(true)}
				onBlur={() => setFocused(false)}
				onChange={(event) => {
					const text = event.target.value;
					setDraft(text);
					const parsed = text.trim() === "" ? 0 : parseEuroAmount(text);
					if (parsed !== null && parsed <= 4_294_967_295) onChange(parsed);
				}}
			/>
			{minor === null && (
				<p className="text-xs text-destructive">
					{t(
						"invalidEuroAmount",
						"Enter a euro amount with at most two decimal places.",
					)}
				</p>
			)}
			<p className="text-xs text-muted-foreground">
				{t("zeroPriceFree", "Set 0.00 for free access.")}
			</p>
		</div>
	);
}
