"use client";

import { useTranslation } from "@flow-like/locales";
import {
	BracesIcon,
	CheckIcon,
	LayersIcon,
	LockIcon,
	TriangleAlertIcon,
	XIcon,
} from "lucide-react";
import { type ReactNode, useCallback, useId, useMemo, useState } from "react";
import { IPinType } from "../../lib/schema/flow/board";
import { IValueType, IVariableType } from "../../lib/schema/flow/variable";
import {
	Badge,
	Button,
	Dialog,
	DialogBody,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	Input,
	Label,
	Switch,
	Textarea,
} from "../ui";
import {
	type PinOptionSections,
	type PinOptionsDraft,
	type PinOptionsIssue,
	type PinOptionsResult,
	type PinOptionsTarget,
	appendValues,
	buildPinOptionsResult,
	draftFromPin,
	formatSchema,
	pinOptionSections,
	schemaStatus,
	splitValueDraft,
	validatePinOptionsDraft,
} from "./pin-options-model";
import { typeToColor } from "./utils";
import { GeometrySubtypeSelect } from "./variables/geometry-variable";

export interface PinOptionsDialogPin extends PinOptionsTarget {
	id: string;
	friendly_name: string;
	pin_type: IPinType;
	value_type: IValueType;
}

type Patch = (patch: Partial<PinOptionsDraft>) => void;

export function PinOptionsDialog({
	pin,
	refs,
	open,
	onOpenChange,
	onSave,
}: Readonly<{
	pin: PinOptionsDialogPin;
	refs?: Record<string, string>;
	open: boolean;
	onOpenChange: (open: boolean) => void;
	onSave: (result: PinOptionsResult) => void;
}>) {
	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent
				className="gap-0 p-0 outline-none sm:max-w-xl"
				onDoubleClick={(e) => e.stopPropagation()}
				onOpenAutoFocus={(e) => {
					e.preventDefault();
					const content = e.currentTarget as HTMLElement;
					(
						content.querySelector<HTMLElement>("[data-autofocus]") ?? content
					).focus();
				}}
			>
				<PinOptionsForm
					pin={pin}
					refs={refs}
					onCancel={() => onOpenChange(false)}
					onSave={(result) => {
						onSave(result);
						onOpenChange(false);
					}}
				/>
			</DialogContent>
		</Dialog>
	);
}

function PinOptionsForm({
	pin,
	refs,
	onCancel,
	onSave,
}: Readonly<{
	pin: PinOptionsDialogPin;
	refs?: Record<string, string>;
	onCancel: () => void;
	onSave: (result: PinOptionsResult) => void;
}>) {
	const { t } = useTranslation("flow");
	const sections = useMemo(() => pinOptionSections(pin), [pin]);
	const [draft, setDraft] = useState(() => draftFromPin(pin, refs));
	const patch = useCallback<Patch>(
		(next) => setDraft((current) => ({ ...current, ...next })),
		[],
	);
	const issues = useMemo(
		() => validatePinOptionsDraft(draft, sections),
		[draft, sections],
	);

	const typeLabel =
		pin.value_type === IValueType.Normal
			? pin.data_type
			: `${pin.value_type}<${pin.data_type}>`;

	return (
		<>
			<DialogHeader className="border-b px-6 py-4 pr-12">
				<DialogTitle className="flex min-w-0 items-center gap-2 text-base">
					<span
						className="size-2.5 shrink-0 rounded-full"
						style={{ backgroundColor: typeToColor(pin.data_type) }}
					/>
					<span className="truncate">{pin.friendly_name}</span>
				</DialogTitle>
				<DialogDescription className="flex flex-wrap items-center gap-x-1.5 text-xs">
					<span>{t("pinOptionsHeading", "Pin options")}</span>
					<span aria-hidden>·</span>
					<span>
						{pin.pin_type === IPinType.Input
							? t("pinOptionsInputPin", "Input")
							: t("pinOptionsOutputPin", "Output")}
					</span>
					<span aria-hidden>·</span>
					<code className="font-mono">{typeLabel}</code>
				</DialogDescription>
			</DialogHeader>

			<DialogBody className="space-y-6 px-6 py-5">
				{sections.validValues && (
					<AllowedValuesSection draft={draft} patch={patch} />
				)}
				{(sections.range || sections.step) && (
					<NumberLimitsSection
						draft={draft}
						patch={patch}
						sections={sections}
						issues={issues}
					/>
				)}
				{sections.schema && (
					<SchemaSection pin={pin} refs={refs} draft={draft} patch={patch} />
				)}
				{(sections.sensitive || sections.valueShape) && (
					<BehaviorSection draft={draft} patch={patch} sections={sections} />
				)}
			</DialogBody>

			<DialogFooter className="border-t px-6 py-3">
				<Button variant="ghost" onClick={onCancel}>
					{t("cancel", "Cancel")}
				</Button>
				<Button
					disabled={issues.length > 0}
					onClick={() => onSave(buildPinOptionsResult(pin, draft, refs))}
				>
					{t("save", "Save")}
				</Button>
			</DialogFooter>
		</>
	);
}

