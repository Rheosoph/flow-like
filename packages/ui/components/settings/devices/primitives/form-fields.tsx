"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Eye,
	EyeOff,
	FileUp,
	type LucideIcon,
	OctagonX,
	Plus,
	TriangleAlert,
	X,
} from "lucide-react";
import {
	type ComponentProps,
	type DragEvent,
	type KeyboardEvent,
	type ReactElement,
	type ReactNode,
	cloneElement,
	isValidElement,
	useState,
} from "react";
import { Checkbox } from "../../../ui/checkbox";
import { Input } from "../../../ui/input";
import { Label } from "../../../ui/label";
import { RadioGroup, RadioGroupItem } from "../../../ui/radio-group";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../ui/select";
import { Switch } from "../../../ui/switch";
import { Textarea } from "../../../ui/textarea";
import { DvButton } from "./dv-button";
import { cx } from "./tone";

/* SPEC §4.35. Every control has a stable `id`; validation is inline; byte limits count UTF-8 bytes. */

/*
 * The shadcn bases carry `outline-none`, which also sets the outline style to
 * none; `outline-solid` brings the 2 px focus ring back (SPEC §4.35).
 */
const CONTROL =
	"rounded-lg border-input bg-card px-2.5 text-[13px]/[18px] shadow-none md:text-[13px]/[18px] hover:border-border-strong focus-visible:border-input focus-visible:ring-0 focus-visible:outline-2 focus-visible:outline-offset-0 focus-visible:outline-ring focus-visible:outline-solid aria-invalid:border-critical-line aria-invalid:ring-0 dark:bg-card disabled:opacity-60";

export function utf8Bytes(value: string): number {
	return new TextEncoder().encode(value).length;
}

export interface DvInputProps extends ComponentProps<"input"> {
	mono?: boolean;
	numeric?: boolean;
}

export function DvInput({
	mono = false,
	numeric = false,
	className,
	...props
}: DvInputProps) {
	return (
		<Input
			className={cx(
				CONTROL,
				"h-8.5 py-0",
				mono && "font-mono",
				numeric && "text-right tabular-nums",
				className,
			)}
			{...props}
		/>
	);
}

export function DvTextarea({
	className,
	...props
}: ComponentProps<"textarea">) {
	return <Textarea className={cx(CONTROL, "py-2", className)} {...props} />;
}

/** Label, hint and error around one control; wires `id`, `aria-describedby` and `aria-invalid`. */
export function Field({
	id,
	label,
	hint,
	error,
	className,
	children,
}: Readonly<{
	id: string;
	label: ReactNode;
	hint?: ReactNode;
	error?: ReactNode;
	className?: string;
	children: ReactElement<Record<string, unknown>>;
}>) {
	const hintId = `${id}-hint`;
	const errorId = `${id}-error`;
	const describedBy =
		[error ? errorId : null, hint ? hintId : null].filter(Boolean).join(" ") ||
		undefined;
	const control = isValidElement(children)
		? cloneElement(children, {
				id,
				"aria-describedby": describedBy,
				"aria-invalid": error ? true : undefined,
			})
		: children;
	return (
		<div
			data-field=""
			data-invalid={error ? "true" : undefined}
			className={cx("flex min-w-0 flex-col gap-1.5", className)}
		>
			<Label htmlFor={id} className="text-[13px]/[18px] font-medium">
				<span className="min-w-0">{label}</span>
			</Label>
			{control}
			{error ? (
				<p
					id={errorId}
					className="flex items-start gap-1 text-xs text-critical"
				>
					<OctagonX aria-hidden className="mt-px size-3.25 shrink-0" />
					<span>{error}</span>
				</p>
			) : null}
			{hint ? (
				<p id={hintId} className="text-xs text-muted-foreground">
					{hint}
				</p>
			) : null}
		</div>
	);
}

export interface SecretInputProps
	extends Omit<ComponentProps<"input">, "type" | "value" | "onChange"> {
	value: string;
	onValueChange(value: string): void;
	minBytes?: number;
	maxBytes?: number;
}

type KeyHandler = ComponentProps<"input">["onKeyDown"];

function useCapsLock(onKeyDown: KeyHandler, onKeyUp: KeyHandler) {
	const [capsLock, setCapsLock] = useState(false);
	const track =
		(next: KeyHandler) => (event: KeyboardEvent<HTMLInputElement>) => {
			setCapsLock(event.getModifierState?.("CapsLock") ?? false);
			next?.(event);
		};
	return {
		capsLock,
		onKeyDown: track(onKeyDown),
		onKeyUp: track(onKeyUp),
	};
}

