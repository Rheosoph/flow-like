"use client";

import { useId, useState } from "react";
import { Input } from "../ui/input";
import { microUsdToUsd } from "./bit-editor-model";

export function HostedPricingField({
	label,
	value,
	optional,
	onChange,
}: {
	label: string;
	value: unknown;
	optional: boolean;
	onChange: (text: string) => void;
}) {
	const id = useId();
	const [editing, setEditing] = useState<string | null>(null);
	return (
		<div className="space-y-2">
			<label htmlFor={id} className="text-sm font-medium">
				{label}
				{optional ? " (optional)" : " *"}
			</label>
			<Input
				id={id}
				inputMode="decimal"
				value={editing ?? microUsdToUsd(value)}
				placeholder={optional ? "No request fee" : "Enter an amount"}
				onFocus={() => setEditing(microUsdToUsd(value))}
				onBlur={() => setEditing(null)}
				onChange={(event) => {
					setEditing(event.target.value);
					onChange(event.target.value);
				}}
			/>
		</div>
	);
}