function OptionSection({
	title,
	htmlFor,
	aside,
	hint,
	children,
}: Readonly<{
	title: string;
	htmlFor?: string;
	aside?: ReactNode;
	hint?: ReactNode;
	children: ReactNode;
}>) {
	return (
		<section className="space-y-2">
			<div className="flex min-h-6 items-center justify-between gap-3">
				<Label
					htmlFor={htmlFor}
					className="text-xs font-medium uppercase tracking-wide text-muted-foreground"
				>
					{title}
				</Label>
				{aside}
			</div>
			{children}
			{hint && <div className="text-xs text-muted-foreground">{hint}</div>}
		</section>
	);
}

function IssueText({ children }: Readonly<{ children: ReactNode }>) {
	return (
		<p className="flex items-center gap-1.5 text-xs text-destructive">
			<TriangleAlertIcon className="size-3.5 shrink-0" />
			{children}
		</p>
	);
}

function AllowedValuesSection({
	draft,
	patch,
}: Readonly<{ draft: PinOptionsDraft; patch: Patch }>) {
	const { t } = useTranslation("flow");
	const id = useId();
	return (
		<OptionSection
			title={t("pinOptionsAllowedValues", "Allowed values")}
			htmlFor={id}
			aside={
				draft.validValues.length > 0 && (
					<Button
						variant="ghost"
						size="sm"
						className="h-6 px-2 text-xs text-muted-foreground"
						onClick={() => patch({ validValues: [] })}
					>
						{t("pinOptionsClearValues", "Clear")}
					</Button>
				)
			}
			hint={t(
				"pinOptionsAllowedValuesHint",
				"Press Enter or comma to add a value. The pin then offers only these values in a dropdown.",
			)}
		>
			<ValueChipsInput
				id={id}
				values={draft.validValues}
				pending={draft.pendingValue}
				onChange={({ values, pending }) =>
					patch({ validValues: values, pendingValue: pending })
				}
			/>
		</OptionSection>
	);
}

export function ValueChipsInput({
	id,
	values,
	pending,
	onChange,
}: Readonly<{
	id?: string;
	values: readonly string[];
	pending: string;
	onChange: (next: { values: string[]; pending: string }) => void;
}>) {
	const { t } = useTranslation("flow");
	const commitPending = () => {
		if (pending.trim() === "") return;
		onChange({ values: appendValues(values, [pending]), pending: "" });
	};

	return (
		<div className="flex min-h-9 flex-wrap items-center gap-1.5 rounded-md border border-input bg-transparent px-1.5 py-1.5 shadow-xs transition-[color,box-shadow] focus-within:border-ring focus-within:ring-[3px] focus-within:ring-ring/50 dark:bg-input/30">
			{values.map((value) => (
				<Badge
					key={value}
					variant="secondary"
					className="h-6 max-w-full gap-1 rounded-md pr-0.5 pl-2 font-mono text-xs font-normal"
				>
					<span className="truncate">{value}</span>
					<button
						type="button"
						className="rounded-sm p-0.5 text-muted-foreground hover:bg-foreground/10 hover:text-foreground"
						aria-label={t("pinOptionsRemoveValue", "Remove {{value}}", {
							value,
						})}
						onClick={() =>
							onChange({
								values: values.filter((entry) => entry !== value),
								pending,
							})
						}
					>
						<XIcon className="size-3" />
					</button>
				</Badge>
			))}
			<input
				id={id}
				data-autofocus
				value={pending}
				autoComplete="off"
				spellCheck={false}
				placeholder={
					values.length === 0
						? t("pinOptionsValuePlaceholder", "e.g. low, medium, high")
						: t("pinOptionsAddValue", "Add value…")
				}
				className="h-6 min-w-[8rem] flex-1 bg-transparent px-1 text-sm outline-none placeholder:text-muted-foreground"
				onChange={(e) => {
					const { complete, rest } = splitValueDraft(e.target.value);
					onChange({ values: appendValues(values, complete), pending: rest });
				}}
				onKeyDown={(e) => {
					if (e.nativeEvent.isComposing) return;
					if (e.key === "Enter") {
						e.preventDefault();
						commitPending();
					} else if (
						e.key === "Backspace" &&
						pending === "" &&
						values.length > 0
					) {
						e.preventDefault();
						onChange({ values: values.slice(0, -1), pending });
					}
				}}
				onBlur={commitPending}
			/>
		</div>
	);
}

