"use client";

import { useTranslation } from "@flow-like/locales";
import { BracesIcon, MapIcon, PlusIcon, XIcon } from "lucide-react";
import {
	type ComponentType,
	type ReactNode,
	Suspense,
	lazy,
	useCallback,
	useEffect,
	useId,
	useMemo,
	useRef,
	useState,
} from "react";
import {
	type GeometryKind,
	geometryKindFromSchema,
	normalizeGeometryValue,
} from "../../../lib/geometry";
import {
	FIRST_RING,
	GEOMETRY_DRAFT_KINDS,
	type GeometryDraft,
	type GeometryDraftCapabilities,
	type GeometryDraftKind,
	type GeometryPosition,
	type GeometryRing,
	type GeometryRingRef,
	addDraftPosition,
	addDraftRing,
	addDraftShape,
	clampRingRef,
	convertGeometryDraft,
	draftCapabilities,
	draftPositionCount,
	draftToGeometry,
	emptyGeometryDraft,
	geometryToDraft,
	removeDraftPosition,
	removeDraftRing,
	removeDraftShape,
	updateDraftPosition,
} from "../../../lib/geometry-draft";
import { IValueType } from "../../../lib/schema/flow/pin";
import { cn } from "../../../lib/utils";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import {
	type GeometryFieldProps,
	GeometryJsonField,
} from "./geometry-json-field";

const GeometryEditorMap = lazy(() => import("../../ui/geometry-editor-map"));

type EditorMode = "map" | "json";

interface EditorState {
	draft: GeometryDraft;
	mode: EditorMode;
	active: GeometryRingRef;
	/** False for GeometryCollection or values the visual editor cannot show. */
	draftable: boolean;
	error: string | null;
}

type DraftUpdate = (draft: GeometryDraft) => GeometryDraft;
type ApplyOptions = { commit?: boolean; active?: GeometryRingRef };
type Apply = (update: DraftUpdate, options?: ApplyOptions) => void;

const valueKey = (value: unknown) =>
	value == null ? "null" : JSON.stringify(value);

const editableKind = (kind: GeometryKind | null): GeometryDraftKind | null =>
	kind && kind !== "GeometryCollection" ? kind : null;

function deriveState(
	value: unknown,
	lockedKind: GeometryKind | null,
	schema: string | null | undefined,
	refs: Record<string, string> | undefined,
	previous?: EditorState,
): EditorState {
	const jsonOnly = lockedKind === "GeometryCollection";
	const mode: EditorMode = jsonOnly ? "json" : (previous?.mode ?? "map");
	const active = previous?.active ?? FIRST_RING;
	if (value == null) {
		const kind = editableKind(lockedKind) ?? previous?.draft.kind ?? "Point";
		return {
			draft: emptyGeometryDraft(kind),
			mode,
			active: FIRST_RING,
			draftable: !jsonOnly,
			error: null,
		};
	}
	let draft: GeometryDraft | null = null;
	try {
		normalizeGeometryValue(value, { schema, refs });
		draft = geometryToDraft(value);
	} catch {
		draft = null;
	}
	if (!draft) {
		return {
			draft:
				previous?.draft ??
				emptyGeometryDraft(editableKind(lockedKind) ?? "Point"),
			mode: "json",
			active,
			draftable: false,
			error: null,
		};
	}
	return {
		draft,
		mode,
		active: clampRingRef(active, draft),
		draftable: true,
		error: null,
	};
}

