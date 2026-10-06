"use client";

import { useTranslation } from "@flow-like/locales";
import { LoaderCircle, OctagonX, X } from "lucide-react";
import type { KeyboardEvent, ReactNode } from "react";
import { cx } from "../../../settings/devices/primitives/tone";
import type { FileSlot } from "../contracts";
import { formatBytes } from "../run/format";
import { DIFFERS_EDGE, ghostIcon, textButton } from "./control-style";
import { fileIconOf } from "./file-icon";
import { isComposing, isPlain, swallowRepeat } from "./keys";

/*
 * One file in a field, a next-files list or a "Pick again" reminder (spec M2, M3, S4): icon, name in mono, the size
 * once sent or "Sending" / "Waiting" / "Not sent", a × that is no Tab stop, and a 4 px bar while sending.
 */

/** The right part of a row: the size once sent, else the state in words. */
export function useSlotMeta(slot: FileSlot, decimalSign: "." | ",") {
	const { t } = useTranslation("interfaces");
	if (slot.state === "sending")
		return {
			text: t("workbench.field.file.sending", "Sending"),
			critical: false,
		};
	if (slot.state === "waiting")
		return {
			text: t("workbench.field.file.waiting", "Waiting"),
			critical: false,
		};
	if (slot.state === "failed")
		return {
			text: t("workbench.field.file.notSent", "Not sent"),
			critical: true,
		};
	return { text: formatBytes(slot.size, decimalSign), critical: false };
}

export function SlotIcon({ slot }: Readonly<{ slot: FileSlot }>) {
	if (slot.state === "sending")
		return (
			<LoaderCircle
				aria-hidden
				className="size-4 shrink-0 animate-spin text-info motion-reduce:animate-none"
			/>
		);
	if (slot.state === "failed")
		return <OctagonX aria-hidden className="size-4 shrink-0 text-critical" />;
	const Icon = fileIconOf(slot.name);
	return <Icon aria-hidden className="size-4 shrink-0 text-muted-foreground" />;
}

/** A 4 px bar along the bottom of a row that is sending. */
export function ProgressBar({ slot }: Readonly<{ slot: FileSlot }>) {
	if (slot.state !== "sending") return null;
	const percent = Math.round(
		Math.min(1, Math.max(0, slot.progress ?? 0)) * 100,
	);
	return (
		<span
			aria-hidden
			className="absolute inset-x-0 bottom-0 block h-1 bg-muted"
		>
			<span
				className="block h-1 bg-info-solid"
				style={{ width: `${percent}%` }}
			/>
		</span>
	);
}

export const ROW_FOCUS =
	"has-focus-visible:outline-2 has-focus-visible:outline-offset-1 has-focus-visible:outline-solid has-focus-visible:outline-ring";

export const MAIN_BUTTON =
	"flex min-w-0 flex-1 items-center gap-2 self-stretch bg-transparent text-left outline-none";

function TryAgain({
	name,
	touch,
	onRetry,
}: Readonly<{ name: string; touch: boolean; onRetry: () => void }>) {
	const { t } = useTranslation("interfaces");
	return (
		<button
			type="button"
			onClick={onRetry}
			onKeyDown={swallowRepeat}
			aria-label={t(
				"workbench.field.file.tryAgainAria",
				"Try sending {{name}} again",
				{ name },
			)}
			className={textButton(touch)}
		>
			{t("workbench.field.file.tryAgain", "Try again")}
		</button>
	);
}

/** A 28 px × (24 px in a list row); on a touch screen 40 px with a 44 px hit area, so a 44 px row keeps its height. */
const removeSize = (touch: boolean, small: boolean) => {
	if (touch) return "size-10";
	return small ? "size-6" : undefined;
};

export function RemoveButton({
	label,
	touch,
	small,
	onRemove,
}: Readonly<{
	label: string;
	touch: boolean;
	small?: boolean;
	onRemove: () => void;
}>) {
	return (
		<button
			type="button"
			tabIndex={-1}
			aria-label={label}
			onClick={onRemove}
			className={cx(
				ghostIcon(touch, removeSize(touch, small === true)),
				touch && "relative after:absolute after:-inset-0.5 after:content-['']",
			)}
		>
			<X aria-hidden className="size-3.5" />
		</button>
	);
}

export interface AttachedRowProps {
	readonly slot: FileSlot;
	readonly label: string;
	readonly decimalSign: "." | ",";
	readonly touch: boolean;
	readonly differs: boolean;
	readonly invalid: boolean;
	readonly disabled: boolean;
	readonly focus: Readonly<Record<string, string>> | null;
	readonly describedBy: string | undefined;
	/** Replace the file: Space, a click. */
	readonly onReplace: () => void;
	readonly onRemove: () => void;
	readonly onRetry: () => void;
	/** ↵ moves on. */
	readonly onEnter: (event: KeyboardEvent<HTMLButtonElement>) => boolean;
	/** Keys of the row that belong to a list around it (↓ into the next files). */
	readonly onKey?: (event: KeyboardEvent<HTMLButtonElement>) => void;
	/** Shown instead of the name and size while files are dragged over the row. */
	readonly overlay: ReactNode;
}