function NumberLimitsSection({
	draft,
	patch,
	sections,
	issues,
}: Readonly<{
	draft: PinOptionsDraft;
	patch: Patch;
	sections: PinOptionSections;
	issues: readonly PinOptionsIssue[];
}>) {
	const { t } = useTranslation("flow");
	const id = useId();
	const rangeIssue = issues.includes("rangeIncomplete")
		? t(
				"pinOptionsRangeIncomplete",
				"Set both a minimum and a maximum, or leave both empty.",
			)
		: issues.includes("rangeInverted")
			? t(
					"pinOptionsRangeInverted",
					"The minimum must not be larger than the maximum.",
				)
			: null;
	const stepIssue = issues.includes("stepNotPositive")
		? t("pinOptionsStepNotPositive", "The step must be larger than 0.")
		: null;

	return (
		<OptionSection
			title={t("pinOptionsLimits", "Limits")}
			hint={
				rangeIssue || stepIssue ? (
					<div className="space-y-1">
						{rangeIssue && <IssueText>{rangeIssue}</IssueText>}
						{stepIssue && <IssueText>{stepIssue}</IssueText>}
					</div>
				) : (
					t(
						"pinOptionsLimitsHint",
						"Number fields for this pin stay within these bounds.",
					)
				)
			}
		>
			<div
				className={`grid gap-3 ${sections.range && sections.step ? "grid-cols-3" : "grid-cols-2"}`}
			>
				{sections.range && (
					<>
						<NumberField
							id={`${id}-min`}
							label={t("pinOptionsMinimum", "Minimum")}
							value={draft.min}
							invalid={rangeIssue !== null}
							onChange={(min) => patch({ min })}
						/>
						<NumberField
							id={`${id}-max`}
							label={t("pinOptionsMaximum", "Maximum")}
							value={draft.max}
							invalid={rangeIssue !== null}
							onChange={(max) => patch({ max })}
						/>
					</>
				)}
				{sections.step && (
					<NumberField
						id={`${id}-step`}
						label={t("step", "Step")}
						value={draft.step}
						invalid={stepIssue !== null}
						onChange={(step) => patch({ step })}
					/>
				)}
			</div>
		</OptionSection>
	);
}

function NumberField({
	id,
	label,
	value,
	invalid,
	onChange,
}: Readonly<{
	id: string;
	label: string;
	value: string;
	invalid: boolean;
	onChange: (value: string) => void;
}>) {
	const { t } = useTranslation("flow");
	return (
		<div className="space-y-1.5">
			<Label htmlFor={id} className="text-xs font-normal">
				{label}
			</Label>
			<Input
				id={id}
				type="number"
				inputMode="decimal"
				className="h-8"
				value={value}
				placeholder={t("pinOptionsNoLimit", "No limit")}
				aria-invalid={invalid}
				onChange={(e) => onChange(e.target.value)}
			/>
		</div>
	);
}

