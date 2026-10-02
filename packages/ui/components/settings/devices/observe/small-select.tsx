"use client";

import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../ui/select";

export interface SmallSelectOption {
	value: string;
	label: string;
}

/** The 28 px select of block tool rows (who sent it, which log source). */
export function SmallSelect({
	value,
	onChange,
	label,
	options,
}: Readonly<{
	value: string;
	onChange(value: string): void;
	/** Accessible name. */
	label: string;
	options: readonly SmallSelectOption[];
}>) {
	return (
		<Select value={value} onValueChange={onChange}>
			<SelectTrigger
				size="sm"
				aria-label={label}
				className="h-7 rounded-lg border-border bg-card text-ui shadow-none"
			>
				<SelectValue />
			</SelectTrigger>
			<SelectContent className="border-border-strong bg-popover shadow-none">
				{options.map((option) => (
					<SelectItem
						key={option.value}
						value={option.value}
						className="focus:bg-row-hover focus:text-foreground"
					>
						{option.label}
					</SelectItem>
				))}
			</SelectContent>
		</Select>
	);
}
