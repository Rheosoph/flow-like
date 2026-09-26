"use client";

import { useTranslation } from "@flow-like/locales";
import { AlertTriangle, Check, Loader2, Lock, Pencil, X } from "lucide-react";
import {
	type KeyboardEvent,
	type Ref,
	useCallback,
	useEffect,
	useId,
	useMemo,
	useRef,
	useState,
} from "react";
import { apiErrorMessage } from "../../../lib/api-error";
import { getErrorMessage } from "../../../lib/error-message";
import {
	type EditableProperty,
	type ObjectEditField,
	type PropertyDraft,
	type PropertyDraftError,
	type PropertyEditability,
	type PropertyLockReason,
	StaleObjectError,
	changedProperties,
	draftFromValue,
	isEditableProperty,
	parsePropertyDraft,
} from "../../../lib/ontology-object-edit";
import { cn } from "../../../lib/utils";
import { Alert, AlertDescription } from "../alert";
import { Button } from "../button";
import { Input } from "../input";
import { Switch } from "../switch";
import { TemporalValueEditor } from "../temporal-value-editor";
import { Textarea } from "../textarea";
import { Tooltip, TooltipContent, TooltipTrigger } from "../tooltip";
import { PropertyValue } from "./graph-node-inspector";

const MULTILINE_THRESHOLD = 80;
const EMPTY_DRAFT: PropertyDraft = { text: "", isNull: true };

const DRAFT_ERRORS: ReadonlySet<string> = new Set<PropertyDraftError>([
	"number",
	"integer",
	"unsafeInteger",
	"boolean",
	"temporal",
]);

function isDraftError(error: string): error is PropertyDraftError {
	return DRAFT_ERRORS.has(error);
}

function isMultilineText(text: string): boolean {
	return text.includes("\n") || text.length > MULTILINE_THRESHOLD;
}

function capitalize(value: string): string {
	return value ? value[0].toUpperCase() + value.slice(1) : value;
}

export interface OntologyPropertyEditorProps {
	editability: PropertyEditability;
	draft: PropertyDraft;
	onChange: (draft: PropertyDraft) => void;
	/** Property name, used as the field's accessible label. */
	name?: string;
	disabled?: boolean;
	autoFocus?: boolean;
	/** A draft parse error code, or a message shown as is. */
	error?: PropertyDraftError | string | null;
	/** Enter in a single-line field, or Mod+Enter in a multi-line one. */
	onCommit?: () => void;
	onCancel?: () => void;
	compact?: boolean;
}

function useDraftErrorMessage(
	error: PropertyDraftError | string | null | undefined,
): string | null {
	const { t } = useTranslation("common");
	return useMemo(() => {
		if (!error) return null;
		if (!isDraftError(error)) return error;
		switch (error) {
			case "number":
				return t("enterANumber", "Enter a number");
			case "integer":
				return t("enterAWholeNumber", "Enter a whole number");
			case "unsafeInteger":
				return t(
					"valueTooLargeToEdit",
					"This number is too large to edit here",
				);
			case "boolean":
				return t("enterTrueOrFalse", "Enter true or false");
			case "temporal":
				return t("enterAValidDate", "Enter a valid date");
		}
	}, [error, t]);
}

/** Enter commits a single-line field; a multi-line one needs Mod+Enter. */
function isCommitKey(event: KeyboardEvent<HTMLElement>): boolean {
	if (event.key !== "Enter" || event.nativeEvent.isComposing) return false;
	const tag = (event.target as HTMLElement).tagName;
	return (
		tag === "INPUT" || (tag === "TEXTAREA" && (event.metaKey || event.ctrlKey))
	);
}

interface DraftControlProps {
	draft: PropertyDraft;
	onChange: (draft: PropertyDraft) => void;
	name?: string;
	disabled?: boolean;
	invalid: boolean;
	/** The id of the message that explains why the draft is invalid. */
	describedBy?: string;
	compact?: boolean;
}

