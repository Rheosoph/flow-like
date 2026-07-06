import { useReactFlow } from "@xyflow/react";
import { ChevronDown } from "lucide-react";
import { type RefObject, useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { useBackend } from "../../../..";
import {
	Select,
	SelectContent,
	SelectGroup,
	SelectItem,
	SelectLabel,
	SelectTrigger,
} from "../../../../components/ui/select";
import { useInvalidateInvoke } from "../../../../hooks";
import { updateNodeCommand } from "../../../../lib";
import type { IBoard } from "../../../../lib/schema/flow/board";
import type { IPin } from "../../../../lib/schema/flow/pin";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../../lib/uint8";
import type { IRemoteEvent } from "../../../../state/backend-state/types";
import { useUndoRedo } from "../../flow-history";

const REMOTE_APP_PIN_NAME = "_flow_remote_app_id";
const REMOTE_EVENT_META_PIN_NAME = "_flow_remote_event_meta";

function normalizeStringValue(value: number[] | undefined | null): string {
	const parsed = parseUint8ArrayToJson(value);
	return typeof parsed === "string" ? parsed : "";
}

export function RemoteEventSelect({
	pin,
	value,
	appId,
	boardId,
	nodeId,
	boardRef,
	setValue,
	onPreviewValue,
}: Readonly<{
	pin: IPin;
	value: number[] | undefined | null;
	appId: string;
	boardId?: string;
	nodeId: string;
	boardRef?: RefObject<IBoard | undefined>;
	setValue: (value: number[] | undefined) => void;
	onPreviewValue?: (value: number[] | undefined) => void;
}>) {
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const { getNode } = useReactFlow();
	const { pushCommand } = useUndoRedo(appId, boardId ?? "");
	const [open, setOpen] = useState(false);
	const [loadedEvents, setLoadedEvents] = useState<{
		targetAppId: string;
		events: IRemoteEvent[];
	}>({ targetAppId: "", events: [] });
	const [loading, setLoading] = useState(false);
	const [error, setError] = useState(false);
	const selectedEventId = normalizeStringValue(value);

	const remoteAppPin = Object.values(
		boardRef?.current?.nodes?.[nodeId]?.pins ?? {},
	).find((nodePin) => nodePin.name === REMOTE_APP_PIN_NAME);
	const targetAppId = normalizeStringValue(remoteAppPin?.default_value);

	const events =
		loadedEvents.targetAppId === targetAppId ? loadedEvents.events : [];
	const eventsLoaded = loadedEvents.targetAppId === targetAppId && !loading;
	const selectedEvent = events.find((event) => event.id === selectedEventId);
	const selectedEventMissing =
		Boolean(selectedEventId) && eventsLoaded && !error && !selectedEvent;

	useEffect(() => {
		if (!appId || !targetAppId || !open) return;

		let cancelled = false;

		async function loadEvents() {
			setLoading(true);
			setError(false);

			try {
				const remoteEvents = await backend.teamState.getRemoteEvents(
					appId,
					targetAppId,
				);

				if (cancelled) return;

				setLoadedEvents({ targetAppId, events: remoteEvents });
			} catch {
				if (!cancelled) setError(true);
			} finally {
				if (!cancelled) setLoading(false);
			}
		}

		void loadEvents();

		return () => {
			cancelled = true;
		};
	}, [appId, backend.teamState, open, targetAppId]);

	const handleOpenChange = useCallback((isOpen: boolean) => {
		setOpen(isOpen);
	}, []);

	const persistSelection = useCallback(
		async (eventId: string) => {
			const encodedEventId = convertJsonToUint8Array(eventId);
			if (!encodedEventId) return;

			const boardNode = boardRef?.current?.nodes?.[nodeId];
			const metaPin = boardNode
				? Object.values(boardNode.pins ?? {}).find(
						(nodePin) => nodePin.name === REMOTE_EVENT_META_PIN_NAME,
					)
				: undefined;

			if (!boardId || !boardNode || !metaPin) {
				setValue(encodedEventId);
				return;
			}

			onPreviewValue?.(encodedEventId);

			let metaValue = "";
			try {
				const detail = await backend.teamState.getRemoteEventDetail(
					appId,
					targetAppId,
					eventId,
				);
				metaValue = JSON.stringify(detail);
			} catch {
				toast.warning(
					"Could not load remote event details. Dynamic pins may be unavailable.",
				);
			}

			const flowNode = getNode(nodeId);
			const coordinates = flowNode
				? [flowNode.position.x, flowNode.position.y, 0]
				: (boardNode.coordinates ?? [0, 0, 0]);

			const command = updateNodeCommand({
				node: {
					...boardNode,
					hash: undefined,
					coordinates,
					pins: {
						...boardNode.pins,
						[pin.id]: { ...pin, default_value: encodedEventId },
						[metaPin.id]: {
							...metaPin,
							default_value: convertJsonToUint8Array(metaValue) ?? [],
						},
					},
				},
			});

			try {
				const result = await backend.boardState.executeCommand(
					appId,
					boardId,
					command,
				);
				await pushCommand(result, false);
			} catch {
				toast.error("Failed to save remote event selection");
			} finally {
				await invalidate(backend.boardState.getBoard, [appId, boardId]);
			}
		},
		[
			appId,
			backend.boardState,
			backend.teamState,
			boardId,
			boardRef,
			getNode,
			invalidate,
			nodeId,
			onPreviewValue,
			pin,
			pushCommand,
			setValue,
			targetAppId,
		],
	);

	return (
		<div
			className="flex flex-row items-center justify-start max-w-full ml-1 overflow-hidden"
			onMouseDown={(e) => e.stopPropagation()}
			onPointerDown={(e) => e.stopPropagation()}
		>
			<Select
				disabled={!targetAppId}
				open={open}
				onOpenChange={handleOpenChange}
				value={selectedEventId || undefined}
				onValueChange={(eventId) => void persistSelection(eventId)}
			>
				<SelectTrigger
					noChevron
					size="sm"
					className="w-fit! max-w-full! p-0 border-0 text-xs bg-card! text-start max-h-fit h-4 gap-0.5 flex-row items-center overflow-hidden"
				>
					<small className="text-start text-[10px] m-0! truncate">
						{!targetAppId && "Select a project first"}
						{targetAppId &&
							(selectedEvent?.name || selectedEventId || "Select event")}
					</small>
					<ChevronDown className="size-2 min-w-2 min-h-2 text-card-foreground shrink-0" />
				</SelectTrigger>
				<SelectContent>
					<SelectGroup>
						<SelectLabel>{pin.friendly_name}</SelectLabel>
						{loading && events.length === 0 && (
							<SelectLabel>Loading events...</SelectLabel>
						)}
						{error && <SelectLabel>Could not load remote events</SelectLabel>}
						{!loading && !error && events.length === 0 && !selectedEventId && (
							<SelectLabel>No shared events found</SelectLabel>
						)}
						{events.map((event) => (
							<SelectItem key={event.id} value={event.id}>
								{event.name}
								<span className="text-muted-foreground">
									{" "}
									· {event.event_type}
								</span>
							</SelectItem>
						))}
						{selectedEventId && !selectedEvent && (
							<SelectItem key={selectedEventId} value={selectedEventId}>
								{selectedEventId}
								{selectedEventMissing && (
									<span className="text-muted-foreground">
										{" "}
										(not found in project)
									</span>
								)}
							</SelectItem>
						)}
					</SelectGroup>
				</SelectContent>
			</Select>
		</div>
	);
}
