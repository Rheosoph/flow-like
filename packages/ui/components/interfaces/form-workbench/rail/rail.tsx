"use client";

import {
	type KeyboardEvent,
	memo,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { cx } from "../../../settings/devices/primitives/tone";
import type { RailProps, WorkbenchField } from "../contracts";
import { blockedNames } from "../model/validate";
import { sameButStreamedOutput } from "../session/state";
import { FieldList } from "./field-list";
import { FilterRow } from "./filter-row";
import { FIELD_FOCUS_SELECTOR } from "./focus";
import { PresetSaveDialog } from "./preset-save-dialog";
import { RailHead } from "./rail-head";
import {
	anyFieldDiffers,
	changedSet,
	fieldIndex,
	fieldsMatching,
	filterCounts,
	focusOwnerName,
	isFiltering,
	localDay,
	markerMap,
	railShapeOf,
} from "./rail-model";
import { RunsList } from "./runs-list";

const MODIFIER_KEYS: ReadonlySet<string> = new Set([
	"Shift",
	"Control",
	"Alt",
	"Meta",
	"CapsLock",
]);

/**
 * The rail column above the dock (desktop) and the Inputs pane (narrow): the head with its Presets
 * button and Inputs | Runs switch, the filter row of a large form, then the field list or the Runs list.
 * A pure renderer of the session state; every change goes through `actions`.
 */
function RailView({ state, actions, layout }: Readonly<RailProps>) {
	const root = useRef<HTMLDivElement | null>(null);
	const { form, rail, runs, memory } = state;
	const { fields } = form;
	const shape = railShapeOf(state, layout);
	const index = useMemo(() => fieldIndex(fields), [fields]);
	const markers = useMemo(
		() => markerMap({ form, rail, runs, memory }, index),
		[form, rail, runs, memory, index],
	);
	const changed = useMemo(
		() => changedSet({ form, rail, memory }),
		[form, rail, memory],
	);
	const blocked = useMemo(
		() => blockedNames(fields, form.host),
		[fields, form.host],
	);
	const [held, setHeld] = useState<string | null>(null);
	const filter = rail.filter;
	const filtering = shape.filter && isFiltering(filter);
	const matching = filtering ? fieldsMatching(fields, filter, changed) : fields;
	const owner = filtering ? focusOwnerName(state) : null;
	const forced =
		owner !== null &&
		!matching.some((field) => field.name === owner) &&
		fields.some((field) => field.name === owner);
	const kept = (field: WorkbenchField) =>
		matching.includes(field) || field.name === owner || field.name === held;
	const visible = filtering ? fields.filter(kept) : fields;

	useEffect(() => {
		if (forced) actions.setFilter({ query: "", chip: "all" });
	}, [forced, actions]);

	const focusFirst = () =>
		root.current?.querySelector<HTMLElement>(FIELD_FOCUS_SELECTOR)?.focus();
	const onKeyDownCapture = (event: KeyboardEvent<HTMLDivElement>) => {
		if (
			state.view.message?.message.kind === "start" &&
			!MODIFIER_KEYS.has(event.key)
		)
			actions.railKey();
	};

	if (fields.length === 0) return null;
	return (
		<div
			ref={root}
			onKeyDownCapture={onKeyDownCapture}
			data-fw-tab={shape.tab}
			className={cx(
				"flex min-h-0 min-w-0 flex-col",
				shape.tab === "runs" ? "flex-1" : "flex-[0_1_auto]",
			)}
		>
			<RailHead
				state={state}
				actions={actions}
				layout={layout}
				shape={shape}
				differs={anyFieldDiffers(fields, markers)}
			/>
			{shape.filter ? (
				<div
					className={cx(
						"flex-none",
						layout.split ? "px-5" : "mx-auto w-full max-w-2xl px-4",
					)}
				>
					<FilterRow
						filter={filter}
						counts={filterCounts(fields, changed)}
						layout={layout}
						setFilter={(next) => actions.setFilter(next)}
						focusFirst={focusFirst}
					/>
				</div>
			) : null}
			{shape.tab === "inputs" ? (
				<FieldList
					state={state}
					actions={actions}
					layout={layout}
					fields={visible}
					index={index}
					markers={markers}
					blocked={blocked}
					today={localDay()}
					afterRun={shape.afterRun}
					noMatch={filtering && visible.length === 0}
					onShowAll={() => actions.setFilter({ query: "", chip: "all" })}
					onHold={setHeld}
				/>
			) : (
				<RunsList state={state} actions={actions} layout={layout} />
			)}
			<PresetSaveDialog state={state} actions={actions} layout={layout} />
		</div>
	);
}

/** The Runs list previews each run's answer, so it follows every chunk; the inputs never show one. */
const sameRail = (prev: Readonly<RailProps>, next: Readonly<RailProps>) =>
	prev.actions === next.actions &&
	prev.layout === next.layout &&
	prev.routes === next.routes &&
	(railShapeOf(next.state, next.layout).tab === "runs"
		? prev.state === next.state
		: sameButStreamedOutput(prev.state, next.state));

/** The rail renders again for what it shows, not for every chunk of a streamed answer. */
export const Rail = memo(RailView, sameRail);
