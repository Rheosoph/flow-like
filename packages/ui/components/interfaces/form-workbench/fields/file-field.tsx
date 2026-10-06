"use client";

import {
	type DragEvent,
	type KeyboardEvent,
	type ReactNode,
	type RefObject,
	useEffect,
	useLayoutEffect,
	useRef,
	useState,
} from "react";
import {
	FILE_FIELD_ATTR,
	FILE_INPUT_ATTR,
	FOCUS_ATTR,
	type FileSlot,
} from "../contracts";
import { isEmpty, slotsOf } from "../model/values";
import type { ControlProps } from "./bind";
import { handleEnter } from "./enter";
import { BlockedRow, DropRow, useDropText } from "./file-drop";
import { type DragState, NO_DRAG } from "./file-model";
import { AttachedRow, ReminderRow } from "./file-row";
import { isPlain } from "./keys";
import { type NextFilesProps, NextLine, NextList } from "./next-files";

/*
 * A file field (spec M2, M3, F, S4): the drop row, attached rows, the next files of a one-file field, "Pick again"
 * reminders, the disabled row where this host can send no FlowPath. The hidden `<input type="file">` carries
 * `multiple` on hosts with next files; ⌘O in the shell clicks it through `data-fw-file-input`.
 */

const FOCUS_SELECTOR = `[${FOCUS_ATTR}]`;

function focusFirst(container: RefObject<HTMLElement | null>) {
	container.current?.querySelector<HTMLElement>(FOCUS_SELECTOR)?.focus();
}

/** The cursor goes back to the field after a pick or a removal changed its rows, never to the page (spec M2). */
function useKeepFocus(
	container: RefObject<HTMLElement | null>,
	signature: string,
) {
	const requested = useRef(0);
	// biome-ignore lint/correctness/useExhaustiveDependencies: runs when the rows change
	useLayoutEffect(() => {
		const at = requested.current;
		requested.current = 0;
		if (at !== 0 && Date.now() - at < 1500) focusFirst(container);
	}, [signature]);
	return () => {
		requested.current = Date.now();
	};
}

function dragStateOf(event: DragEvent): DragState | null {
	const transfer = event.dataTransfer;
	if (!transfer || !Array.from(transfer.types).includes("Files")) return null;
	const items = transfer.items ? transfer.items.length : 0;
	return { over: true, count: items > 0 ? items : null };
}

/** Files dragged over the field: the dashed border and the text of what dropping does; a drop picks them. */
function useDragDrop(enabled: boolean, onFiles: (files: File[]) => void) {
	const [drag, setDrag] = useState<DragState>(NO_DRAG);
	const over = (event: DragEvent) => {
		const next = enabled ? dragStateOf(event) : null;
		if (!next) return;
		event.preventDefault();
		if (event.dataTransfer) event.dataTransfer.dropEffect = "copy";
		setDrag(next);
	};
	return {
		drag,
		handlers: {
			onDragEnter: over,
			onDragOver: over,
			onDragLeave: (event: DragEvent<HTMLElement>) => {
				const to = event.relatedTarget as Node | null;
				if (!to || !event.currentTarget.contains(to)) setDrag(NO_DRAG);
			},
			onDrop: (event: DragEvent) => {
				if (!enabled) return;
				event.preventDefault();
				setDrag(NO_DRAG);
				const files = Array.from(event.dataTransfer?.files ?? []);
				if (files.length > 0) onFiles(files);
			},
		},
	};
}

/** The hidden input and what picking does: replace for a one-file field, append (or swap one row) for several. */
function useFileInput(
	props: ControlProps,
	keep: () => void,
	container: RefObject<HTMLElement | null>,
) {
	const { field, actions } = props;
	const input = useRef<HTMLInputElement>(null);
	const replacing = useRef<FileSlot | null>(null);
	useEffect(() => {
		const node = input.current;
		if (!node) return;
		const onCancel = () => focusFirst(container);
		node.addEventListener("cancel", onCancel);
		return () => node.removeEventListener("cancel", onCancel);
	}, [container]);
	const onFiles = (files: File[]) => {
		if (files.length === 0) return;
		keep();
		if (field.kind === "file") actions.pickFiles(field.name, files, "replace");
		else {
			if (replacing.current)
				actions.removeFile(field.name, replacing.current.id);
			actions.pickFiles(field.name, files, "append");
		}
		replacing.current = null;
	};
	return {
		input,
		onFiles,
		open(slot?: FileSlot) {
			replacing.current = slot ?? null;
			input.current?.click();
		},
	};
}

type NextRefs = Pick<NextFilesProps, "toggle" | "rows" | "onGone">;

const listOpenFor = (props: ControlProps) =>
	props.list?.kind === "nextFiles" && props.list.key === props.field.key;

function nextFilesOf(
	props: ControlProps,
	refs: NextRefs,
): NextFilesProps | null {
	const { field, rail, actions, viewer, layout } = props;
	const next = rail.nextFiles[field.name] ?? [];
	if (field.kind !== "file" || props.blocked || next.length === 0) return null;
	return {
		field,
		next,
		leftOut: rail.leftOut[field.name] ?? [],
		open: listOpenFor(props),
		touch: layout.touch,
		decimalSign: viewer.decimalSign,
		listId: `${props.bind.ids.control}-next`,
		actions,
		...refs,
	};
}