function DraftSwitch({
	draft,
	onChange,
	name,
	disabled,
	invalid,
	describedBy,
}: Readonly<DraftControlProps>) {
	const { t } = useTranslation("common");
	return (
		<div className="flex h-8 items-center gap-2">
			<Switch
				checked={!draft.isNull && draft.text === "true"}
				onCheckedChange={(checked) =>
					onChange({ text: String(checked), isNull: false })
				}
				disabled={disabled}
				aria-label={name}
				aria-invalid={invalid || undefined}
				aria-describedby={describedBy}
			/>
			<span className="text-sm text-muted-foreground">
				{draft.isNull ? t("emptyValue", "Empty") : draft.text}
			</span>
		</div>
	);
}

function DraftTemporal({
	draft,
	onChange,
	name,
	disabled,
	invalid,
	describedBy,
	field,
}: Readonly<DraftControlProps & { field: ObjectEditField }>) {
	if (!field.temporal) return null;
	return (
		<TemporalValueEditor
			value={draft.isNull ? "null" : draft.text}
			temporal={field.temporal}
			nullable={field.nullable}
			onChange={(next) =>
				onChange(next === "null" ? EMPTY_DRAFT : { text: next, isNull: false })
			}
			disabled={disabled}
			invalid={invalid}
			aria-label={name}
			aria-describedby={describedBy}
		/>
	);
}

function DraftText({
	draft,
	onChange,
	name,
	disabled,
	invalid,
	describedBy,
	compact,
	multiline,
	inputMode,
}: Readonly<
	DraftControlProps & {
		multiline: boolean;
		inputMode?: "numeric" | "decimal";
	}
>) {
	const { t } = useTranslation("common");
	const shared = {
		value: draft.text,
		placeholder: draft.isNull ? t("emptyValue", "Empty") : undefined,
		disabled,
		"aria-label": name,
		"aria-invalid": invalid || undefined,
		"aria-describedby": describedBy,
	};
	return multiline ? (
		<Textarea
			{...shared}
			className={cn("min-h-20", compact && "text-sm")}
			onChange={(event) =>
				onChange({ text: event.target.value, isNull: false })
			}
		/>
	) : (
		<Input
			{...shared}
			className={cn(compact && "h-8 text-sm")}
			inputMode={inputMode}
			onChange={(event) =>
				onChange({ text: event.target.value, isNull: false })
			}
		/>
	);
}

function DraftControl({
	editability,
	multiline,
	...props
}: Readonly<
	DraftControlProps & { editability: EditableProperty; multiline: boolean }
>) {
	switch (editability.editor) {
		case "boolean":
			return <DraftSwitch {...props} />;
		case "temporal":
			return <DraftTemporal {...props} field={editability.field} />;
		case "integer":
			return <DraftText {...props} multiline={false} inputMode="numeric" />;
		case "number":
			return <DraftText {...props} multiline={false} inputMode="decimal" />;
		case "text":
			return <DraftText {...props} multiline={multiline} />;
	}
}

