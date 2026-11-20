"use client";

import { type ReactNode, cloneElement, isValidElement, useId } from "react";

export const selectClass =
	"h-9 w-full min-w-0 rounded-md border border-input bg-background px-2 text-sm";

export function Field({
	label,
	children,
	hint,
}: { label: string; children: ReactNode; hint?: string }) {
	const id = useId();
	return (
		<div className="flex min-w-0 flex-col gap-1.5 text-sm">
			<label htmlFor={id} className="text-xs font-medium">
				{label}
			</label>
			{isValidElement<Record<string, unknown>>(children)
				? cloneElement(children, { id })
				: children}
			{hint && (
				<span className="text-xs font-normal text-muted-foreground">
					{hint}
				</span>
			)}
		</div>
	);
}

export function Choice({
	label,
	value,
	options,
	onChange,
	optional,
	hint,
}: {
	label: string;
	value: string;
	options: readonly (readonly [string, string])[];
	onChange: (value: string) => void;
	optional?: string;
	hint?: string;
}) {
	return (
		<Field label={label} hint={hint}>
			<select
				className={selectClass}
				value={value}
				onChange={(event) => onChange(event.target.value)}
			>
				{optional !== undefined && <option value="">{optional}</option>}
				{options.map(([id, name]) => (
					<option key={id} value={id}>
						{name}
					</option>
				))}
			</select>
		</Field>
	);
}
