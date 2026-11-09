"use client";

import { Dices, Lock, LockOpen } from "lucide-react";
import {
	type ReactNode,
	memo,
	useEffect,
	useId,
	useRef,
	useState,
} from "react";
import { resolveBotAppearance } from "./appearance";
import { BotAvatar } from "./bot-avatar";
import { BotColorPicker } from "./bot-color-picker";
import { BotOutfitTile } from "./bot-outfit-tile";
import { defaultBotCatalog } from "./catalog";
import {
	getBotCustomizableColorSlots,
	normalizeBotColorSelection,
} from "./colors";
import { BOT_EFFECTS, BOT_EFFECT_GROUPS } from "./effects";
import { isBotEquipmentCompatible } from "./equipment-fit";
import {
	canUseBotEquipment,
	canUseBotShape,
	getBotEquipmentPacks,
	getBotPackContents,
	getBotShapePacks,
} from "./packs";
import {
	BOT_LOOK_PARTS,
	type BotLookPart,
	randomizeBotLook,
} from "./random-look";
import { getBotSaveEligibility } from "./save-eligibility";
import { BOT_AGENT_STATES, BOT_MOODS } from "./states";
import {
	BOT_COLOR_SLOTS,
	BOT_EQUIPMENT_SLOTS,
	BOT_EXPRESSIONS,
	type BotAgentState,
	type BotAppearance,
	type BotCatalog,
	type BotColorPackDefinition,
	type BotColorSelection,
	type BotColorSlot,
	type BotEquipmentDefinition,
	type BotEquipmentSlot,
	type BotExpression,
	type BotMood,
	type BotPackDefinition,
	type BotPose,
	type BotShapeDefinition,
} from "./types";
import "./bot-customizer.css";

export interface BotCustomizerProps {
	value: BotAppearance;
	/** Updates the preview draft only. The host must not persist changes here. */
	onChange: (value: BotAppearance) => void;
	/** Persists an eligible snapshot. The host remains responsible for enforcing access. */
	onSave?: (value: BotAppearance) => void | Promise<void>;
	catalog?: BotCatalog;
	className?: string;
	previewSize?: number;
	/** Optional ownership restrictions. Omit to make the full catalog available. */
	ownedPackIds?: readonly string[];
	/** The host opens pack details or its purchase flow. */
	onRequestPack?: (pack: BotPackDefinition) => void;
	onRequestColorPack?: (pack: BotColorPackDefinition) => void;
}

const SECTIONS = [
	{ id: "crew", label: "Crew", hint: "Find your character" },
	{ id: "outfit", label: "Outfit", hint: "Give them a style" },
	{ id: "colors", label: "Colors", hint: "Make the palette yours" },
	{ id: "finish", label: "Finish", hint: "Add a little personality" },
	{ id: "skills", label: "Skills", hint: "Show what they do" },
] as const;
type Section = (typeof SECTIONS)[number]["id"];
type SeriesFilter =
	| { kind: "all" }
	| { kind: "other" }
	| { kind: "pack"; id: string }
	| { kind: "series"; id: string };
const SLOT_LABELS: Record<BotEquipmentSlot, string> = {
	neck: "Clothing",
	eyes: "Eyewear",
	hand: "Hand accessory",
	head: "Headwear",
};
const FINISH_GROUPS = [
	{
		id: "backdrop",
		label: "Pattern",
		options: [
			["none", "None"],
			["gradient", "Gradient"],
			["dots", "Dots"],
			["stripes", "Stripes"],
			["waves", "Waves"],
			["rays", "Rays"],
			["stars", "Stars"],
			["confetti", "Confetti"],
		],
	},
	{
		id: "halo",
		label: "Ring",
		options: [
			["none", "None"],
			["flow", "Flow"],
			["gold", "Gold"],
			["aurora", "Aurora"],
			["pulse", "Pulse"],
			["orbit", "Orbit"],
		],
	},
	{
		id: "effect",
		label: "Effect",
		options: BOT_EFFECTS.map((effect) => [effect.id, effect.label] as const),
	},
	{
		id: "frame",
		label: "Frame",
		options: [
			["circle", "Circle"],
			["squircle", "Squircle"],
			["square", "Square"],
		],
	},
	{
		id: "emote",
		label: "Emote",
		options: [
			["none", "None"],
			["heart", "Heart"],
			["note", "Music note"],
			["spark", "Spark"],
			["alert", "Exclamation"],
			["question", "Question"],
			["drop", "Drop"],
			["zzz", "Sleep"],
			["rain", "Rain"],
			["anger", "Anger"],
			["loading", "Working"],
			["idea", "Idea"],
			["check", "Done"],
			["error", "Error"],
		],
	},
] as const;
type Finish = (typeof FINISH_GROUPS)[number]["id"];
type PreviewState =
	| "custom"
	| `mood:${BotMood}`
	| `state:${Exclude<BotAgentState, "idle">}`;
const PREVIEW_POSES: readonly (readonly [BotPose, string])[] = [
	["idle", "Idle"],
	["sleep", "Sleep"],
	["track", "Follow cursor"],
];
const PART_LABELS: Record<BotLookPart, string> = {
	crew: "Character",
	outfit: "Outfit",
	colors: "Colors",
	finish: "Finish",
};

function isLookPart(section: Section): section is BotLookPart {
	return (BOT_LOOK_PARTS as readonly string[]).includes(section);
}

// Keep the crew's SVG artwork stable while the selected bot changes.
const CharacterThumbnail = memo(function CharacterThumbnail({
	kind,
	catalog,
	expression,
	size,
}: {
	kind: string;
	catalog: BotCatalog;
	expression: BotExpression;
	size: number;
}) {
	return (
		<BotAvatar
			appearance={{ kind }}
			catalog={catalog}
			size={size}
			expression={expression}
			pose="center"
			animated={false}
			decorative
		/>
	);
});