function ByteCount({
	bytes,
	minBytes,
	maxBytes,
}: Readonly<{ bytes: number; minBytes?: number; maxBytes?: number }>) {
	const { t } = useTranslation("devices");
	if (maxBytes === undefined) return null;
	const text =
		minBytes === undefined
			? t(
					"common.secret.bytesMax",
					"{{count, number}} bytes · up to {{max, number}} bytes",
					{ count: bytes, max: maxBytes },
				)
			: t(
					"common.secret.bytesRange",
					"{{count, number}} bytes · {{min, number}}–{{max, number}} bytes",
					{ count: bytes, min: minBytes, max: maxBytes },
				);
	const outOfRange =
		bytes > maxBytes ||
		(minBytes !== undefined && bytes > 0 && bytes < minBytes);
	return (
		<p
			data-bytes={bytes}
			className={cx(
				"col-span-full text-xs tabular-nums",
				outOfRange ? "text-critical" : "text-muted-foreground",
			)}
		>
			{text}
		</p>
	);
}

/** Password field: show/hide, Caps Lock hint and a UTF-8 byte count when limits apply. */
export function SecretInput({
	value,
	onValueChange,
	minBytes,
	maxBytes,
	className,
	...props
}: SecretInputProps) {
	const { t } = useTranslation("devices");
	const [shown, setShown] = useState(false);
	const caps = useCapsLock(props.onKeyDown, props.onKeyUp);
	return (
		<div
			data-secret=""
			className={cx(
				"grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-1.5 gap-y-1",
				className,
			)}
		>
			<DvInput
				mono
				type={shown ? "text" : "password"}
				autoComplete="current-password"
				spellCheck={false}
				value={value}
				onChange={(event) => onValueChange(event.target.value)}
				{...props}
				onKeyDown={caps.onKeyDown}
				onKeyUp={caps.onKeyUp}
			/>
			<DvButton
				variant="ghost"
				size="xs"
				iconOnly
				icon={shown ? EyeOff : Eye}
				aria-label={
					shown
						? t("common.secret.hide", "Hide password")
						: t("common.secret.show", "Show password")
				}
				onClick={() => setShown((current) => !current)}
			/>
			{caps.capsLock ? (
				<output className="col-span-full flex items-center gap-1 text-xs text-warning">
					<TriangleAlert aria-hidden className="size-3.25" />
					{t("common.secret.capsLock", "Caps Lock is on")}
				</output>
			) : null}
			<ByteCount
				bytes={utf8Bytes(value)}
				minBytes={minBytes}
				maxBytes={maxBytes}
			/>
		</div>
	);
}

export function InputWithUnit({
	unit,
	className,
	...props
}: DvInputProps & { unit: ReactNode }) {
	return (
		<div className={cx("flex min-w-0 items-center", className)}>
			<DvInput className="rounded-r-none" {...props} />
			<span className="inline-flex h-8.5 items-center rounded-r-lg border border-l-0 border-input bg-surface-sunken px-2.5 text-xs whitespace-nowrap text-muted-foreground">
				{unit}
			</span>
		</div>
	);
}

export interface DvSelectOption<T extends string> {
	value: T;
	label: ReactNode;
	disabled?: boolean;
}

export interface DvSelectProps<T extends string> {
	/** `Field` fills `id`, `aria-describedby` and `aria-invalid`. */
	id?: string;
	value: T | undefined;
	onValueChange(value: T): void;
	options: readonly DvSelectOption<T>[];
	placeholder?: string;
	disabled?: boolean;
	/** `sm` is the 28 px select of block tool rows. */
	size?: "md" | "sm";
	mono?: boolean;
	className?: string;
	"aria-label"?: string;
	"aria-describedby"?: string;
	"aria-invalid"?: boolean;
}

/** The shadcn Select dressed like `DvInput`: flat popover, neutral highlight, the area's focus ring. */
export function DvSelect<T extends string>({
	value,
	onValueChange,
	options,
	placeholder,
	disabled,
	size = "md",
	mono = false,
	className,
	...trigger
}: Readonly<DvSelectProps<T>>) {
	return (
		<Select
			value={value ?? ""}
			onValueChange={(next) => onValueChange(next as T)}
			disabled={disabled}
		>
			<SelectTrigger
				data-slot="dv-select"
				size={size === "sm" ? "sm" : "default"}
				className={cx(
					CONTROL,
					"w-full data-[size=default]:h-8.5 data-[size=sm]:h-7 dark:hover:bg-card",
					size === "sm" && "w-fit",
					mono && "font-mono",
					className,
				)}
				{...trigger}
			>
				<SelectValue placeholder={placeholder} />
			</SelectTrigger>
			<SelectContent className="border-border-strong bg-popover shadow-none backdrop-blur-none">
				{options.map((option) => (
					<SelectItem
						key={option.value}
						value={option.value}
						disabled={option.disabled}
						className={cx(
							"text-[13px]/[18px] focus:bg-row-hover focus:text-foreground",
							mono && "font-mono",
						)}
					>
						{option.label}
					</SelectItem>
				))}
			</SelectContent>
		</Select>
	);
}