/** Map + coordinate editor for one geometry; GeoJSON text stays available as a mode. */
export function GeometryEditor({
	value,
	onChange,
	schema,
	refs,
	disabled,
	allowUnset = true,
	preview = true,
}: Readonly<GeometryFieldProps>) {
	const { t } = useTranslation("flow");
	const id = useId();
	const lockedKind = useMemo(() => {
		try {
			return geometryKindFromSchema(schema, refs);
		} catch {
			return null;
		}
	}, [schema, refs]);
	const [state, setState] = useState<EditorState>(() =>
		deriveState(value, lockedKind, schema, refs),
	);
	const stateRef = useRef(state);
	stateRef.current = state;
	const synced = useRef({ key: valueKey(value), lockedKind });
	const centerRef = useRef<GeometryPosition | null>(null);

	useEffect(() => {
		const key = valueKey(value);
		if (synced.current.key === key && synced.current.lockedKind === lockedKind)
			return;
		synced.current = { key, lockedKind };
		setState((previous) =>
			deriveState(value, lockedKind, schema, refs, previous),
		);
	}, [value, lockedKind, schema, refs]);

	const apply = useCallback<Apply>(
		(update, options = {}) => {
			const commit = options.commit ?? true;
			const draft = update(stateRef.current.draft);
			let error: string | null = null;
			if (commit) {
				try {
					const normalized = normalizeGeometryValue(draftToGeometry(draft), {
						schema,
						refs,
						allowUnset,
					});
					synced.current = { key: valueKey(normalized), lockedKind };
					onChange(normalized, true);
				} catch (e) {
					error = e instanceof Error ? e.message : String(e);
					onChange(undefined, false);
				}
			}
			const next: EditorState = {
				...stateRef.current,
				draft,
				draftable: true,
				active: clampRingRef(options.active ?? stateRef.current.active, draft),
				error: commit ? error : stateRef.current.error,
			};
			stateRef.current = next;
			setState(next);
		},
		[schema, refs, allowUnset, onChange, lockedKind],
	);

	const setMode = (mode: EditorMode) =>
		setState((previous) => ({ ...previous, mode }));

	const setKind = (kind: string) => {
		if (kind === "GeometryCollection") {
			setState((previous) => ({ ...previous, mode: "json", draftable: false }));
			return;
		}
		const next = kind as GeometryDraftKind;
		if (!state.draftable) {
			stateRef.current = { ...stateRef.current, mode: "map" };
			apply(() => emptyGeometryDraft(next), { active: FIRST_RING });
			return;
		}
		apply((draft) => convertGeometryDraft(draft, next), { active: FIRST_RING });
	};

	const capabilities = draftCapabilities(state.draft.kind);
	const count = draftPositionCount(state.draft);
	const kindValue = state.draftable ? state.draft.kind : "GeometryCollection";
	const hint = capabilities.singlePosition
		? t("geometryMapHintPoint", "Click the map to place the point, or drag it.")
		: t(
				"geometryMapHintDraw",
				"Click the map to add vertices to the selected part. Drag a vertex to move it.",
			);

	return (
		<div className="grid w-full gap-3">
			<div className="flex flex-wrap items-center gap-2">
				<Label htmlFor={`${id}-kind`} className="text-xs text-muted-foreground">
					{t("geometryType", "Type")}
				</Label>
				<Select
					value={kindValue}
					disabled={disabled || lockedKind !== null}
					onValueChange={setKind}
				>
					<SelectTrigger id={`${id}-kind`} size="sm" className="w-44">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						{GEOMETRY_DRAFT_KINDS.map((kind) => (
							<SelectItem key={kind} value={kind}>
								{kind}
							</SelectItem>
						))}
						<SelectItem value="GeometryCollection">
							GeometryCollection
						</SelectItem>
					</SelectContent>
				</Select>
				{count > 0 && (
					<Button
						type="button"
						variant="ghost"
						size="sm"
						className="h-7 gap-1 px-2 text-xs"
						disabled={disabled}
						onClick={() =>
							apply((draft) => emptyGeometryDraft(draft.kind), {
								active: FIRST_RING,
							})
						}
					>
						<XIcon className="size-3.5" />
						{t("geometryClear", "Clear")}
					</Button>
				)}
				<div
					role="group"
					aria-label={t("geometryEditorMode", "Editor mode")}
					className="ml-auto flex items-center gap-0.5 rounded-md border p-0.5"
				>
					<ModeButton
						active={state.mode === "map"}
						disabled={!state.draftable}
						icon={MapIcon}
						onClick={() => setMode("map")}
					>
						{t("geometryModeMap", "Map")}
					</ModeButton>
					<ModeButton
						active={state.mode === "json"}
						icon={BracesIcon}
						onClick={() => setMode("json")}
					>
						{t("geometryModeJson", "JSON")}
					</ModeButton>
				</div>
			</div>
			{state.mode === "json" ? (
				<GeometryJsonField
					value={value}
					onChange={onChange}
					schema={schema}
					refs={refs}
					valueType={IValueType.Normal}
					disabled={disabled}
					allowUnset={allowUnset}
					preview={preview}
				/>
			) : (
				<>
					{preview && (
						<Suspense fallback={<MapPlaceholder />}>
							<GeometryEditorMap
								draft={state.draft}
								active={state.active}
								disabled={disabled}
								centerRef={centerRef}
								onAddPosition={(position) =>
									apply((draft) =>
										addDraftPosition(draft, stateRef.current.active, position),
									)
								}
								onMovePosition={(ref, index, position, commit) =>
									apply(
										(draft) => updateDraftPosition(draft, ref, index, position),
										{ commit },
									)
								}
								onSelectRing={(ref) =>
									setState((previous) => ({ ...previous, active: ref }))
								}
							/>
						</Suspense>
					)}
					{preview && <p className="text-xs text-muted-foreground">{hint}</p>}
					<CoordinatePanel
						draft={state.draft}
						active={state.active}
						capabilities={capabilities}
						disabled={disabled}
						centerRef={centerRef}
						apply={apply}
						onSelect={(ref) =>
							setState((previous) => ({ ...previous, active: ref }))
						}
					/>
					{state.error && (
						<p role="alert" className="text-xs text-destructive">
							{state.error}
						</p>
					)}
				</>
			)}
		</div>
	);
}