function Tile({
	label,
	selected,
	onClick,
	children,
	description,
	showDescription = false,
	locked = false,
}: {
	label: string;
	selected: boolean;
	onClick: () => void;
	children: ReactNode;
	description?: string;
	showDescription?: boolean;
	locked?: boolean;
}) {
	return (
		<button
			type="button"
			className="fl-bot-studio-tile"
			aria-label={`Choose ${label}`}
			aria-pressed={selected}
			data-locked={locked || undefined}
			onClick={onClick}
			title={description}
		>
			<span className="fl-bot-studio-tile-art" aria-hidden="true">
				{children}
			</span>
			<span className="fl-bot-studio-tile-name">{label}</span>
			{showDescription && description && (
				<span className="fl-bot-studio-tile-description">{description}</span>
			)}
			{locked && <span className="fl-bot-studio-tile-locked">Preview</span>}
			{selected && (
				<span className="fl-bot-studio-tile-check" aria-hidden="true">
					✓
				</span>
			)}
		</button>
	);
}

function ShuffleButton({
	part,
	locked,
	filtered,
	onShuffle,
}: Readonly<{
	part: BotLookPart;
	locked: boolean;
	filtered: boolean;
	onShuffle: () => void;
}>) {
	const label = PART_LABELS[part].toLowerCase();
	return (
		<button
			type="button"
			className="fl-bot-studio-shuffle"
			aria-label={`Shuffle ${label}`}
			disabled={locked}
			title={
				locked
					? `Unlock ${label} to shuffle it`
					: filtered
						? `Picks from the ${part === "crew" ? "characters" : "collection"} shown`
						: undefined
			}
			onClick={onShuffle}
		>
			{locked ? <Lock aria-hidden="true" /> : <Dices aria-hidden="true" />}
			{locked ? "Locked" : filtered ? "Shuffle shown" : "Shuffle"}
		</button>
	);
}

