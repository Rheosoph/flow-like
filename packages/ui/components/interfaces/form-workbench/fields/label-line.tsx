"use client";

import { useTranslation } from "@flow-like/locales";
import { Files, RotateCcw } from "lucide-react";
import type { ReactNode } from "react";
import { cx } from "../../../settings/devices/primitives/tone";
import type { WorkbenchField } from "../contracts";
import type { Bind } from "./bind";
import { labelSize } from "./control-style";

/*
 * The 18 px line above a control (spec M1 anatomy a, canvas): label, 6 px dot when the value differs from its
 * starting value, the "2 files" count, the "Per run" / "Next file" marker, then at the right "Optional" and the
 * hover or focus "Reset" / "Clear". Marker and Reset are `tabIndex -1`: each has a key of its own.
 */

interface LineProps {
	readonly field: WorkbenchField;
	readonly bind: Bind;
	/** The label is a `<label for>` of the control; otherwise a span that the control names itself by. */
	readonly labelFor: boolean;
	/** The field sits inside an object: its hover Reset is scoped to its own frame. */
	readonly nested: boolean;
	readonly onMarker: () => void;
	readonly onReset: () => void;
	readonly extra?: ReactNode;
}

function LabelText({
	field,
	bind,
	labelFor,
}: Readonly<Pick<LineProps, "field" | "bind" | "labelFor">>) {
	const className = cx("min-w-0 truncate font-medium", labelSize(bind.touch));
	if (labelFor)
		return (
			<label
				htmlFor={bind.ids.control}
				id={bind.ids.label}
				className={className}
			>
				{field.label}
			</label>
		);
	return (
		<span id={bind.ids.label} className={className}>
			{field.label}
		</span>
	);
}

function ChangedDot() {
	const { t } = useTranslation("interfaces");
	const text = t("workbench.field.changed", "Changed from its default");
	return (
		<span
			role="img"
			aria-label={text}
			title={text}
			className="size-1.5 shrink-0 rounded-full bg-foreground"
		/>
	);
}

function Count({ count }: Readonly<{ count: number }>) {
	const { t } = useTranslation("interfaces");
	return (
		<span className="shrink-0 text-xs/4 text-muted-foreground">
			{t("workbench.field.count", "{{count}} files", {
				count,
				defaultValue_one: "{{count}} file",
			})}
		</span>
	);
}

const MARKER_BODY =
	"inline-flex h-4.5 shrink-0 items-center gap-1 text-xs/4 font-medium text-muted-foreground";
const MARKER_BUTTON = cx(
	MARKER_BODY,
	"relative rounded-md px-1 hover:bg-row-hover hover:text-ink-2 focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-solid focus-visible:outline-ring after:absolute after:inset-x-0 after:top-1/2 after:h-6 after:-translate-y-1/2 after:content-['']",
);

function MarkerIcon({ next }: Readonly<{ next: boolean }>) {
	const Icon = next ? Files : RotateCcw;
	return <Icon aria-hidden className="size-3.25 shrink-0" />;
}

/** Both words of the marker, the way it is named and titled. */
function useMarkerCopy(field: WorkbenchField, next: boolean) {
	const { t } = useTranslation("interfaces");
	const label = field.label;
	if (next)
		return {
			word: t("workbench.field.nextFile", "Next file"),
			title: t(
				"workbench.field.nextFileTitle",
				"Takes the next file after each run. Click to change.",
			),
			aria: t(
				"workbench.field.nextFileAria",
				"{{label}} takes the next file after each run. Change",
				{ label },
			),
		};
	return {
		word: t("workbench.field.perRun", "Per run"),
		title: t(
			"workbench.field.perRunTitle",
			"Goes back to its starting value after each run. Click to change.",
		),
		aria: t("workbench.field.perRunAria", "{{label}} is per run. Change", {
			label,
		}),
	};
}

function PerRunMarker({
	field,
	kind,
	bind,
	onMarker,
}: Readonly<{
	field: WorkbenchField;
	kind: "perRun" | "nextFile";
	bind: Bind;
	onMarker: () => void;
}>) {
	const next = kind === "nextFile";
	const copy = useMarkerCopy(field, next);
	const icon = <MarkerIcon next={next} />;
	if (bind.touch)
		return (
			<span className={MARKER_BODY}>
				{icon}
				{copy.word}
			</span>
		);
	const compact = bind.markers.optional ? "@max-[200px]/field:hidden" : "";
	return (
		<button
			type="button"
			tabIndex={-1}
			title={copy.title}
			aria-label={copy.aria}
			onClick={onMarker}
			className={MARKER_BUTTON}
		>
			{icon}
			<span className={compact}>{copy.word}</span>
		</button>
	);
}

