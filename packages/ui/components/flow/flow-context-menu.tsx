import { useTranslation } from "@flow-like/locales";
import { createId } from "@paralleldrive/cuid2";
import { Slot } from "@radix-ui/react-slot";
import {
	type PointerEvent,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { boardModules } from "../../lib/flow-modules";
import type { IBoard } from "../../lib/schema/flow/board";
import type { INode } from "../../lib/schema/flow/node";
import { type IPin, IPinType, IVariableType } from "../../lib/schema/flow/pin";
import type { IVariable } from "../../lib/schema/flow/variable";
import { Popover, PopoverAnchor } from "../ui/popover";
import { NodePalette, type NodePaletteHandlers } from "./node-palette";
import {
	type PaletteEntry,
	bindVariableNode,
	boardInputsKey,
	buildDropIndex,
	buildPaletteEntries,
	buildPaletteModel,
	collectBoardInputs,
	isFunctionReferencePin,
} from "./node-palette-model";

const LONG_PRESS_MS = 700;

function useStableByKey<T>(value: T, keyOf: (value: T) => string): T {
	const key = keyOf(value);
	const ref = useRef({ key, value });
	if (ref.current.key !== key) ref.current = { key, value };
	return ref.current.value;
}

const isTouchOrPen = (event: PointerEvent) => event.pointerType !== "mouse";

export function FlowContextMenu({
	nodes,
	board,
	refs,
	children,
	droppedPin,
	currentLayerId,
	selectionCount = 0,
	movableSelectionCount = 0,
	onPlaceholder,
	onNodePlace,
	onCommentPlace,
	onCreateVariable,
	onEditSelectionAsFlowScript,
	onMoveSelectionToModule,
	onPingHere,
	onDiscussInChat,
	onClose,
}: Readonly<{
	nodes: INode[];
	board: IBoard | undefined;
	refs: { [key: string]: string };
	children: React.ReactNode;
	droppedPin?: IPin;
	currentLayerId?: string;
	/** Selected flow nodes on the canvas; gates the scoped-FlowScript action. */
	selectionCount?: number;
	/** Selected nodes *and* comments — what a move to another file would carry. */
	movableSelectionCount?: number;
	onPlaceholder: (name: string) => void;
	onNodePlace: (node: INode) => void;
	onCommentPlace: () => void;
	onCreateVariable?: (variable: IVariable) => void;
	/** Present only when the backend supports selection-scoped FlowScript editing. */
	onEditSelectionAsFlowScript?: () => void;
	/** Absent while a board version is being viewed — a read-only board takes no edits. */
	onMoveSelectionToModule?: (target: string | null) => void;
	/** Drop a transient "look here" marker for peers at the click position; absent offline. */
	onPingHere?: () => void;
	/** Open the board chat with a reference to the selected node; absent offline. */
	onDiscussInChat?: () => void;
	onClose: () => void;
}>) {
	const { t } = useTranslation("flow");
	const [open, setOpen] = useState(false);
	const [point, setPoint] = useState({ x: 0, y: 0 });
	const [session, setSession] = useState(0);
	const longPressRef = useRef<ReturnType<typeof setTimeout> | undefined>(
		undefined,
	);
	const refsRef = useRef(refs);
	refsRef.current = refs;
	const everOpened = session > 0;

	// The board object is replaced on every edit; only these slices feed the
	// palette, so keying on their content keeps the catalog sort + search index
	// from being rebuilt after every mutation.
	const rawInputs = useMemo(
		() => collectBoardInputs(board, currentLayerId),
		[board, currentLayerId],
	);
	const boardInputs = useStableByKey(rawInputs, boardInputsKey);

	// Built on first open: most board sessions never right-click the canvas.
	const model = useMemo(() => {
		if (!everOpened) return null;
		return buildPaletteModel(
			buildPaletteEntries(nodes, boardInputs, {
				call: (name) => t("callName", "Call {{name}}", { name }),
				get: (name) => t("getName", "Get {{name}}", { name }),
				set: (name) => t("setName", "Set {{name}}", { name }),
			}),
		);
	}, [everOpened, nodes, boardInputs, t]);

	const liveDrop = useMemo(
		() =>
			open && model && droppedPin
				? {
						pin: droppedPin,
						fits: buildDropIndex(model.entries, droppedPin, refsRef.current),
					}
				: undefined,
		[open, model, droppedPin],
	);

	const modules = useMemo(() => boardModules(board?.layers), [board?.layers]);

	const openAt = useCallback((x: number, y: number) => {
		setSession((value) => value + 1);
		setPoint({ x, y });
		setOpen(true);
	}, []);

	const close = useCallback(() => {
		setOpen(false);
		onClose();
	}, [onClose]);

	const clearLongPress = useCallback(
		() => clearTimeout(longPressRef.current),
		[],
	);
	useEffect(() => clearLongPress, [clearLongPress]);

	const createVariableFromPin = useCallback(
		(pin: IPin) => {
			if (!onCreateVariable) return;
			const variable: IVariable = {
				id: createId(),
				name: pin.friendly_name || pin.name,
				data_type: pin.data_type,
				value_type: pin.value_type,
				exposed: false,
				secret: false,
				editable: true,
				schema: pin.schema ? (refsRef.current[pin.schema] ?? pin.schema) : null,
				default_value: pin.default_value ?? null,
			};
			onCreateVariable(variable);
			const writes = pin.pin_type === IPinType.Output;
			const base = nodes.find(
				(node) => node.name === (writes ? "variable_set" : "variable_get"),
			);
			if (!base) return;
			onNodePlace(
				bindVariableNode(
					base,
					variable,
					writes
						? t("setName", "Set {{name}}", { name: variable.name })
						: t("getName", "Get {{name}}", { name: variable.name }),
				),
			);
		},
		[onCreateVariable, onNodePlace, nodes, t],
	);

	const handlers = useMemo<NodePaletteHandlers>(() => {
		const thenClose =
			<A extends unknown[]>(run: (...args: A) => unknown) =>
			(...args: A) => {
				void run(...args);
				close();
			};
		const eventNode = nodes.find((node) => node.name === "events_simple");
		const canCreateVariable =
			droppedPin &&
			onCreateVariable &&
			droppedPin.data_type !== IVariableType.Execution &&
			!isFunctionReferencePin(droppedPin);
		return {
			placeEntry: thenClose((entry: PaletteEntry) => onNodePlace(entry.node)),
			placeComment: thenClose(onCommentPlace),
			placePlaceholder: thenClose(onPlaceholder),
			placeEvent: eventNode
				? thenClose(() => onNodePlace(eventNode))
				: undefined,
			ping: onPingHere ? thenClose(onPingHere) : undefined,
			editAsFlowScript:
				onEditSelectionAsFlowScript && selectionCount > 0
					? thenClose(onEditSelectionAsFlowScript)
					: undefined,
			discussInChat:
				onDiscussInChat && selectionCount === 1
					? thenClose(onDiscussInChat)
					: undefined,
			// Which file an event belongs to follows its ENTRY node: moving part of a
			// chain changes where those nodes are drawn, not the event's file.
			moveToModule:
				onMoveSelectionToModule &&
				movableSelectionCount > 0 &&
				modules.length > 0
					? thenClose(onMoveSelectionToModule)
					: undefined,
			createVariable: canCreateVariable
				? thenClose(() => createVariableFromPin(droppedPin))
				: undefined,
		};
	}, [
		close,
		nodes,
		droppedPin,
		onCreateVariable,
		onNodePlace,
		onCommentPlace,
		onPlaceholder,
		onPingHere,
		onEditSelectionAsFlowScript,
		onDiscussInChat,
		onMoveSelectionToModule,
		selectionCount,
		movableSelectionCount,
		modules.length,
		createVariableFromPin,
	]);

	// Closing clears the board's dropped pin, which reshapes the palette's rows and chips.
	// The exit animation keeps showing the session the user acted on.
	const sessionRef = useRef({ drop: liveDrop, handlers });
	if (open) sessionRef.current = { drop: liveDrop, handlers };
	const shown = open ? { drop: liveDrop, handlers } : sessionRef.current;

	const virtualAnchor = useMemo(
		() => ({
			current: {
				getBoundingClientRect: () =>
					DOMRect.fromRect({ ...point, width: 0, height: 0 }),
			},
		}),
		[point],
	);

	return (
		// Modal like the context menu it replaces: the click that dismisses it must not
		// also reach the canvas, where it would clear the selection or start a drag.
		<Popover
			modal
			open={open}
			onOpenChange={(next) => {
				if (!next) close();
			}}
		>
			<PopoverAnchor virtualRef={virtualAnchor} />
			<Slot
				// A node's own context menu handles its right-click first and marks the
				// event handled; the board palette only opens for what is left over.
				onContextMenu={(event: React.MouseEvent) => {
					clearLongPress();
					if (event.defaultPrevented) return;
					event.preventDefault();
					openAt(event.clientX, event.clientY);
				}}
				onPointerDown={(event: PointerEvent) => {
					if (event.defaultPrevented || !isTouchOrPen(event)) return;
					clearLongPress();
					const { clientX, clientY } = event;
					longPressRef.current = setTimeout(
						() => openAt(clientX, clientY),
						LONG_PRESS_MS,
					);
				}}
				onPointerMove={(event: PointerEvent) => {
					if (isTouchOrPen(event)) clearLongPress();
				}}
				onPointerUp={(event: PointerEvent) => {
					if (isTouchOrPen(event)) clearLongPress();
				}}
				onPointerCancel={(event: PointerEvent) => {
					if (isTouchOrPen(event)) clearLongPress();
				}}
			>
				{children}
			</Slot>
			{model && (
				<NodePalette
					key={session}
					open={open}
					model={model}
					drop={shown.drop}
					selectionCount={Math.max(selectionCount, movableSelectionCount)}
					modules={modules}
					currentModuleId={currentLayerId ?? null}
					handlers={shown.handlers}
				/>
			)}
		</Popover>
	);
}