function SchemaSection({
	pin,
	refs,
	draft,
	patch,
}: Readonly<{
	pin: PinOptionsDialogPin;
	refs?: Record<string, string>;
	draft: PinOptionsDraft;
	patch: Patch;
}>) {
	const { t } = useTranslation("flow");
	const id = useId();
	const status = schemaStatus(draft.schema);
	const isGeometry = pin.data_type === IVariableType.Geometry;
	const formatted = useMemo(() => formatSchema(draft.schema), [draft.schema]);

	return (
		<OptionSection
			title={t("schema", "Schema")}
			htmlFor={isGeometry ? undefined : id}
			aside={
				!isGeometry && (
					<div className="flex items-center gap-1.5">
						{status.kind === "json" && formatted !== draft.schema && (
							<Button
								variant="ghost"
								size="sm"
								className="h-6 px-2 text-xs text-muted-foreground"
								onClick={() => patch({ schema: formatted })}
							>
								{t("pinOptionsFormatSchema", "Format")}
							</Button>
						)}
						<SchemaStatusBadge status={status} />
					</div>
				)
			}
		>
			{isGeometry ? (
				<GeometrySubtypeSelect
					schema={draft.schema || null}
					refs={refs}
					onChange={(schema) => patch({ schema: schema ?? "" })}
				/>
			) : (
				<Textarea
					id={id}
					value={draft.schema}
					spellCheck={false}
					aria-invalid={status.kind === "invalid"}
					placeholder={t(
						"pinOptionsSchemaPlaceholder",
						"A JSON Schema, or a schema identifier such as my.schema.Identifier",
					)}
					className="max-h-72 min-h-20 overflow-auto font-mono text-xs leading-relaxed md:text-xs"
					onChange={(e) => patch({ schema: e.target.value })}
				/>
			)}
			<div className="rounded-lg border">
				<OptionToggle
					id={`${id}-enforce`}
					icon={<BracesIcon />}
					label={t("pinOptionsEnforceSchema", "Enforce schema")}
					description={t(
						"pinOptionsEnforceSchemaHint",
						"Only allow connections to pins whose schema matches this one.",
					)}
					checked={draft.enforceSchema}
					disabled={!draft.enforceSchema && status.kind === "empty"}
					onCheckedChange={(enforceSchema) => patch({ enforceSchema })}
				/>
			</div>
		</OptionSection>
	);
}

function SchemaStatusBadge({
	status,
}: Readonly<{ status: ReturnType<typeof schemaStatus> }>) {
	const { t } = useTranslation("flow");
	if (status.kind === "empty") return null;
	if (status.kind === "invalid")
		return (
			<Badge variant="destructive" className="gap-1 text-[11px]">
				<TriangleAlertIcon className="size-3" />
				{t("pinOptionsInvalidJson", "Invalid JSON")}
			</Badge>
		);
	if (status.kind === "identifier")
		return (
			<Badge variant="outline" className="text-[11px] font-normal">
				{t("pinOptionsSchemaIdentifier", "Identifier")}
			</Badge>
		);
	return (
		<Badge
			variant="outline"
			className="max-w-56 gap-1 text-[11px] font-normal"
			title={status.title ?? undefined}
		>
			<CheckIcon className="size-3 text-emerald-500" />
			<span className="truncate">
				{status.title ?? t("pinOptionsJsonSchema", "JSON Schema")}
			</span>
		</Badge>
	);
}

function BehaviorSection({
	draft,
	patch,
	sections,
}: Readonly<{
	draft: PinOptionsDraft;
	patch: Patch;
	sections: PinOptionSections;
}>) {
	const { t } = useTranslation("flow");
	const id = useId();
	return (
		<OptionSection title={t("pinOptionsBehavior", "Behavior")}>
			<div className="divide-y rounded-lg border">
				{sections.sensitive && (
					<OptionToggle
						id={`${id}-sensitive`}
						icon={<LockIcon />}
						label={t("sensitive", "Sensitive")}
						description={t(
							"pinOptionsSensitiveHint",
							"Mask the value in the editor and in board diffs.",
						)}
						checked={draft.sensitive}
						onCheckedChange={(sensitive) => patch({ sensitive })}
					/>
				)}
				{sections.valueShape && (
					<OptionToggle
						id={`${id}-shape`}
						icon={<LayersIcon />}
						label={t("pinOptionsLockValueShape", "Lock value shape")}
						description={t(
							"pinOptionsLockValueShapeHint",
							"Only connect pins with the same shape: single value, array, set or map.",
						)}
						checked={draft.enforceValueShape}
						onCheckedChange={(enforceValueShape) =>
							patch({ enforceValueShape })
						}
					/>
				)}
			</div>
		</OptionSection>
	);
}

function OptionToggle({
	id,
	icon,
	label,
	description,
	checked,
	disabled,
	onCheckedChange,
}: Readonly<{
	id: string;
	icon: ReactNode;
	label: string;
	description: string;
	checked: boolean;
	disabled?: boolean;
	onCheckedChange: (checked: boolean) => void;
}>) {
	return (
		<div className="flex items-start justify-between gap-4 px-3 py-2.5">
			<div className="flex min-w-0 gap-2.5">
				<span className="mt-0.5 text-muted-foreground [&_svg]:size-4">
					{icon}
				</span>
				<div className="space-y-0.5">
					<Label htmlFor={id} className="text-sm font-medium">
						{label}
					</Label>
					<p className="text-xs text-muted-foreground">{description}</p>
				</div>
			</div>
			<Switch
				id={id}
				checked={checked}
				disabled={disabled}
				onCheckedChange={onCheckedChange}
			/>
		</div>
	);
}