function useRowName(props: AttachedRowProps, meta: { text: string }) {
	const { t } = useTranslation("interfaces");
	const { slot, label } = props;
	if (meta.text === "")
		return t(
			"workbench.field.file.rowAria",
			"{{label}}: {{name}}. Space replaces it, Delete removes it.",
			{ label, name: slot.name },
		);
	return t(
		"workbench.field.file.rowAriaDetail",
		"{{label}}: {{name}}, {{detail}}. Space replaces it, Delete removes it.",
		{ label, name: slot.name, detail: meta.text },
	);
}

/** An attached file: a button over icon, name and size; ↵ moves on, Space replaces, ⌫ or Delete removes (spec M2). */
export function AttachedRow(props: Readonly<AttachedRowProps>) {
	const { t } = useTranslation("interfaces");
	const { slot, touch } = props;
	const meta = useSlotMeta(slot, props.decimalSign);
	const name = useRowName(props, meta);
	const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
		if (isComposing(event) || props.onEnter(event)) return;
		const remove = event.key === "Backspace" || event.key === "Delete";
		if (remove && isPlain(event)) {
			event.preventDefault();
			props.onRemove();
			return;
		}
		props.onKey?.(event);
	};
	return (
		<div
			className={cx(
				"relative flex items-center gap-1 overflow-hidden rounded-lg border bg-card pr-1 pl-2.5",
				props.invalid
					? "border-critical-line"
					: "border-input hover:border-border-strong",
				touch ? "min-h-11" : "h-9",
				props.differs && DIFFERS_EDGE,
				ROW_FOCUS,
			)}
		>
			<button
				type="button"
				disabled={props.disabled}
				aria-label={name}
				aria-describedby={props.describedBy}
				{...(props.focus ?? {})}
				onClick={props.onReplace}
				onKeyDown={onKeyDown}
				className={MAIN_BUTTON}
			>
				{props.overlay ?? (
					<>
						<SlotIcon slot={slot} />
						<span
							title={slot.name}
							className="min-w-0 flex-1 truncate font-mono text-[12.5px]/4"
						>
							{slot.name}
						</span>
						<span
							className={cx(
								"shrink-0 font-mono text-xs/4 tabular-nums [word-spacing:-0.25em]",
								meta.critical ? "text-critical" : "text-muted-foreground",
							)}
						>
							{meta.text}
						</span>
					</>
				)}
			</button>
			{slot.state === "failed" ? (
				<TryAgain name={slot.name} touch={touch} onRetry={props.onRetry} />
			) : null}
			<RemoveButton
				label={t("workbench.field.file.remove", "Remove {{name}}", {
					name: slot.name,
				})}
				touch={touch}
				onRemove={props.onRemove}
			/>
			<ProgressBar slot={slot} />
		</div>
	);
}

export interface ReminderRowProps {
	readonly slot: FileSlot;
	readonly label: string;
	readonly touch: boolean;
	readonly disabled: boolean;
	readonly focus: Readonly<Record<string, string>> | null;
	readonly describedBy: string | undefined;
	readonly onPick: () => void;
	readonly onRemove: () => void;
}

/** "Pick again" (spec S4): a file of an older run that has to be chosen again. ↵ or Space opens the dialog, ⌫ removes it. */
export function ReminderRow(props: Readonly<ReminderRowProps>) {
	const { t } = useTranslation("interfaces");
	const { slot, touch } = props;
	const word = t("workbench.field.file.pickAgain", "Pick again");
	const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
		if (isComposing(event) || !isPlain(event)) return;
		if (event.key !== "Backspace" && event.key !== "Delete") return;
		event.preventDefault();
		props.onRemove();
	};
	return (
		<div
			title={t(
				"workbench.field.file.pickAgainTitle",
				"Files from earlier runs can be sent again while this page is open.",
			)}
			className={cx(
				"relative flex items-center gap-1 overflow-hidden rounded-lg border border-dashed border-border-strong bg-card pr-1 pl-2.5",
				touch ? "min-h-11" : "h-9",
				ROW_FOCUS,
			)}
		>
			<button
				type="button"
				disabled={props.disabled}
				aria-label={t(
					"workbench.field.file.pickAgainAria",
					"{{label}}: {{name}}. Pick again.",
					{ label: props.label, name: slot.name },
				)}
				aria-describedby={props.describedBy}
				{...(props.focus ?? {})}
				onClick={props.onPick}
				onKeyDown={onKeyDown}
				className={MAIN_BUTTON}
			>
				<SlotIcon slot={slot} />
				<span
					title={slot.name}
					className="min-w-0 flex-1 truncate font-mono text-[12.5px]/4 text-muted-foreground"
				>
					{slot.name}
				</span>
				<span className="shrink-0 text-[12.5px]/4 font-medium text-ink-2">
					{word}
				</span>
			</button>
			<RemoveButton
				label={t(
					"workbench.field.file.removeReminder",
					"Remove the reminder for {{name}}",
					{ name: slot.name },
				)}
				touch={touch}
				onRemove={props.onRemove}
			/>
		</div>
	);
}
