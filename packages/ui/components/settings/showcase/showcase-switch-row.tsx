"use client";

import { useId } from "react";
import { cn } from "../../../lib/utils";
import { Switch } from "../../ui/switch";

export function ShowcaseSwitchRow({
	label,
	hint,
	checked,
	disabled,
	master,
	onChange,
}: Readonly<{
	label: string;
	hint?: string;
	checked: boolean;
	disabled?: boolean;
	master?: boolean;
	onChange: (value: boolean) => void;
}>) {
	const id = useId();
	return (
		<label
			htmlFor={id}
			className={cn(
				"flex items-center justify-between gap-4 py-2",
				disabled ? "cursor-not-allowed opacity-60" : "cursor-pointer",
			)}
		>
			<span className="min-w-0">
				<span className={cn("block text-sm", master && "font-semibold")}>
					{label}
				</span>
				{hint && (
					<span className="block text-xs text-muted-foreground">{hint}</span>
				)}
			</span>
			<Switch
				id={id}
				checked={checked}
				disabled={disabled}
				onCheckedChange={onChange}
				aria-label={label}
			/>
		</label>
	);
}