/** A controlled value field for one ontology property; saving is the caller's. */
export function OntologyPropertyEditor(
	props: Readonly<OntologyPropertyEditorProps>,
) {
	const { editability, draft, onChange, autoFocus, onCommit, onCancel } = props;
	const { t } = useTranslation("common");
	const containerRef = useRef<HTMLDivElement>(null);
	const [multiline] = useState(() => isMultilineText(draft.text));
	const errorMessage = useDraftErrorMessage(props.error);
	const errorId = useId();

	useEffect(() => {
		if (!autoFocus) return;
		containerRef.current
			?.querySelector<HTMLElement>("input, textarea, button")
			?.focus();
	}, [autoFocus]);

	const handleKeyDown = useCallback(
		(event: KeyboardEvent<HTMLDivElement>) => {
			const action =
				event.key === "Escape"
					? onCancel
					: isCommitKey(event)
						? onCommit
						: undefined;
			if (!action) return;
			event.preventDefault();
			event.stopPropagation();
			action();
		},
		[onCancel, onCommit],
	);

	if (!isEditableProperty(editability)) return null;
	const { editor, field } = editability;

	return (
		<div
			ref={containerRef}
			className="min-w-0 space-y-1.5"
			onKeyDown={handleKeyDown}
		>
			<DraftControl
				editability={editability}
				multiline={multiline}
				draft={draft}
				onChange={onChange}
				name={props.name}
				disabled={props.disabled}
				invalid={Boolean(errorMessage)}
				describedBy={errorMessage ? errorId : undefined}
				compact={props.compact}
			/>
			{field.nullable && editor !== "temporal" && (
				<Button
					type="button"
					variant="ghost"
					size="sm"
					className="h-6 px-2 text-xs text-muted-foreground"
					disabled={props.disabled || draft.isNull}
					onClick={() => onChange(EMPTY_DRAFT)}
				>
					{t("setEmpty", "Set empty")}
				</Button>
			)}
			{errorMessage && (
				<p id={errorId} role="alert" className="text-xs text-destructive">
					{errorMessage}
				</p>
			)}
		</div>
	);
}

export interface PropertyLockHintProps {
	reason: PropertyLockReason;
	kind?: string;
	relationshipLabel?: string;
}

export function PropertyLockHint({
	reason,
	kind,
	relationshipLabel,
}: Readonly<PropertyLockHintProps>) {
	const { t } = useTranslation("common");
	const message = useMemo(() => {
		switch (reason) {
			case "identity":
				return t(
					"propertyLockedIdentity",
					"This column identifies the object, so it can't be edited here.",
				);
			case "relationship":
				return t(
					"propertyLockedRelationship",
					"This column links objects together, so it can't be edited here.",
				);
			case "kind":
				return t(
					"propertyLockedKind",
					"{{kind}} values can't be edited here yet.",
					{
						kind: capitalize(kind ?? ""),
					},
				);
			case "unknownType":
				return t(
					"propertyLockedUnknownType",
					"This column's type couldn't be read, so it can't be edited.",
				);
			case "unsafeInteger":
				return t(
					"valueTooLargeToEdit",
					"This number is too large to edit here",
				);
		}
	}, [kind, reason, t]);

	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<button
					type="button"
					aria-label={message}
					className="inline-flex size-5 shrink-0 cursor-default items-center justify-center rounded text-muted-foreground/70 outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
				>
					<Lock className="h-3 w-3" />
				</button>
			</TooltipTrigger>
			<TooltipContent className="max-w-64">
				<p>{message}</p>
				{relationshipLabel && (
					<p className="mt-0.5 font-mono opacity-80">{relationshipLabel}</p>
				)}
			</TooltipContent>
		</Tooltip>
	);
}

export interface PropertyEditButtonProps {
	name: string;
	onClick: () => void;
	disabled?: boolean;
	ref?: Ref<HTMLButtonElement>;
}

export function PropertyEditButton({
	name,
	onClick,
	disabled,
	ref,
}: Readonly<PropertyEditButtonProps>) {
	const { t } = useTranslation("common");
	return (
		<Button
			ref={ref}
			type="button"
			variant="ghost"
			size="icon"
			className="size-6 shrink-0 text-muted-foreground opacity-60 group-hover:opacity-100 focus-visible:opacity-100"
			onClick={onClick}
			disabled={disabled}
			aria-label={t("editName", "Edit {{name}}", { name })}
		>
			<Pencil className="h-3.5 w-3.5" />
		</Button>
	);
}

export interface InlinePropertyEditorProps {
	name: string;
	value: unknown;
	editability: PropertyEditability;
	/** Rejects with StaleObjectError when the stored value moved on. */
	onSave: (value: unknown) => Promise<void>;
	onDone: () => void;
}