function MapPlaceholder() {
	const { t } = useTranslation("flow");
	return (
		<div className="flex h-64 items-center justify-center rounded-md border bg-muted/30 text-xs text-muted-foreground">
			{t("geometryLoadingMap", "Loading map…")}
		</div>
	);
}

function ModeButton({
	active,
	disabled,
	icon: Icon,
	onClick,
	children,
}: {
	active: boolean;
	disabled?: boolean;
	icon: ComponentType<{ className?: string }>;
	onClick: () => void;
	children: ReactNode;
}) {
	return (
		<Button
			type="button"
			size="sm"
			variant={active ? "secondary" : "ghost"}
			className="h-7 gap-1 px-2 text-xs"
			aria-pressed={active}
			disabled={disabled}
			onClick={onClick}
		>
			<Icon className="size-3.5" />
			{children}
		</Button>
	);
}

function RemoveButton({
	label,
	disabled,
	onClick,
}: {
	label: string;
	disabled?: boolean;
	onClick: () => void;
}) {
	return (
		<Button
			type="button"
			variant="ghost"
			size="icon"
			className="ml-auto size-6 text-muted-foreground hover:text-destructive"
			aria-label={label}
			title={label}
			disabled={disabled}
			onClick={onClick}
		>
			<XIcon className="size-3.5" />
		</Button>
	);
}

