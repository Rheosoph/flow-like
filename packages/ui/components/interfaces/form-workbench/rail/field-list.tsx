"use client";

import { useTranslation } from "@flow-like/locales";
import { cx } from "../../../settings/devices/primitives/tone";
import type {
	FieldControlProps,
	FieldKey,
	FieldMarkers,
	OpenList,
	WorkbenchField,
} from "../contracts";
import { FieldControl } from "../fields/field-control";
import { dateAnchorOf } from "../model/dates";
import { enterHint } from "../model/keyboard";
import { recallFor as recallOf } from "../model/recent";
import { splitKey } from "../model/values";
import { AfterRunRow } from "./after-run-row";
import { NO_MARKERS, type RailPartProps, columnsOf } from "./rail-model";
import { useScrollMemory } from "./use-scroll-memory";

export interface FieldListProps extends RailPartProps {
	/** The fields to draw: what the filter keeps. */
	readonly fields: readonly WorkbenchField[];
	readonly index: ReadonlyMap<FieldKey, WorkbenchField>;
	readonly markers: ReadonlyMap<FieldKey, FieldMarkers>;
	/** Names of the FlowPath fields this host cannot fill. */
	readonly blocked: readonly string[];
	/** The viewer's day, `YYYY-MM-DD`. */
	readonly today: string;
	/** Show the after-run row at the end (narrow Inputs pane, once introduced). */
	readonly afterRun: boolean;
	/** The filter keeps nothing. */
	readonly noMatch: boolean;
	readonly onShowAll: () => void;
	/** The top-level field that holds the cursor, or null: the filter keeps it on screen while it does. */
	readonly onHold: (name: string | null) => void;
}

/** The top-level field a focus event happened in; null for focus in a portal (a calendar), which is not a change of field. */
function holderOf(target: EventTarget) {
	const key = (target as HTMLElement).closest<HTMLElement>("[data-rail-field]")
		?.dataset.railField;
	return key === undefined ? null : splitKey(key).name;
}

/** The list a field control may use: its own, or one of its properties'. */
function listOf(list: OpenList | null, field: WorkbenchField) {
	if (!list) return null;
	if (list.key === field.key) return list;
	return field.props.some((prop) => prop.key === list.key) ? list : null;
}

function NoMatch({
	touch,
	onShowAll,
}: Readonly<{ touch: boolean; onShowAll: () => void }>) {
	const { t } = useTranslation("interfaces");
	return (
		<div className="flex flex-col items-start gap-1.5 pt-3 pb-1 text-[13px]/[18px] text-ink-2">
			<span>{t("workbench.rail.filter.none", "No field matches.")}</span>
			<button
				type="button"
				onClick={onShowAll}
				className={cx(
					"rounded-md border border-border bg-card px-2.5 text-[12.5px] font-medium text-foreground hover:bg-row-hover focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ring",
					touch ? "h-11" : "h-7",
				)}
			>
				{t("workbench.rail.filter.showAll", "Show all fields")}
			</button>
		</div>
	);
}

/** What every control shares: the function props of `FieldControlProps`, closed over the state. */
type SharedProps = Pick<
	FieldControlProps,
	| "rail"
	| "markersFor"
	| "recallFor"
	| "dateAnchorFor"
	| "enterHint"
	| "host"
	| "viewer"
	| "layout"
	| "today"
	| "disabled"
	| "actions"
>;

function sharedPropsOf(props: Readonly<FieldListProps>): SharedProps {
	const { state, actions, layout, index, markers, blocked, today } = props;
	return {
		rail: state.rail,
		markersFor: (key) => markers.get(key) ?? NO_MARKERS,
		recallFor: (key) => {
			const field = index.get(key);
			return field ? recallOf(state, field, layout.finePointer) : null;
		},
		dateAnchorFor: (key) => dateAnchorOf(state, key, today),
		enterHint: (key) =>
			enterHint(state.form.fields, state.rail.values, key, blocked),
		host: state.form.host,
		viewer: state.form.viewer,
		layout,
		today,
		disabled: false,
		actions,
	};
}

interface CellProps {
	readonly field: WorkbenchField;
	readonly shared: SharedProps;
	readonly props: Readonly<FieldListProps>;
}

/** One grid cell: a half-width one pairs with its neighbour, a full one takes the row. */
function FieldCell({ field, shared, props }: Readonly<CellProps>) {
	const { state, layout, blocked } = props;
	const columns = columnsOf(field, shared.markersFor(field.key), layout.split);
	return (
		<div
			data-rail-field={field.key}
			data-fw-cell={columns === 1 ? "half" : "full"}
			className={cx("min-w-0", columns === 1 ? "col-span-1" : "col-span-full")}
		>
			<FieldControl
				{...shared}
				field={field}
				blocked={blocked.includes(field.name)}
				list={listOf(state.view.list, field)}
			/>
		</div>
	);
}

/**
 * The inputs: a real `<form noValidate>` whose submit never runs (a lone number or date box would
 * otherwise submit on ↵), the fields in a grid that pairs short ones, and the scroll position kept.
 */
export function FieldList(props: Readonly<FieldListProps>) {
	const { state, actions, layout, fields, afterRun, noMatch } = props;
	const { onShowAll, onHold } = props;
	const { t } = useTranslation("interfaces");
	const scroll = useScrollMemory(
		`${state.form.appId}\u0001${state.form.eventId}\u0001inputs`,
	);
	const shared = sharedPropsOf(props);
	return (
		<div
			ref={scroll.ref}
			onScroll={scroll.onScroll}
			data-fw-over={scroll.over ? "" : undefined}
			className={cx(
				"min-h-0 flex-[0_1_auto] overflow-y-auto border-t [scrollbar-width:thin]",
				scroll.over ? "border-hairline" : "border-transparent",
			)}
		>
			<form
				noValidate
				aria-label={t("workbench.rail.tabs.inputs", "Inputs")}
				onSubmit={(event) => event.preventDefault()}
				onFocus={(event) => {
					const name = holderOf(event.target);
					if (name !== null) onHold(name);
				}}
				onBlur={(event) => {
					if (!event.currentTarget.contains(event.relatedTarget as Node | null))
						onHold(null);
				}}
				className={layout.split ? "px-5 pt-2 pb-5" : "px-4 pt-3 pb-4"}
			>
				<div
					className={cx(
						"grid gap-x-3 gap-y-4",
						layout.split ? "grid-cols-2" : "mx-auto max-w-160 grid-cols-1",
					)}
				>
					{fields.map((field) => (
						<FieldCell
							key={field.key}
							field={field}
							shared={shared}
							props={props}
						/>
					))}
				</div>
				{noMatch ? (
					<NoMatch touch={layout.touch} onShowAll={onShowAll} />
				) : null}
			</form>
			{afterRun ? <AfterRunRow state={state} actions={actions} /> : null}
		</div>
	);
}