/** Edits one property in place and saves it on its own. */
export function InlinePropertyEditor(
	props: Readonly<InlinePropertyEditorProps>,
) {
	const { editability, ...rest } = props;
	return isEditableProperty(editability) ? (
		<InlinePropertyEditorBody editability={editability} {...rest} />
	) : null;
}

function saveErrorMessage(error: unknown): string {
	return apiErrorMessage(error, getErrorMessage(error));
}

function InlinePropertyEditorBody({
	name,
	value,
	editability,
	onSave,
	onDone,
}: Readonly<Omit<InlinePropertyEditorProps, "editability">> & {
	editability: EditableProperty;
}) {
	const { t } = useTranslation("common");
	const [draft, setDraft] = useState(() => draftFromValue(editability, value));
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [stale, setStale] = useState<{ current: unknown } | null>(null);
	const parsed = useMemo(
		() => parsePropertyDraft(editability, draft),
		[editability, draft],
	);

	const save = useCallback(async () => {
		if (saving || !parsed.ok) return;
		if (
			Object.keys(
				changedProperties({ [name]: value }, { [name]: parsed.value }),
			).length === 0
		) {
			onDone();
			return;
		}
		setSaving(true);
		setError(null);
		try {
			await onSave(parsed.value);
		} catch (caught) {
			if (caught instanceof StaleObjectError) {
				setStale({ current: caught.current[name] ?? null });
			} else {
				setStale(null);
				setError(saveErrorMessage(caught));
			}
			setSaving(false);
			return;
		}
		setSaving(false);
		onDone();
	}, [name, onDone, onSave, parsed, saving, value]);

	/** A save in flight keeps the editor open, so its outcome can still be shown. */
	const cancel = useCallback(() => {
		if (!saving) onDone();
	}, [onDone, saving]);

	return (
		<div className="min-w-0 space-y-2">
			<div className="flex min-w-0 items-start gap-1">
				<div className="min-w-0 flex-1">
					<OntologyPropertyEditor
						editability={editability}
						draft={draft}
						onChange={setDraft}
						name={name}
						disabled={saving}
						autoFocus
						error={parsed.ok ? null : parsed.error}
						onCommit={save}
						onCancel={cancel}
						compact
					/>
				</div>
				<Button
					type="button"
					variant="ghost"
					size="icon"
					className="size-8 shrink-0"
					onClick={save}
					disabled={saving || !parsed.ok}
					aria-label={t("saveProperty", "Save")}
				>
					{saving ? (
						<Loader2 className="h-4 w-4 animate-spin" />
					) : (
						<Check className="h-4 w-4" />
					)}
				</Button>
				<Button
					type="button"
					variant="ghost"
					size="icon"
					className="size-8 shrink-0"
					onClick={onDone}
					disabled={saving}
					aria-label={t("cancelEdit", "Cancel")}
				>
					<X className="h-4 w-4" />
				</Button>
			</div>
			{stale && (
				<Alert className="border-amber-500/40 bg-amber-500/10 text-amber-700 dark:text-amber-400">
					<AlertTriangle />
					<AlertDescription className="text-current">
						<p className="text-xs">
							{t(
								"objectChangedWhileEditing",
								"Someone changed this value after you opened it. It now shows the current value; save again to replace it.",
							)}
						</p>
						<div className="w-full min-w-0 space-y-0.5">
							<p className="text-[11px] font-medium opacity-80">
								{t("currentValue", "Current value")}
							</p>
							<div className="min-w-0 text-foreground">
								<PropertyValue value={stale.current} propKey={name} compact />
							</div>
						</div>
					</AlertDescription>
				</Alert>
			)}
			{error && (
				<p
					role="alert"
					className="text-xs text-destructive [overflow-wrap:anywhere]"
				>
					{error}
				</p>
			)}
		</div>
	);
}
