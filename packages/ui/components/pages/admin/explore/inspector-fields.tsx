"use client";

import { type ReactNode, useId } from "react";
import { cn } from "../../../../lib/utils";
import { Input } from "../../../ui/input";
import { Switch } from "../../../ui/switch";
import { Textarea } from "../../../ui/textarea";
import { SECTION_TITLE } from "./explore-admin-visuals";

export function InspectorSection({
	title,
	meta,
	action,
	children,
	className,
}: {
	title: ReactNode;
	meta?: ReactNode;
	action?: ReactNode;
	children: ReactNode;
	className?: string;
}) {
	return (
		<section className={cn("flex flex-col gap-2", className)}>
			<div className="flex min-h-6 items-center gap-2">
				<h3 className={cn(SECTION_TITLE, "shrink-0 whitespace-nowrap")}>
					{title}
				</h3>
				{meta && (
					<span className="min-w-0 truncate text-[11.5px] text-muted-foreground">
						{meta}
					</span>
				)}
				{action && <div className="ml-auto shrink-0">{action}</div>}
			</div>
			{children}
		</section>
	);
}

export function FieldError({ id, message }: { id: string; message?: string }) {
	if (!message) return null;
	return (
		<p id={id} className="text-[11.5px] leading-snug text-destructive">
			{message}
		</p>
	);
}

export function CountedField({
	label,
	value,
	max,
	placeholder,
	multiline = false,
	half = false,
	error,
	inputMode,
	onChange,
}: {
	label: string;
	value: string | null | undefined;
	max?: number;
	placeholder?: string;
	multiline?: boolean;
	half?: boolean;
	error?: string;
	inputMode?: "url" | "text";
	onChange: (value: string) => void;
}) {
	const id = useId();
	const errorId = `${id}-error`;
	const text = value ?? "";
	const count = Array.from(text.trim()).length;
	const common = {
		id,
		value: text,
		placeholder,
		"aria-invalid": error ? true : undefined,
		"aria-describedby": error ? errorId : undefined,
		className: "bg-background text-[13px]",
	};
	return (
		<div
			className={cn(
				"flex min-w-0 flex-col gap-1.25",
				half ? "col-span-1" : "col-span-2",
			)}
		>
			<div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
				<label htmlFor={id}>{label}</label>
				{max !== undefined && (
					<span
						className={cn(
							"font-mono text-[11px] tabular-nums",
							count > max && "text-destructive",
						)}
					>
						{count}/{max}
					</span>
				)}
			</div>
			{multiline ? (
				<Textarea
					{...common}
					rows={2}
					className={cn(common.className, "min-h-0 resize-none leading-4.75")}
					onChange={(event) => onChange(event.target.value)}
				/>
			) : (
				<Input
					{...common}
					type="text"
					inputMode={inputMode}
					className={cn(common.className, "h-8.5")}
					onChange={(event) => onChange(event.target.value)}
				/>
			)}
			<FieldError id={errorId} message={error} />
		</div>
	);
}

export interface SegmentOption<T extends string> {
	value: T;
	label: string;
	adornment?: ReactNode;
	pressedClassName?: string;
}

export function Segmented<T extends string>({
	label,
	options,
	value,
	onChange,
	className,
	size = "md",
}: {
	label: string;
	options: readonly SegmentOption<T>[];
	value: T;
	onChange: (value: T) => void;
	className?: string;
	size?: "sm" | "md";
}) {
	return (
		<fieldset
			aria-label={label}
			className={cn(
				"grid min-w-0 gap-1 rounded-lg border bg-card p-0.75",
				className,
			)}
			style={{
				gridTemplateColumns: `repeat(${options.length}, minmax(0, 1fr))`,
			}}
		>
			{options.map((option) => {
				const pressed = option.value === value;
				return (
					<button
						key={option.value}
						type="button"
						aria-pressed={pressed}
						title={option.label}
						onClick={() => onChange(option.value)}
						className={cn(
							"inline-flex min-w-0 items-center justify-center gap-1.25 truncate rounded-[5px] border px-1 text-xs transition-colors focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ring",
							size === "sm" ? "h-6.5" : "h-7",
							pressed
								? (option.pressedClassName ??
										"border-transparent bg-muted font-semibold text-foreground shadow-xs")
								: "border-transparent font-medium text-muted-foreground hover:text-foreground",
						)}
					>
						{option.adornment}
						<span className="truncate">{option.label}</span>
					</button>
				);
			})}
		</fieldset>
	);
}

export function SwitchRow({
	label,
	note,
	checked,
	disabled,
	onCheckedChange,
	trailing,
}: {
	label: string;
	note?: string;
	checked: boolean;
	disabled?: boolean;
	onCheckedChange: (checked: boolean) => void;
	trailing?: ReactNode;
}) {
	const id = useId();
	return (
		<div className="flex items-center gap-3 rounded-lg border bg-muted/30 px-3 py-2.5">
			<Switch
				id={id}
				checked={checked}
				disabled={disabled}
				onCheckedChange={onCheckedChange}
				aria-describedby={note ? `${id}-note` : undefined}
			/>
			<div className="flex min-w-0 flex-1 flex-col">
				<label htmlFor={id} className="text-[13px] font-medium">
					{label}
				</label>
				{note && (
					<span
						id={`${id}-note`}
						className="text-[11.5px] leading-snug text-muted-foreground"
					>
						{note}
					</span>
				)}
			</div>
			{trailing}
		</div>
	);
}