interface View {
	readonly props: ControlProps;
	readonly slots: readonly FileSlot[];
	readonly many: boolean;
	readonly describedBy: string | undefined;
	readonly keep: () => void;
	readonly file: ReturnType<typeof useFileInput>;
	readonly drag: DragState;
	readonly replaceText: string | null;
	readonly oneRunEach: boolean;
	readonly next: NextFilesProps | null;
	readonly rows: RefObject<(HTMLElement | null)[]>;
}

function replaceNode(text: string | null): ReactNode {
	return text === null ? null : (
		<span className="min-w-0 flex-1 text-[13px]">{text}</span>
	);
}

function Rows({ view }: Readonly<{ view: View }>) {
	const { props, slots, many, describedBy, keep, file } = view;
	const { field, actions, bind, disabled, viewer } = props;
	const focus = many ? null : bind.focus;
	const remove = (slot: FileSlot) => {
		keep();
		actions.removeFile(field.name, slot.id);
	};
	const onKey = (event: KeyboardEvent<HTMLButtonElement>) => {
		if (!view.next?.open || event.key !== "ArrowDown" || !isPlain(event))
			return;
		event.preventDefault();
		view.rows.current[0]?.focus();
	};
	return slots.map((slot) =>
		slot.state === "reminder" ? (
			<ReminderRow
				key={slot.id}
				slot={slot}
				label={field.label}
				touch={bind.touch}
				disabled={disabled}
				focus={focus}
				describedBy={describedBy}
				onPick={() => file.open(slot)}
				onRemove={() => remove(slot)}
			/>
		) : (
			<AttachedRow
				key={many ? slot.id : "current"}
				slot={slot}
				label={field.label}
				decimalSign={viewer.decimalSign}
				touch={bind.touch}
				differs={bind.markers.differs}
				invalid={bind.invalid}
				disabled={disabled}
				focus={focus}
				describedBy={describedBy}
				onReplace={() => file.open(slot)}
				onRemove={() => remove(slot)}
				onRetry={() => actions.retryFile(slot.id)}
				onEnter={(event) => handleEnter(event, field.key, actions)}
				onKey={onKey}
				overlay={many ? null : replaceNode(view.replaceText)}
			/>
		),
	);
}

function Tail({ view }: Readonly<{ view: View }>) {
	const { props, slots, many, next } = view;
	const { field, bind, disabled, blocked } = props;
	const showDrop = !blocked && (many || slots.length === 0);
	return (
		<>
			{blocked ? <BlockedRow id={bind.ids.hint} touch={bind.touch} /> : null}
			{showDrop ? (
				<DropRow
					label={field.label}
					many={many}
					touch={bind.touch}
					invalid={bind.invalid}
					disabled={disabled}
					drag={view.drag}
					oneRunEach={view.oneRunEach}
					focus={bind.focus}
					describedBy={view.describedBy}
					onOpen={() => view.file.open()}
				/>
			) : null}
			{next ? <NextLine props={next} /> : null}
			{next ? (
				<div className="-mt-1">
					<NextList props={next} />
				</div>
			) : null}
		</>
	);
}

export function FileField(props: ControlProps) {
	const { field, rail, bind, disabled, host, blocked } = props;
	const many = field.kind === "files";
	const value = rail.values[field.name];
	const slots = slotsOf(value);
	const container = useRef<HTMLFieldSetElement>(null);
	const toggle = useRef<HTMLButtonElement>(null);
	const rows = useRef<(HTMLElement | null)[]>([]);
	const queued = (rail.nextFiles[field.name] ?? []).length;
	const keep = useKeepFocus(
		container,
		`${slots.map((slot) => slot.id).join()}|${queued}`,
	);
	const file = useFileInput(props, keep, container);
	const { drag, handlers } = useDragDrop(!blocked && !disabled, file.onFiles);
	const oneRunEach = !many && host.nextFiles;
	const replaceText = useDropText(drag, oneRunEach, true);
	const view: View = {
		props,
		slots,
		many,
		describedBy: bind.describedBy(blocked),
		keep,
		file,
		drag,
		replaceText,
		oneRunEach,
		rows,
		next: nextFilesOf(props, {
			toggle,
			rows,
			onGone: () => focusFirst(container),
		}),
	};
	return (
		<fieldset
			ref={container}
			aria-labelledby={bind.ids.label}
			aria-describedby={view.describedBy}
			aria-disabled={blocked || undefined}
			{...{ [FILE_FIELD_ATTR]: field.name }}
			data-empty={isEmpty(field, value) ? "" : undefined}
			{...handlers}
			className="m-0 flex min-w-0 flex-col gap-1.5 border-0 p-0"
		>
			<Rows view={view} />
			<Tail view={view} />
			<input
				ref={file.input}
				type="file"
				tabIndex={-1}
				aria-hidden
				multiple={many || host.nextFiles}
				{...{ [FILE_INPUT_ATTR]: field.name }}
				onChange={(event) => {
					file.onFiles(Array.from(event.target.files ?? []));
					event.target.value = "";
				}}
				className="sr-only"
			/>
		</fieldset>
	);
}