/** A controlled visual editor, with catalog extensions available in every picker. */
export function BotCustomizer({
	value,
	onChange,
	onSave,
	catalog = defaultBotCatalog,
	className,
	previewSize = 200,
	ownedPackIds,
	onRequestPack,
	onRequestColorPack,
}: BotCustomizerProps) {
	const [section, setSection] = useState<Section>("crew");
	const [requestedSeries, setSeries] = useState<SeriesFilter>({ kind: "all" });
	const [search, setSearch] = useState("");
	const [outfitSearch, setOutfitSearch] = useState("");
	const [requestedOutfitCollection, setOutfitCollection] = useState<
		Exclude<SeriesFilter, { kind: "series" }>
	>({ kind: "all" });
	const [slot, setSlot] = useState<BotEquipmentSlot>("head");
	const [finish, setFinish] = useState<Finish>("backdrop");
	const [playing, setPlaying] = useState(true);
	const [pose, setPose] = useState<BotPose>("idle");
	const [expression, setExpression] = useState<BotExpression>("neutral");
	const [previewState, setPreviewState] = useState<PreviewState>("custom");
	const [locked, setLocked] = useState<readonly BotLookPart[]>([]);
	const previewMood = previewState.startsWith("mood:")
		? (previewState.slice(5) as BotMood)
		: undefined;
	const previewAgentState = previewState.startsWith("state:")
		? (previewState.slice(6) as BotAgentState)
		: "idle";
	const [saving, setSaving] = useState(false);
	const [savingDraft, setSavingDraft] = useState<string>();
	const [saveError, setSaveError] = useState<{ draft: string }>();
	const savingRef = useRef(false);
	const mountedRef = useRef(true);
	useEffect(() => {
		mountedRef.current = true;
		return () => {
			mountedRef.current = false;
		};
	}, []);
	const id = useId();
	const draftKey = JSON.stringify(value);
	const saveEligibility = getBotSaveEligibility(value, catalog, ownedPackIds);
	const hasUnavailableItems = saveEligibility.requirements.some(
		(item) => item.packIds.length === 0,
	);
	const appearance = resolveBotAppearance(value, catalog);
	const shape = catalog.shapes[appearance.kind];
	const shapes = Object.values(catalog.shapes);
	const packs = Object.values(catalog.packs);
	const unbundledShapes = shapes.filter(
		(item) => getBotShapePacks(item.id, catalog).length === 0,
	);
	const shapeSeries =
		shape.series && Object.hasOwn(catalog.series, shape.series)
			? catalog.series[shape.series]
			: undefined;
	const seriesOptions = Object.values(catalog.series).filter((entry) =>
		shapes.some((item) => item.series === entry.id),
	);
	const hasOther = unbundledShapes.length > 0;
	const series: SeriesFilter =
		(requestedSeries.kind === "series" &&
			!seriesOptions.some((entry) => entry.id === requestedSeries.id)) ||
		(requestedSeries.kind === "pack" &&
			!Object.hasOwn(catalog.packs, requestedSeries.id)) ||
		(requestedSeries.kind === "other" && !hasOther)
			? { kind: "all" }
			: requestedSeries;
	const query = search.trim().toLocaleLowerCase();
	const visibleShapes = shapes.filter((item) => {
		const itemSeries =
			item.series && Object.hasOwn(catalog.series, item.series)
				? catalog.series[item.series]
				: undefined;
		const matchesSeries =
			series.kind === "all" ||
			(series.kind === "other"
				? getBotShapePacks(item.id, catalog).length === 0
				: series.kind === "pack"
					? catalog.packs[series.id].shapeIds.includes(item.id)
					: item.series === series.id);
		return (
			matchesSeries &&
			`${item.label} ${item.description ?? ""} ${itemSeries?.label ?? ""} ${getBotShapePacks(
				item.id,
				catalog,
			)
				.map((pack) => `${pack.label} ${pack.description}`)
				.join(" ")}`
				.toLocaleLowerCase()
				.includes(query)
		);
	});
	const selectionOutsideFilters = !visibleShapes.some(
		(item) => item.id === appearance.kind,
	);
	const lockedVisiblePacks = [
		...new Map(
			visibleShapes
				.filter((item) => !canUseBotShape(item.id, catalog, ownedPackIds))
				.flatMap((item) => getBotShapePacks(item.id, catalog))
				.map((pack) => [pack.id, pack]),
		).values(),
	];
	const availableEquipment = Object.values(catalog.equipment).filter(
		(item) => item.slot === slot && isBotEquipmentCompatible(item, shape),
	);
	const outfitPacks = packs.filter((pack) => pack.equipmentIds?.length);
	const outfitCollection =
		requestedOutfitCollection.kind === "pack" &&
		!outfitPacks.some((pack) => pack.id === requestedOutfitCollection.id)
			? ({ kind: "all" } as const)
			: requestedOutfitCollection;
	const outfitQuery = outfitSearch.trim().toLocaleLowerCase();
	const inOutfitCollection = (item: BotEquipmentDefinition) => {
		const itemPacks = getBotEquipmentPacks(item.id, catalog);
		return (
			outfitCollection.kind === "all" ||
			(outfitCollection.kind === "other"
				? itemPacks.length === 0
				: itemPacks.some((pack) => pack.id === outfitCollection.id))
		);
	};
	const visibleEquipment = availableEquipment.filter(
		(item) =>
			inOutfitCollection(item) &&
			`${item.label} ${getBotEquipmentPacks(item.id, catalog)
				.map((pack) => pack.label)
				.join(" ")}`
				.toLocaleLowerCase()
				.includes(outfitQuery),
	);
	const groupedEquipment = new Set<string>();
	const themedOutfits = outfitPacks
		.filter(
			(pack) =>
				outfitCollection.kind !== "pack" || outfitCollection.id === pack.id,
		)
		.map((pack) => {
			const equipment = visibleEquipment.filter(
				(item) =>
					!groupedEquipment.has(item.id) &&
					pack.equipmentIds?.includes(item.id),
			);
			for (const item of equipment) groupedEquipment.add(item.id);
			return { key: `pack:${pack.id}`, label: pack.label, pack, equipment };
		});
	const outfitGroups = [
		{
			key: "essentials",
			label: "Essentials",
			pack: undefined,
			equipment: visibleEquipment.filter(
				(item) => !groupedEquipment.has(item.id),
			),
		},
		...themedOutfits,
	].filter((group) => group.equipment.length > 0);
	const groupedCrew = !query && series.kind === "all" && packs.length > 0;
	const finishGroup =
		FINISH_GROUPS.find((group) => group.id === finish) ?? FINISH_GROUPS[0];
	const cleanPreview: BotAppearance = {
		kind: appearance.kind,
		frame: appearance.frame,
		dark: appearance.dark,
	};
	const outfitPreview: BotAppearance = {
		...cleanPreview,
		colors: appearance.colors,
		variant: appearance.variant,
		effect: appearance.effect,
	};
	const selectedSection =
		SECTIONS.find((entry) => entry.id === section) ?? SECTIONS[0];
	const colorEffect =
		appearance.effect === "lcd"
			? "LCD"
			: appearance.effect === "blueprint"
				? "Blueprint"
				: appearance.effect === "thermal"
					? "Thermal"
					: undefined;

	function update(patch: Partial<BotAppearance>) {
		const next = { ...value, kind: appearance.kind, ...patch };
		if (next.colors && next.kind !== value.kind) {
			const slots = getBotCustomizableColorSlots(catalog.shapes[next.kind]);
			const carried: NonNullable<BotAppearance["colors"]> = {};
			for (const colorSlot of slots) {
				const selection = normalizeBotColorSelection(
					next.colors[colorSlot],
					catalog,
				);
				if (selection) carried[colorSlot] = selection;
			}
			next.colors = carried;
		}
		setSaveError(undefined);
		onChange(next);
	}

	function replaceAppearance(next: BotAppearance) {
		setSaveError(undefined);
		setPreviewState("custom");
		setExpression("neutral");
		setPose("idle");
		setSearch("");
		setSeries({ kind: "all" });
		setOutfitSearch("");
		setOutfitCollection({ kind: "all" });
		onChange(next);
	}

	function reset() {
		setPlaying(true);
		setSection("crew");
		setSlot("head");
		setFinish("backdrop");
		replaceAppearance({ kind: catalog.defaultKind });
	}

	function randomize() {
		replaceAppearance(
			randomizeBotLook({
				value,
				catalog,
				ownedPackIds,
				parts: BOT_LOOK_PARTS,
				locked,
			}),
		);
	}

	/** Rerolls one part, drawing from what that section currently shows. */
	function shuffle(part: BotLookPart) {
		const shownShapes = new Set(visibleShapes.map((item) => item.id));
		setSaveError(undefined);
		onChange(
			randomizeBotLook({
				value,
				catalog,
				ownedPackIds,
				parts: [part],
				locked,
				shapeFilter: (item) => shownShapes.has(item.id),
				equipmentFilter: inOutfitCollection,
			}),
		);
	}

	function toggleLock(part: BotLookPart) {
		setLocked((current) =>
			current.includes(part)
				? current.filter((entry) => entry !== part)
				: [...current, part],
		);
	}

	function selectColor(
		colorSlot: BotColorSlot,
		selection: BotColorSelection | undefined,
	) {
		const normalized = normalizeBotColorSelection(selection, catalog);
		if (
			selection &&
			(colorEffect ||
				!normalized ||
				!getBotCustomizableColorSlots(shape).includes(colorSlot))
		)
			return;
		const colors = { ...value.colors };
		if (normalized) colors[colorSlot] = normalized;
		else delete colors[colorSlot];
		update({
			colors,
			variant: "original",
			...(colorSlot === "background" ? { dark: false } : {}),
		});
	}

	function selectKind(kind: string) {
		if (!Object.hasOwn(catalog.shapes, kind)) return;
		const equipment = { ...value.equipment };
		for (const equipmentSlot of BOT_EQUIPMENT_SLOTS) {
			const itemId = equipment[equipmentSlot];
			const item =
				itemId && Object.hasOwn(catalog.equipment, itemId)
					? catalog.equipment[itemId]
					: undefined;
			if (
				!item ||
				item.slot !== equipmentSlot ||
				!isBotEquipmentCompatible(item, catalog.shapes[kind])
			)
				delete equipment[equipmentSlot];
		}
		update({ kind, equipment });
	}

	function selectEquipment(itemId?: string) {
		if (itemId) {
			const item = Object.hasOwn(catalog.equipment, itemId)
				? catalog.equipment[itemId]
				: undefined;
			if (!item || item.slot !== slot || !isBotEquipmentCompatible(item, shape))
				return;
		}
		const equipment = { ...value.equipment };
		if (itemId) equipment[slot] = itemId;
		else delete equipment[slot];
		update({ equipment });
	}

	async function save() {
		if (
			!onSave ||
			savingRef.current ||
			!getBotSaveEligibility(value, catalog, ownedPackIds).canSave
		)
			return;
		savingRef.current = true;
		setSaving(true);
		setSavingDraft(draftKey);
		setSaveError(undefined);
		try {
			await onSave(structuredClone(value));
		} catch {
			if (mountedRef.current) setSaveError({ draft: draftKey });
		} finally {
			savingRef.current = false;
			if (mountedRef.current) {
				setSaving(false);
				setSavingDraft(undefined);
			}
		}
	}

	function removeUnavailable() {
		const next = { ...value };
		if (
			saveEligibility.requirements.some(
				(item) => item.kind === "shape" && item.packIds.length === 0,
			)
		)
			next.kind = catalog.defaultKind;
		// Recheck after restoring the shape so valid colors and outfits keep their access requirements.
		for (const item of getBotSaveEligibility(next, catalog, ownedPackIds)
			.requirements) {
			if (item.packIds.length > 0 || item.kind === "shape") continue;
			if (item.kind === "equipment") {
				const current = next.equipment;
				next.equipment =
					current && typeof current === "object" && !Array.isArray(current)
						? Object.fromEntries(
								Object.entries(current).filter(
									([key]) =>
										(BOT_EQUIPMENT_SLOTS as readonly string[]).includes(key) &&
										key !== item.slot,
								),
							)
						: {};
			} else {
				const current = next.colors;
				next.colors =
					current && typeof current === "object" && !Array.isArray(current)
						? Object.fromEntries(
								Object.entries(current).filter(
									([key]) =>
										(BOT_COLOR_SLOTS as readonly string[]).includes(key) &&
										key !== item.slot,
								),
							)
						: {};
			}
		}
		update(next);
	}

	function toggleSkill(skillId: string) {
		const skills = new Set(value.skills ?? []);
		if (skills.has(skillId)) skills.delete(skillId);
		else skills.add(skillId);
		update({ skills: [...skills] });
	}

	function thumbnail(next: BotAppearance) {
		return (
			<BotAvatar
				appearance={next}
				catalog={catalog}
				size={68}
				expression={expression}
				pose="center"
				animated={false}
				decorative
			/>
		);
	}

	function shapeTile(item: BotShapeDefinition) {
		return (
			<Tile
				key={item.id}
				label={item.label}
				description={item.description}
				selected={appearance.kind === item.id}
				locked={!canUseBotShape(item.id, catalog, ownedPackIds)}
				onClick={() => selectKind(item.id)}
			>
				<CharacterThumbnail
					kind={item.id}
					catalog={catalog}
					expression={expression}
					size={68}
				/>
			</Tile>
		);
	}

	function packSection(pack: BotPackDefinition) {
		const contents = getBotPackContents(pack, catalog);
		const owned = ownedPackIds?.includes(pack.id);
		return (
			<section
				key={pack.id}
				className="fl-bot-studio-pack"
				data-pack-id={pack.id}
				aria-label={`${pack.label} pack`}
			>
				<header className="fl-bot-studio-pack-heading">
					<div className="fl-bot-studio-pack-cover" aria-hidden="true">
						<CharacterThumbnail
							kind={pack.coverKind}
							catalog={catalog}
							size={42}
							expression={expression}
						/>
					</div>
					<div className="fl-bot-studio-pack-title">
						<div>
							<h4>{pack.label}</h4>
							{ownedPackIds && (
								<span className="fl-bot-studio-pack-status" data-owned={owned}>
									{owned ? "Owned" : "Preview"}
								</span>
							)}
						</div>
						<p>{pack.description}</p>
						<span>
							{contents.shapes.length}{" "}
							{contents.shapes.length === 1 ? "character" : "characters"}
							{contents.equipment.length
								? ` · ${contents.equipment.length} outfit ${contents.equipment.length === 1 ? "item" : "items"}`
								: ""}
						</span>
					</div>
					{onRequestPack && (
						<button
							type="button"
							className="fl-bot-studio-pack-link"
							aria-label={`View ${pack.label} pack`}
							onClick={() => onRequestPack(pack)}
						>
							View pack <span aria-hidden="true">↗</span>
						</button>
					)}
				</header>
				<div className="fl-bot-studio-tiles">
					{contents.shapes.map(shapeTile)}
				</div>
			</section>
		);
	}

	return (
		<div className={["fl-bot-customizer", className].filter(Boolean).join(" ")}>
			<aside className="fl-bot-studio-preview" aria-label="Your bot preview">
				<div className="fl-bot-studio-preview-heading">
					<span className="fl-bot-studio-eyebrow">YOUR BOT</span>
					<span className="fl-bot-studio-series-tag">
						{shapeSeries?.label ?? "Custom"}
					</span>
				</div>
				<div className="fl-bot-studio-stage">
					<BotAvatar
						appearance={value}
						catalog={catalog}
						size={previewSize}
						pose={pose}
						expression={expression}
						animated={playing}
						mood={previewMood}
						agentState={previewAgentState}
						label={`${shape.label} preview`}
					/>
				</div>
				<div className="fl-bot-studio-character">
					<h3>{shape.label}</h3>
					<p>{shape.description ?? "Make this character your own."}</p>
				</div>
				<div className="fl-bot-studio-actions">
					<button type="button" aria-label="Reset bot" onClick={reset}>
						Reset
					</button>
					<button
						type="button"
						aria-label="Randomize bot"
						className="fl-bot-studio-randomize"
						disabled={locked.length === BOT_LOOK_PARTS.length}
						onClick={randomize}
					>
						<Dices aria-hidden="true" />
						{locked.length ? "Randomize rest" : "Randomize"}
					</button>
				</div>
				<fieldset
					className="fl-bot-studio-locks"
					aria-label="Keep when randomizing"
				>
					<legend>Keep when randomizing</legend>
					{BOT_LOOK_PARTS.map((part) => {
						const isLocked = locked.includes(part);
						return (
							<button
								key={part}
								type="button"
								aria-label={`Lock ${PART_LABELS[part].toLowerCase()}`}
								aria-pressed={isLocked}
								onClick={() => toggleLock(part)}
							>
								{isLocked ? (
									<Lock aria-hidden="true" />
								) : (
									<LockOpen aria-hidden="true" />
								)}
								{PART_LABELS[part]}
							</button>
						);
					})}
				</fieldset>
				<div className="fl-bot-studio-preview-controls">
					<button
						type="button"
						className="fl-bot-studio-play"
						aria-label={
							playing ? "Pause preview animation" : "Play preview animation"
						}
						aria-pressed={playing}
						onClick={() => setPlaying(!playing)}
					>
						<span aria-hidden="true">{playing ? "Ⅱ" : "▷"}</span>
						{playing ? "Pause" : "Play"}
					</button>
					<fieldset className="fl-bot-studio-poses" aria-label="Preview pose">
						{PREVIEW_POSES.map(([key, label]) => (
							<button
								key={key}
								type="button"
								aria-pressed={previewState === "custom" && pose === key}
								onClick={() => {
									setPreviewState("custom");
									setPose(key);
								}}
							>
								{label}
							</button>
						))}
					</fieldset>
					<fieldset
						className="fl-bot-studio-poses fl-bot-studio-expressions"
						aria-label="Preview expression"
					>
						<legend>Expression</legend>
						{BOT_EXPRESSIONS.map((key) => (
							<button
								key={key}
								type="button"
								aria-pressed={previewState === "custom" && expression === key}
								onClick={() => {
									setPreviewState("custom");
									setExpression(key);
								}}
							>
								{key.charAt(0).toUpperCase() + key.slice(1)}
							</button>
						))}
					</fieldset>
					<label className="fl-bot-studio-state-picker">
						<span>
							Mood or status <small>Preview only</small>
						</span>
						<select
							aria-label="Preview mood or status"
							value={previewState}
							onChange={(event) =>
								setPreviewState(event.target.value as PreviewState)
							}
						>
							<option value="custom">Custom expression</option>
							<optgroup label="Moods">
								{Object.entries(BOT_MOODS).map(([key, preset]) => (
									<option key={key} value={`mood:${key}`}>
										{preset.label}
									</option>
								))}
							</optgroup>
							<optgroup label="Agent status">
								{Object.entries(BOT_AGENT_STATES)
									.filter(([key]) => key !== "idle")
									.map(([key, preset]) => (
										<option key={key} value={`state:${key}`}>
											{preset.label}
										</option>
									))}
							</optgroup>
						</select>
					</label>
				</div>
				<div className="fl-bot-studio-outfit-summary">
					{BOT_EQUIPMENT_SLOTS.map((key) => {
						const itemId = appearance.equipment[key];
						const item = itemId ? catalog.equipment[itemId] : undefined;
						return (
							<button
								key={key}
								type="button"
								onClick={() => {
									setSlot(key);
									setSection("outfit");
								}}
								aria-label={`Edit ${SLOT_LABELS[key].toLowerCase()}: ${item?.label ?? "None"}`}
							>
								<span>{SLOT_LABELS[key]}</span>
								<strong>
									{item?.label ?? "Add"}
									<span aria-hidden="true">{item ? "›" : "+"}</span>
								</strong>
							</button>
						);
					})}
				</div>
				{(onSave || !saveEligibility.canSave) && (
					<div className="fl-bot-studio-save-area">
						{!saveEligibility.canSave && (
							<>
								<div className="fl-bot-studio-save-heading" aria-live="polite">
									<strong>Preview only</strong>
									<p>
										{hasUnavailableItems
											? saveEligibility.requirements.some(
													(item) => item.packIds.length > 0,
												)
												? "Remove unavailable items and unlock required packs to save this bot."
												: "Remove unavailable items to save this bot."
											: "Unlock required packs to save this bot."}
									</p>
								</div>
								<ul
									className="fl-bot-studio-save-requirements"
									aria-label="Save requirements"
								>
									{saveEligibility.requirements.map((item, index) => (
										<li
											key={`${item.kind}-${item.slot ?? ""}-${item.id}-${index}`}
										>
											<span className="fl-bot-studio-save-item">
												{item.label}
												{item.kind === "color" && item.slot && (
													<small>
														{item.slot === "ink"
															? "Face details"
															: item.slot === "sclera"
																? "Eye whites"
																: item.slot.charAt(0).toUpperCase() +
																	item.slot.slice(1)}{" "}
														color
													</small>
												)}
											</span>
											{item.packIds.length > 0 ? (
												<span className="fl-bot-studio-save-options">
													{item.packIds.map((packId, alternative) => {
														if (item.kind === "color") {
															const pack = catalog.colorPacks[packId];
															return (
																<span key={packId}>
																	{alternative > 0 && (
																		<span className="fl-bot-studio-save-or">
																			or
																		</span>
																	)}
																	{pack && onRequestColorPack ? (
																		<button
																			type="button"
																			onClick={() => onRequestColorPack(pack)}
																		>
																			{pack.label}
																		</button>
																	) : (
																		<span>
																			{pack?.label ?? "Unavailable pack"}
																		</span>
																	)}
																</span>
															);
														}
														const pack = catalog.packs[packId];
														return (
															<span key={packId}>
																{alternative > 0 && (
																	<span className="fl-bot-studio-save-or">
																		or
																	</span>
																)}
																{pack && onRequestPack ? (
																	<button
																		type="button"
																		onClick={() => onRequestPack(pack)}
																	>
																		{pack.label}
																	</button>
																) : (
																	<span>
																		{pack?.label ?? "Unavailable pack"}
																	</span>
																)}
															</span>
														);
													})}
												</span>
											) : (
												<small className="fl-bot-studio-save-unavailable">
													Remove or replace this unavailable item.
												</small>
											)}
										</li>
									))}
								</ul>
								{hasUnavailableItems && (
									<button
										type="button"
										className="fl-bot-studio-text-button"
										onClick={removeUnavailable}
									>
										Remove unavailable items
									</button>
								)}
							</>
						)}
						{onSave && (
							<button
								type="button"
								className="fl-bot-studio-save-button"
								aria-label="Save bot"
								disabled={saving || !saveEligibility.canSave}
								onClick={() => {
									void save();
								}}
							>
								{saving ? "Saving…" : "Save bot"}
							</button>
						)}
						{saving && savingDraft !== draftKey && (
							<p className="fl-bot-studio-save-message" aria-live="polite">
								Your latest edits are not part of this save.
							</p>
						)}
						{saveError && (
							<p className="fl-bot-studio-save-message" role="alert">
								{saveError.draft === draftKey
									? "Could not save this bot. Try again."
									: "The previous draft could not be saved. Save your latest changes to try again."}
							</p>
						)}
					</div>
				)}
			</aside>
			<div className="fl-bot-studio-workbench">
				<fieldset
					className="fl-bot-studio-sections"
					aria-label="Customization sections"
				>
					{SECTIONS.map((entry, index) => (
						<button
							key={entry.id}
							id={`${id}-${entry.id}`}
							type="button"
							aria-pressed={section === entry.id}
							aria-controls={`${id}-panel`}
							data-locked={
								(isLookPart(entry.id) && locked.includes(entry.id)) || undefined
							}
							onClick={() => setSection(entry.id)}
						>
							<span className="fl-bot-studio-step" aria-hidden="true">
								{index + 1}
							</span>
							{entry.label}
						</button>
					))}
				</fieldset>
				<section
					id={`${id}-panel`}
					aria-labelledby={`${id}-${section}`}
					className="fl-bot-studio-panel"
				>
					<header className="fl-bot-studio-panel-heading">
						<div>
							<h3>{selectedSection.hint}</h3>
							<p>
								{section === "crew"
									? packs.length
										? `${packs.length} character packs · ${shapes.length} characters`
										: `${shapes.length} characters. One that's yours.`
									: section === "outfit"
										? "Browse by theme, then try something on."
										: section === "colors"
											? "Mix colors, gradients, and patterns for each part."
											: section === "finish"
												? "See each detail before you choose."
												: "Choose the skills your bot represents."}
							</p>
						</div>
						<div className="fl-bot-studio-panel-actions">
							{section === "outfit" && (
								<button
									type="button"
									className="fl-bot-studio-text-button"
									disabled={!Object.keys(appearance.equipment).length}
									onClick={() => update({ equipment: {} })}
								>
									Reset outfit
								</button>
							)}
							{isLookPart(section) && (
								<ShuffleButton
									part={section}
									locked={locked.includes(section)}
									filtered={
										section === "crew"
											? !!query || series.kind !== "all"
											: section === "outfit" && outfitCollection.kind !== "all"
									}
									onShuffle={() => shuffle(section)}
								/>
							)}
						</div>
					</header>
					{section === "crew" && (
						<>
							<label className="fl-bot-studio-search" htmlFor={`${id}-search`}>
								<svg
									aria-hidden="true"
									width="18"
									height="18"
									viewBox="0 0 24 24"
									fill="none"
									stroke="currentColor"
									strokeWidth="1.8"
								>
									<circle cx="10.5" cy="10.5" r="6.5" />
									<path d="m16 16 5 5" />
								</svg>
								<input
									id={`${id}-search`}
									type="search"
									aria-label="Search crew"
									placeholder="Search by name or personality…"
									value={search}
									onChange={(event) => setSearch(event.target.value)}
								/>
							</label>
							<details className="fl-bot-studio-filter-drawer">
								<summary>
									Filter characters
									<span>
										{series.kind === "all"
											? "All packs"
											: series.kind === "other"
												? "Other characters"
												: series.kind === "pack"
													? catalog.packs[series.id].label
													: catalog.series[series.id].label}
									</span>
								</summary>
								<fieldset
									className="fl-bot-studio-filters"
									aria-label="Character series"
								>
									<button
										type="button"
										aria-pressed={series.kind === "all"}
										onClick={() => setSeries({ kind: "all" })}
									>
										All<span>{shapes.length}</span>
									</button>
									{seriesOptions.map((entry) => (
										<button
											key={entry.id}
											type="button"
											aria-pressed={
												series.kind === "series" && series.id === entry.id
											}
											onClick={() =>
												setSeries({ kind: "series", id: entry.id })
											}
										>
											{entry.label}
											<span>
												{
													shapes.filter((item) => item.series === entry.id)
														.length
												}
											</span>
										</button>
									))}
									{hasOther && (
										<button
											type="button"
											aria-pressed={series.kind === "other"}
											onClick={() => setSeries({ kind: "other" })}
										>
											Other
										</button>
									)}
								</fieldset>
								{packs.length > 0 && (
									<fieldset
										className="fl-bot-studio-filters"
										aria-label="Character packs"
									>
										{packs.map((pack) => (
											<button
												key={pack.id}
												type="button"
												aria-pressed={
													series.kind === "pack" && series.id === pack.id
												}
												onClick={() => setSeries({ kind: "pack", id: pack.id })}
											>
												{pack.label}
												<span>{pack.shapeIds.length}</span>
											</button>
										))}
									</fieldset>
								)}
							</details>
							<div className="fl-bot-studio-result-heading">
								<span>
									{series.kind === "all"
										? groupedCrew
											? "CHARACTER PACKS"
											: "THE WHOLE CREW"
										: series.kind === "other"
											? "OTHER CHARACTERS"
											: series.kind === "pack"
												? catalog.packs[series.id].label.toUpperCase()
												: catalog.series[series.id]?.label.toUpperCase()}
								</span>
								<span aria-live="polite">
									{visibleShapes.length}{" "}
									{visibleShapes.length === 1 ? "character" : "characters"}
								</span>
							</div>
							{selectionOutsideFilters && (
								<div className="fl-bot-studio-current-selection">
									<span>Your bot is still {shape.label}.</span>
									<button
										type="button"
										className="fl-bot-studio-text-button"
										onClick={() => {
											setSearch("");
											setSeries({ kind: "all" });
										}}
									>
										Show selected
									</button>
								</div>
							)}
							{groupedCrew ? (
								<div className="fl-bot-studio-packs">
									{packs.map(packSection)}
									{unbundledShapes.length > 0 && (
										<section
											className="fl-bot-studio-pack"
											aria-label="Other characters"
										>
											<header className="fl-bot-studio-pack-heading">
												<div className="fl-bot-studio-pack-title">
													<h4>Other characters</h4>
													<p>
														Available individually, outside a character pack.
													</p>
												</div>
											</header>
											<div className="fl-bot-studio-tiles">
												{unbundledShapes.map(shapeTile)}
											</div>
										</section>
									)}
								</div>
							) : visibleShapes.length ? (
								<div className="fl-bot-studio-tiles">
									{visibleShapes.map(shapeTile)}
								</div>
							) : (
								<div className="fl-bot-studio-empty">
									<p>No characters match your search.</p>
									<button
										type="button"
										className="fl-bot-studio-text-button"
										onClick={() => {
											setSearch("");
											setSeries({ kind: "all" });
										}}
									>
										Clear filters
									</button>
								</div>
							)}
							{!groupedCrew &&
								onRequestPack &&
								lockedVisiblePacks.length > 0 && (
									<div className="fl-bot-studio-pack-links">
										<span>Explore the packs for these characters</span>
										{lockedVisiblePacks.map((pack) => (
											<button
												key={pack.id}
												type="button"
												className="fl-bot-studio-pack-link"
												onClick={() => onRequestPack(pack)}
											>
												View {pack.label} pack <span aria-hidden="true">↗</span>
											</button>
										))}
									</div>
								)}
						</>
					)}
					{section === "outfit" && (
						<>
							<label
								className="fl-bot-studio-search"
								htmlFor={`${id}-outfit-search`}
							>
								<input
									id={`${id}-outfit-search`}
									type="search"
									aria-label="Search outfits"
									placeholder="Search clothes, accessories, or collections…"
									value={outfitSearch}
									onChange={(event) => setOutfitSearch(event.target.value)}
								/>
							</label>
							<fieldset
								className="fl-bot-studio-filters"
								aria-label="Equipment slots"
							>
								{BOT_EQUIPMENT_SLOTS.map((key) => (
									<button
										key={key}
										type="button"
										aria-pressed={slot === key}
										onClick={() => setSlot(key)}
									>
										{SLOT_LABELS[key]}
										{appearance.equipment[key] && (
											<span
												className="fl-bot-studio-dot"
												aria-label="Equipped"
											/>
										)}
									</button>
								))}
							</fieldset>
							{outfitPacks.length > 0 && (
								<details className="fl-bot-studio-filter-drawer">
									<summary>
										Filter outfits
										<span>
											{outfitCollection.kind === "all"
												? "All collections"
												: outfitCollection.kind === "other"
													? "Essentials"
													: catalog.packs[outfitCollection.id].label}
										</span>
									</summary>
									<fieldset
										className="fl-bot-studio-filters"
										aria-label="Outfit collections"
									>
										<button
											type="button"
											aria-pressed={outfitCollection.kind === "all"}
											onClick={() => setOutfitCollection({ kind: "all" })}
										>
											All collections
										</button>
										<button
											type="button"
											aria-pressed={outfitCollection.kind === "other"}
											onClick={() => setOutfitCollection({ kind: "other" })}
										>
											Essentials
										</button>
										{outfitPacks.map((pack) => (
											<button
												key={pack.id}
												type="button"
												aria-pressed={
													outfitCollection.kind === "pack" &&
													outfitCollection.id === pack.id
												}
												onClick={() =>
													setOutfitCollection({ kind: "pack", id: pack.id })
												}
											>
												{pack.label}
												<span>
													{
														availableEquipment.filter((item) =>
															pack.equipmentIds?.includes(item.id),
														).length
													}
												</span>
											</button>
										))}
									</fieldset>
								</details>
							)}
							<div className="fl-bot-studio-result-heading">
								<span>{SLOT_LABELS[slot]}</span>
								<span aria-live="polite">
									{visibleEquipment.length}{" "}
									{visibleEquipment.length === 1 ? "item" : "items"}
									{outfitGroups.length > 0 &&
										` · ${outfitGroups.length} ${outfitGroups.length === 1 ? "theme" : "themes"}`}
								</span>
							</div>
							<div className="fl-bot-studio-outfit-actions">
								<button
									type="button"
									className="fl-bot-studio-outfit-none"
									aria-label="Choose None"
									aria-pressed={!appearance.equipment[slot]}
									onClick={() => selectEquipment()}
								>
									No {SLOT_LABELS[slot].toLowerCase()}
								</button>
								<span>Color ideas appear on hover or keyboard focus.</span>
							</div>
							<div className="fl-bot-studio-outfit-groups">
								{outfitGroups.map(({ key, label, pack, equipment }) => (
									<section
										key={key}
										className="fl-bot-studio-outfit-group"
										data-outfit-group={pack?.id ?? "essentials"}
										aria-label={`${label} outfits`}
									>
										<header className="fl-bot-studio-outfit-heading">
											<h4>{label}</h4>
											<span>
												{equipment.length}{" "}
												{equipment.length === 1 ? "item" : "items"}
											</span>
										</header>
										<div className="fl-bot-studio-tiles">
											{equipment.map((item) => (
												<BotOutfitTile
													key={`${appearance.kind}:${item.id}`}
													item={item}
													appearance={outfitPreview}
													catalog={catalog}
													expression={expression}
													selected={appearance.equipment[slot] === item.id}
													locked={
														!canUseBotEquipment(item.id, catalog, ownedPackIds)
													}
													onSelect={() => selectEquipment(item.id)}
												/>
											))}
										</div>
										{pack &&
											onRequestPack &&
											equipment.some(
												(item) =>
													!canUseBotEquipment(item.id, catalog, ownedPackIds),
											) && (
												<button
													type="button"
													className="fl-bot-studio-pack-link fl-bot-studio-outfit-pack-link"
													onClick={() => onRequestPack(pack)}
												>
													View {pack.label} pack{" "}
													<span aria-hidden="true">↗</span>
												</button>
											)}
									</section>
								))}
							</div>
							{!visibleEquipment.length && (
								<div className="fl-bot-studio-empty">
									<p>
										{availableEquipment.length
											? "No items match these filters in this slot."
											: "No items for this character in this slot yet."}
									</p>
									{(outfitQuery || outfitCollection.kind !== "all") && (
										<button
											type="button"
											className="fl-bot-studio-text-button"
											onClick={() => {
												setOutfitSearch("");
												setOutfitCollection({ kind: "all" });
											}}
										>
											Clear outfit filters
										</button>
									)}
								</div>
							)}
						</>
					)}
					{section === "colors" && (
						<>
							{colorEffect && (
								<div className="fl-bot-studio-color-notice">
									<div>
										<strong>{colorEffect} controls this palette</strong>
										<p>
											This effect sets the colors of your bot and outfit. Remove
											it to use your chosen colors.
										</p>
										<button
											type="button"
											className="fl-bot-studio-text-button"
											onClick={() => update({ effect: "none" })}
										>
											Remove color effect
										</button>
									</div>
								</div>
							)}
							{appearance.variant !== "original" && (
								<div className="fl-bot-studio-legacy-color">
									<p>
										A saved{" "}
										{appearance.variant === "holo"
											? "holographic"
											: appearance.variant}{" "}
										finish is active. Choosing a color restores the original
										finish.
									</p>
									<button
										type="button"
										className="fl-bot-studio-text-button"
										onClick={() => update({ variant: "original" })}
									>
										Reset legacy finish
									</button>
								</div>
							)}
							<BotColorPicker
								key={appearance.kind}
								shape={shape}
								catalog={catalog}
								value={value.colors}
								ownedPackIds={ownedPackIds}
								onChange={selectColor}
								onReset={() =>
									update({ colors: {}, variant: "original", dark: false })
								}
								onRequestPack={onRequestColorPack}
								disabled={!!colorEffect}
							/>
						</>
					)}
					{section === "finish" && (
						<>
							<fieldset
								className="fl-bot-studio-filters"
								aria-label="Finish categories"
							>
								{FINISH_GROUPS.map((group) => (
									<button
										key={group.id}
										type="button"
										aria-pressed={finish === group.id}
										onClick={() => setFinish(group.id)}
									>
										{group.label}
									</button>
								))}
							</fieldset>
							{finish === "effect" ? (
								<div className="fl-bot-studio-effect-groups">
									{BOT_EFFECT_GROUPS.map((group) => (
										<section
											key={group.id}
											data-effect-group={group.id}
											aria-label={`${group.label} effects`}
										>
											<div className="fl-bot-studio-effect-heading">
												<h3>{group.label}</h3>
												<p>{group.description}</p>
											</div>
											<div className="fl-bot-studio-tiles">
												{group.effects.map((effect) => (
													<Tile
														key={effect.id}
														label={effect.label}
														description={effect.description}
														showDescription
														selected={appearance.effect === effect.id}
														onClick={() => update({ effect: effect.id })}
													>
														{thumbnail({ ...appearance, effect: effect.id })}
													</Tile>
												))}
											</div>
										</section>
									))}
								</div>
							) : (
								<div className="fl-bot-studio-tiles">
									{finishGroup.options.map(([key, label]) => (
										<Tile
											key={`${finish}-${key}`}
											label={label}
											selected={appearance[finish] === key}
											onClick={() => update({ [finish]: key })}
										>
											{thumbnail({
												...appearance,
												skills: [],
												emote: "none",
												[finish]: key,
											})}
										</Tile>
									))}
								</div>
							)}
							<label className="fl-bot-studio-dark">
								<span>
									<strong>Dark background</strong>
									<small>
										{colorEffect || appearance.effect === "neon"
											? "This effect sets its own background."
											: "A deeper canvas for your bot."}
									</small>
								</span>
								<input
									type="checkbox"
									checked={appearance.dark}
									disabled={!!colorEffect || appearance.effect === "neon"}
									onChange={(event) => update({ dark: event.target.checked })}
								/>
							</label>
						</>
					)}
					{section === "skills" && (
						<>
							<div className="fl-bot-studio-result-heading">
								<span>SKILL BADGES</span>
								<span aria-live="polite">
									{appearance.skills.length} selected
								</span>
							</div>
							<div className="fl-bot-studio-skills">
								{Object.values(catalog.skills).map((skill) => (
									<button
										key={skill.id}
										type="button"
										aria-pressed={appearance.skills.includes(skill.id)}
										onClick={() => toggleSkill(skill.id)}
									>
										<span
											className="fl-bot-studio-skill-glyph"
											style={{ backgroundColor: skill.color }}
											aria-hidden="true"
										>
											{skill.glyph}
										</span>
										<span>{skill.label}</span>
										<span
											className="fl-bot-studio-skill-check"
											aria-hidden="true"
										>
											{appearance.skills.includes(skill.id) ? "✓" : "+"}
										</span>
									</button>
								))}
							</div>
							<p className="fl-bot-studio-hint">
								Your selected skills appear beside your bot. Extra badges fold
								into a count at smaller sizes.
							</p>
						</>
					)}
				</section>
			</div>
		</div>
	);
}