function CoordinatePanel({
	draft,
	active,
	capabilities,
	disabled,
	centerRef,
	apply,
	onSelect,
}: {
	draft: GeometryDraft;
	active: GeometryRingRef;
	capabilities: GeometryDraftCapabilities;
	disabled?: boolean;
	centerRef: { current: GeometryPosition | null };
	apply: Apply;
	onSelect: (ref: GeometryRingRef) => void;
}) {
	const { t } = useTranslation("flow");
	const selectable =
		draft.shapes.length > 1 || draft.shapes.some((shape) => shape.length > 1);
	return (
		<div className="grid max-h-72 gap-2 overflow-y-auto pr-1">
			{draft.shapes.map((shape, s) => (
				<div
					key={s}
					className={cn(
						"grid gap-2",
						capabilities.multiShape && "rounded-md border p-2",
						capabilities.multiShape &&
							!capabilities.holes &&
							s === active.shape &&
							selectable &&
							"border-orange-500/50 bg-orange-500/5",
					)}
				>
					{capabilities.multiShape && (
						<div className="flex items-center gap-2 text-xs">
							{capabilities.holes || !selectable ? (
								<span className="font-medium">
									{t("geometryPart", "Part {{index}}", { index: s + 1 })}
								</span>
							) : (
								<button
									type="button"
									className={cn(
										"rounded font-medium",
										s === active.shape
											? "text-orange-700 dark:text-orange-400"
											: "text-muted-foreground hover:text-foreground",
									)}
									aria-pressed={s === active.shape}
									disabled={disabled}
									onClick={() => onSelect({ shape: s, ring: 0 })}
								>
									{t("geometryPart", "Part {{index}}", { index: s + 1 })}
								</button>
							)}
							{draft.shapes.length > 1 && (
								<RemoveButton
									label={t("geometryRemovePart", "Remove part")}
									disabled={disabled}
									onClick={() =>
										apply((current) => removeDraftShape(current, s), {
											active: FIRST_RING,
										})
									}
								/>
							)}
						</div>
					)}
					{shape.map((ring, r) => {
						const ref = { shape: s, ring: r };
						return (
							<RingSection
								key={r}
								ring={ring}
								label={
									capabilities.holes
										? r === 0
											? t("geometryExteriorRing", "Exterior ring")
											: t("geometryHole", "Hole {{index}}", { index: r })
										: null
								}
								active={s === active.shape && r === active.ring}
								selectable={capabilities.holes && selectable}
								removable={r > 0}
								disabled={disabled}
								capabilities={capabilities}
								onSelect={() => onSelect(ref)}
								onRemove={() =>
									apply((current) => removeDraftRing(current, ref), {
										active: { shape: s, ring: 0 },
									})
								}
								onAdd={() =>
									apply(
										(current) =>
											addDraftPosition(
												current,
												ref,
												centerRef.current ?? ring[ring.length - 1] ?? [0, 0],
											),
										{ active: ref },
									)
								}
								onCommit={(index, position) =>
									apply(
										(current) =>
											index < current.shapes[s][r].length
												? updateDraftPosition(current, ref, index, position)
												: addDraftPosition(current, ref, position),
										{ active: ref },
									)
								}
								onRemovePosition={(index) =>
									apply((current) => removeDraftPosition(current, ref, index))
								}
							/>
						);
					})}
					{capabilities.holes && (
						<Button
							type="button"
							variant="outline"
							size="sm"
							className="w-fit gap-1 text-xs"
							disabled={disabled}
							onClick={() =>
								apply((current) => addDraftRing(current, s), {
									active: { shape: s, ring: shape.length },
								})
							}
						>
							<PlusIcon className="size-3.5" />
							{t("geometryAddHole", "Add hole")}
						</Button>
					)}
				</div>
			))}
			{capabilities.multiShape && (
				<Button
					type="button"
					variant="outline"
					size="sm"
					className="w-fit gap-1 text-xs"
					disabled={disabled}
					onClick={() =>
						apply(addDraftShape, {
							active: { shape: draft.shapes.length, ring: 0 },
						})
					}
				>
					<PlusIcon className="size-3.5" />
					{t("geometryAddPart", "Add part")}
				</Button>
			)}
		</div>
	);
}