/** Editable list of single-line values (origins, names). */
export function ListEditor({
	id,
	values,
	onChange,
	addLabel,
	placeholder,
	mono = true,
	itemLabel,
}: Readonly<{
	id: string;
	values: readonly string[];
	onChange(values: string[]): void;
	addLabel: ReactNode;
	placeholder?: string;
	mono?: boolean;
	/** Accessible name of each row's input ("Origin 2"). */
	itemLabel(index: number): string;
}>) {
	const { t } = useTranslation("devices");
	const rows = values.map((value, index) => ({
		value,
		index,
		key: `${id}-${index}`,
	}));
	return (
		<div id={id} className="flex flex-col items-start gap-1.5">
			{rows.map((row) => (
				<div key={row.key} className="flex w-full items-center gap-1.5">
					<DvInput
						id={row.key}
						mono={mono}
						value={row.value}
						placeholder={placeholder}
						aria-label={itemLabel(row.index)}
						onChange={(event) =>
							onChange(
								values.map((current, index) =>
									index === row.index ? event.target.value : current,
								),
							)
						}
					/>
					<DvButton
						variant="ghost"
						size="xs"
						iconOnly
						icon={X}
						aria-label={t("common.list.remove", "Remove {{item}}", {
							item: itemLabel(row.index),
						})}
						onClick={() =>
							onChange(values.filter((_, index) => index !== row.index))
						}
					/>
				</div>
			))}
			<DvButton
				variant="ghost"
				size="sm"
				icon={Plus}
				onClick={() => onChange([...values, ""])}
			>
				{addLabel}
			</DvButton>
		</div>
	);
}

/** File drop target; selected files stay listed under it (and are kept on validation errors). */
export function DropZone({
	id,
	title,
	hint,
	accept,
	multiple = false,
	icon: Icon = FileUp,
	files,
	onFiles,
	className,
}: Readonly<{
	id: string;
	title: ReactNode;
	hint?: ReactNode;
	accept?: string;
	multiple?: boolean;
	icon?: LucideIcon;
	/** Chips for the files already chosen. */
	files?: ReactNode;
	onFiles(files: File[]): void;
	className?: string;
}>) {
	const [over, setOver] = useState(false);
	const onDrop = (event: DragEvent<HTMLLabelElement>) => {
		event.preventDefault();
		setOver(false);
		const dropped = Array.from(event.dataTransfer?.files ?? []);
		if (dropped.length) onFiles(multiple ? dropped : dropped.slice(0, 1));
	};
	return (
		<div className={cx("flex flex-col gap-1.5", className)}>
			<label
				htmlFor={id}
				data-over={over || undefined}
				onDragOver={(event) => {
					event.preventDefault();
					setOver(true);
				}}
				onDragLeave={() => setOver(false)}
				onDrop={onDrop}
				className={cx(
					"flex cursor-pointer flex-col items-center gap-1.5 rounded-lg border border-dashed border-border-strong bg-surface-sunken px-4 py-5 text-center text-ink-2 hover:border-foreground hover:bg-row-hover focus-within:outline-2 focus-within:outline-offset-2 focus-within:outline-ring",
					over && "border-foreground bg-row-hover",
				)}
			>
				<Icon aria-hidden className="size-5 text-muted-foreground" />
				<span className="text-ui">{title}</span>
				{hint ? (
					<span className="text-xs text-muted-foreground">{hint}</span>
				) : null}
				<input
					id={id}
					type="file"
					accept={accept}
					multiple={multiple}
					className="sr-only"
					onChange={(event) => {
						const chosen = Array.from(event.target.files ?? []);
						if (chosen.length) onFiles(chosen);
						event.target.value = "";
					}}
				/>
			</label>
			{files ? <div className="flex flex-wrap gap-1.5">{files}</div> : null}
		</div>
	);
}

export interface ChoiceOption<T extends string> {
	value: T;
	title: ReactNode;
	hint?: ReactNode;
	icon?: LucideIcon;
	disabled?: boolean;
	/** Shown inside the card while it is the selected one (the fields that belong to this choice). */
	detail?: ReactNode;
}