const RESET_REVEAL =
	"opacity-0 pointer-events-none group-hover/field:pointer-events-auto group-hover/field:opacity-100 group-focus-within/field:pointer-events-auto group-focus-within/field:opacity-100";
const RESET_REVEAL_NESTED =
	"opacity-0 pointer-events-none group-hover/prop:pointer-events-auto group-hover/prop:opacity-100 group-focus-within/prop:pointer-events-auto group-focus-within/prop:opacity-100";
const OPTIONAL_HIDE =
	"group-hover/field:invisible group-focus-within/field:invisible";
const OPTIONAL_HIDE_NESTED =
	"group-hover/prop:invisible group-focus-within/prop:invisible";

/**
 * The scope of a hover or focus reveal, for the shell's quiet focus (S6): while the cursor rests in the
 * first field on open, only a hover reveals (`data-fw-reveal`) or hides (`data-fw-reveal-hide`) these parts.
 */
const revealScope = (nested: boolean) => (nested ? "prop" : "field");

function Optional({
	hidden,
	nested,
	touch,
}: Readonly<{ hidden: boolean; nested: boolean; touch: boolean }>) {
	const { t } = useTranslation("interfaces");
	const hide = nested ? OPTIONAL_HIDE_NESTED : OPTIONAL_HIDE;
	const hoverHides = hidden && !touch;
	return (
		<span
			data-fw-reveal-hide={hoverHides ? revealScope(nested) : undefined}
			className={cx(
				"shrink-0 text-xs/4 text-muted-foreground",
				hidden && (touch ? "invisible" : hide),
			)}
		>
			{t("workbench.field.optional", "Optional")}
		</span>
	);
}

function ResetButton({
	field,
	reset,
	bind,
	nested,
	onReset,
}: Readonly<{
	field: WorkbenchField;
	reset: "reset" | "clear";
	bind: Bind;
	nested: boolean;
	onReset: () => void;
}>) {
	const { t } = useTranslation("interfaces");
	const clear = reset === "clear";
	const word = clear
		? t("workbench.field.clear", "Clear")
		: t("workbench.field.reset", "Reset");
	const aria = clear
		? t("workbench.field.clearAria", "Clear {{label}}", { label: field.label })
		: t("workbench.field.resetAria", "Reset {{label}}", { label: field.label });
	const reveal = nested ? RESET_REVEAL_NESTED : RESET_REVEAL;
	return (
		<button
			type="button"
			tabIndex={-1}
			aria-label={aria}
			data-fw-reveal={bind.touch ? undefined : revealScope(nested)}
			onClick={onReset}
			className={cx(
				"absolute top-0 right-0 h-4.5 min-w-14.5 bg-transparent p-0 text-right text-xs/4 font-medium text-ink-2 hover:underline",
				bind.touch
					? "after:absolute after:-inset-x-1.5 after:-top-4 after:h-9.75 after:content-['']"
					: "",
				bind.touch ? "" : reveal,
			)}
		>
			{word}
		</button>
	);
}

export function LabelLine(props: Readonly<LineProps>) {
	const { field, bind, nested } = props;
	const { markers } = bind;
	return (
		<div className="relative flex min-h-4.5 items-center gap-1.5">
			<LabelText field={field} bind={bind} labelFor={props.labelFor} />
			{markers.changed ? <ChangedDot /> : null}
			{markers.count === null ? null : <Count count={markers.count} />}
			{markers.perRun === null ? null : (
				<PerRunMarker
					field={field}
					kind={markers.perRun}
					bind={bind}
					onMarker={props.onMarker}
				/>
			)}
			<span className="flex-1" />
			{props.extra}
			{markers.optional ? (
				<Optional
					hidden={markers.reset !== null}
					nested={nested}
					touch={bind.touch}
				/>
			) : null}
			{markers.reset === null ? null : (
				<ResetButton
					field={field}
					reset={markers.reset}
					bind={bind}
					nested={nested}
					onReset={props.onReset}
				/>
			)}
		</div>
	);
}