function RingSection({
	ring,
	label,
	active,
	selectable,
	removable,
	disabled,
	capabilities,
	onSelect,
	onRemove,
	onAdd,
	onCommit,
	onRemovePosition,
}: {
	ring: GeometryRing;
	label: string | null;
	active: boolean;
	selectable: boolean;
	removable: boolean;
	disabled?: boolean;
	capabilities: GeometryDraftCapabilities;
	onSelect: () => void;
	onRemove: () => void;
	onAdd: () => void;
	onCommit: (index: number, position: GeometryPosition) => void;
	onRemovePosition: (index: number) => void;
}) {
	const { t } = useTranslation("flow");
	const rows: (GeometryPosition | null)[] =
		ring.length === 0 && capabilities.singlePosition ? [null] : ring;
	return (
		<div
			className={cn(
				"grid gap-1",
				label !== null && "rounded-md border p-2",
				label !== null &&
					active &&
					selectable &&
					"border-orange-500/50 bg-orange-500/5",
			)}
		>
			{label !== null && (
				<div className="flex items-center gap-2 text-xs">
					{selectable ? (
						<button
							type="button"
							className={cn(
								"rounded font-medium",
								active
									? "text-orange-700 dark:text-orange-400"
									: "text-muted-foreground hover:text-foreground",
							)}
							aria-pressed={active}
							disabled={disabled}
							onClick={onSelect}
						>
							{label}
						</button>
					) : (
						<span className="font-medium">{label}</span>
					)}
					<span className="tabular-nums text-muted-foreground">
						{t("geometryVertexCount", "{{count}} vertices", {
							count: ring.length,
							defaultValue_one: "{{count}} vertex",
						})}
					</span>
					{removable && (
						<RemoveButton
							label={t("geometryRemoveHole", "Remove hole")}
							disabled={disabled}
							onClick={onRemove}
						/>
					)}
				</div>
			)}
			{rows.map((position, index) => (
				<PositionRow
					key={index}
					index={index}
					position={position}
					disabled={disabled}
					onCommit={(next) => onCommit(index, next)}
					onRemove={position ? () => onRemovePosition(index) : undefined}
				/>
			))}
			{!capabilities.singlePosition && (
				<Button
					type="button"
					variant="ghost"
					size="sm"
					className="w-fit gap-1 text-xs"
					disabled={disabled}
					onClick={onAdd}
				>
					<PlusIcon className="size-3.5" />
					{t("geometryAddVertex", "Add vertex")}
				</Button>
			)}
		</div>
	);
}

const formatCoordinate = (value: number | undefined) =>
	value === undefined ? "" : String(value);

/** Both fields must parse before a position is committed, so nothing is invented. */
function PositionRow({
	index,
	position,
	disabled,
	onCommit,
	onRemove,
}: {
	index: number;
	position: GeometryPosition | null;
	disabled?: boolean;
	onCommit: (position: GeometryPosition) => void;
	onRemove?: () => void;
}) {
	const { t } = useTranslation("flow");
	const longitude = position?.[0];
	const latitude = position?.[1];
	const [text, setText] = useState(() => ({
		lon: formatCoordinate(longitude),
		lat: formatCoordinate(latitude),
	}));
	const focused = useRef(0);
	useEffect(() => {
		if (focused.current === 0)
			setText({
				lon: formatCoordinate(longitude),
				lat: formatCoordinate(latitude),
			});
	}, [longitude, latitude]);
	const change = (field: "lon" | "lat", raw: string) => {
		const next = { ...text, [field]: raw };
		setText(next);
		const lon = Number.parseFloat(next.lon);
		const lat = Number.parseFloat(next.lat);
		if (Number.isFinite(lon) && Number.isFinite(lat)) onCommit([lon, lat]);
	};
	const blur = () => {
		focused.current--;
		if (focused.current === 0)
			setText({
				lon: formatCoordinate(longitude),
				lat: formatCoordinate(latitude),
			});
	};
	const focus = () => {
		focused.current++;
	};
	return (
		<div className="grid grid-cols-[1.25rem_1fr_1fr_1.5rem] items-center gap-1">
			<span className="text-[10px] tabular-nums text-muted-foreground">
				{index + 1}
			</span>
			<Input
				type="number"
				inputMode="decimal"
				step="any"
				min={-180}
				max={180}
				className="h-7 text-xs tabular-nums"
				aria-label={t("geometryLongitude", "Longitude")}
				placeholder={t("geometryLongitude", "Longitude")}
				value={text.lon}
				disabled={disabled}
				onFocus={focus}
				onBlur={blur}
				onChange={(event) => change("lon", event.target.value)}
			/>
			<Input
				type="number"
				inputMode="decimal"
				step="any"
				min={-90}
				max={90}
				className="h-7 text-xs tabular-nums"
				aria-label={t("geometryLatitude", "Latitude")}
				placeholder={t("geometryLatitude", "Latitude")}
				value={text.lat}
				disabled={disabled}
				onFocus={focus}
				onBlur={blur}
				onChange={(event) => change("lat", event.target.value)}
			/>
			{onRemove ? (
				<RemoveButton
					label={t("geometryRemoveVertex", "Remove vertex")}
					disabled={disabled}
					onClick={onRemove}
				/>
			) : (
				<span />
			)}
		</div>
	);
}