/** Radio cards (platform, mode). */
export function ChoiceCards<T extends string>({
	id,
	legend,
	value,
	onValueChange,
	options,
	className,
}: Readonly<{
	id: string;
	legend: ReactNode;
	value: T | undefined;
	onValueChange(value: T): void;
	options: readonly ChoiceOption<T>[];
	className?: string;
}>) {
	return (
		<fieldset className={cx("m-0 grid min-w-0 gap-2 border-0 p-0", className)}>
			<legend className="mb-2 p-0 text-[13px]/[18px] font-medium">
				{legend}
			</legend>
			<RadioGroup
				value={value ?? ""}
				onValueChange={(next) => onValueChange(next as T)}
				className="grid gap-2"
			>
				{options.map((option) => {
					const optionId = `${id}-${option.value}`;
					const Icon = option.icon;
					const checked = value === option.value;
					return (
						<div
							key={option.value}
							data-checked={checked || undefined}
							className={cx(
								"rounded-lg border border-border bg-card hover:border-border-strong has-focus-visible:outline-2 has-focus-visible:outline-offset-2 has-focus-visible:outline-ring",
								checked && "border-foreground bg-row-selected",
								option.disabled && "opacity-60",
							)}
						>
							<label
								htmlFor={optionId}
								className={cx(
									"flex cursor-pointer items-start gap-2.5 px-3 py-2.5",
									option.disabled && "cursor-not-allowed",
								)}
							>
								<RadioGroupItem
									id={optionId}
									value={option.value}
									disabled={option.disabled}
									className="mt-0.5 border-border-strong text-foreground shadow-none focus-visible:ring-0 [&_svg]:fill-foreground"
								/>
								{Icon ? (
									<Icon
										aria-hidden
										className="mt-0.5 size-4 shrink-0 text-muted-foreground"
									/>
								) : null}
								<span className="flex min-w-0 flex-col gap-0.5">
									<span className="text-ui font-semibold">{option.title}</span>
									{option.hint ? (
										<span className="text-xs text-muted-foreground">
											{option.hint}
										</span>
									) : null}
								</span>
							</label>
							{checked && option.detail ? (
								<div
									data-choice-detail=""
									className={cx("pr-3 pb-3", Icon ? "pl-16" : "pl-9.5")}
								>
									{option.detail}
								</div>
							) : null}
						</div>
					);
				})}
			</RadioGroup>
		</fieldset>
	);
}

const CHECKED_NEUTRAL =
	"shadow-none data-[state=checked]:border-foreground data-[state=checked]:bg-foreground data-[state=checked]:text-background dark:data-[state=checked]:bg-foreground focus-visible:ring-0 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring focus-visible:outline-solid";

/** Checkbox with its label (acknowledgements, "I saved the backup file"). Never coral (R2). */
export function CheckField({
	id,
	checked,
	onCheckedChange,
	disabled,
	className,
	children,
}: Readonly<{
	id: string;
	checked: boolean;
	onCheckedChange(checked: boolean): void;
	disabled?: boolean;
	className?: string;
	children: ReactNode;
}>) {
	return (
		<div className={cx("inline-flex items-start gap-2", className)}>
			<Checkbox
				id={id}
				checked={checked}
				disabled={disabled}
				onCheckedChange={(next) => onCheckedChange(next === true)}
				className={cx("mt-0.5 border-border-strong", CHECKED_NEUTRAL)}
			/>
			<Label
				htmlFor={id}
				className="text-[13px]/[18px] font-normal peer-disabled:opacity-60"
			>
				<span className="min-w-0">{children}</span>
			</Label>
		</div>
	);
}

export function SwitchField({
	id,
	checked,
	onCheckedChange,
	disabled,
	className,
	children,
}: Readonly<{
	id: string;
	checked: boolean;
	onCheckedChange(checked: boolean): void;
	disabled?: boolean;
	className?: string;
	children: ReactNode;
}>) {
	return (
		<div className={cx("inline-flex items-start gap-2.5", className)}>
			<Switch
				id={id}
				checked={checked}
				disabled={disabled}
				onCheckedChange={onCheckedChange}
				className="mt-px shadow-none data-[state=checked]:bg-foreground focus-visible:ring-0 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring focus-visible:outline-solid dark:data-[state=checked]:**:data-[slot=switch-thumb]:bg-background"
			/>
			<Label htmlFor={id} className="text-[13px]/[18px] font-normal">
				<span className="min-w-0">{children}</span>
			</Label>
		</div>
	);
}
