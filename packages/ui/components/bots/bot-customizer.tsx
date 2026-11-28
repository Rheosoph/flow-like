"use client";

import { useTranslation } from "@flow-like/locales";
import type { TFunction } from "i18next";
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

function getSections(t: TFunction<"bots">) {
	return [
		{
			id: "crew",
			label: t("appearance.crew"),
			hint: t("appearance.findYourCharacter"),
		},
		{
			id: "outfit",
			label: t("appearance.outfit"),
			hint: t("appearance.giveThemAStyle"),
		},
		{
			id: "colors",
			label: t("appearance.colors"),
			hint: t("appearance.makeThePaletteYours"),
		},
		{
			id: "finish",
			label: t("appearance.finish"),
			hint: t("appearance.addALittlePersonality"),
		},
		{
			id: "skills",
			label: t("appearance.skills"),
			hint: t("appearance.showWhatTheyDo"),
		},
	] as const;
}

type Section = ReturnType<typeof getSections>[number]["id"];
type SeriesFilter =
	| { kind: "all" }
	| { kind: "other" }
	| { kind: "pack"; id: string }
	| { kind: "series"; id: string };
function getSlotLabels(t: TFunction<"bots">) {
	return {
		neck: t("appearance.clothing"),
		eyes: t("appearance.eyewear"),
		hand: t("appearance.handAccessory"),
		head: t("appearance.headwear"),
	};
}

function getFinishGroups(t: TFunction<"bots">) {
	return [
		{
			id: "backdrop",
			label: t("appearance.pattern"),
			options: [
				["none", t("appearance.none")],
				["gradient", t("appearance.gradient")],
				["dots", t("appearance.dots")],
				["stripes", t("appearance.stripes")],
				["waves", t("appearance.waves")],
				["rays", t("appearance.rays")],
				["stars", t("appearance.stars")],
				["confetti", t("appearance.confetti")],
			],
		},
		{
			id: "halo",
			label: t("appearance.ring"),
			options: [
				["none", t("appearance.none")],
				["flow", t("appearance.flow")],
				["gold", t("appearance.gold")],
				["aurora", t("appearance.aurora")],
				["pulse", t("appearance.pulse")],
				["orbit", t("appearance.orbit")],
			],
		},
		{
			id: "effect",
			label: t("appearance.effect"),
			options: BOT_EFFECTS.map(
				(effect) =>
					[effect.id, t(`appearance.effects.${effect.id}.label`)] as const,
			),
		},
		{
			id: "frame",
			label: t("appearance.frame"),
			options: [
				["circle", t("appearance.circle")],
				["squircle", t("appearance.squircle")],
				["square", t("appearance.square")],
			],
		},
		{
			id: "emote",
			label: t("appearance.emote"),
			options: [
				["none", t("appearance.none")],
				["heart", t("appearance.heart")],
				["note", t("appearance.musicNote")],
				["spark", t("appearance.spark")],
				["alert", t("appearance.exclamation")],
				["question", t("appearance.question")],
				["drop", t("appearance.drop")],
				["zzz", t("appearance.sleep")],
				["rain", t("appearance.rain")],
				["anger", t("appearance.anger")],
				["loading", t("appearance.working")],
				["idea", t("appearance.idea")],
				["check", t("appearance.done")],
				["error", t("appearance.error")],
			],
		},
	] as const;
}

type Finish = ReturnType<typeof getFinishGroups>[number]["id"];
type PreviewState =
	| "custom"
	| `mood:${BotMood}`
	| `state:${Exclude<BotAgentState, "idle">}`;
function getPreviewPoses(t: TFunction<"bots">) {
	return [
		["idle", t("appearance.idle")],
		["sleep", t("appearance.sleep")],
		["track", t("appearance.followCursor")],
	] as const;
}

function getPartLabels(t: TFunction<"bots">) {
	return {
		crew: t("appearance.character"),
		outfit: t("appearance.outfit"),
		colors: t("appearance.colors"),
		finish: t("appearance.finish"),
	};
}

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
	const { t } = useTranslation("bots");
	return (
		<button
			type="button"
			className="fl-bot-studio-tile"
			aria-label={t("appearance.chooseItem", { label })}
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
			{locked && (
				<span className="fl-bot-studio-tile-locked">
					{t("appearance.preview")}
				</span>
			)}
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
	const { t } = useTranslation("bots");

	return (
		<button
			type="button"
			className="fl-bot-studio-shuffle"
			aria-label={t(`appearance.shufflePart.${part}`)}
			disabled={locked}
			title={
				locked
					? t(`appearance.unlockPart.${part}`)
					: filtered
						? part === "crew"
							? t("appearance.shuffleShownCharacters")
							: t("appearance.shuffleShownCollection")
						: undefined
			}
			onClick={onShuffle}
		>
			{locked ? <Lock aria-hidden="true" /> : <Dices aria-hidden="true" />}
			{locked
				? t("appearance.locked")
				: filtered
					? t("appearance.shuffleShown")
					: t("appearance.shuffle")}
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
	const { t } = useTranslation("bots");
	const SECTIONS = getSections(t);
	const SLOT_LABELS = getSlotLabels(t);
	const FINISH_GROUPS = getFinishGroups(t);
	const PREVIEW_POSES = getPreviewPoses(t);
	const PART_LABELS = getPartLabels(t);
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
			label: t("appearance.essentials"),
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
			? t("appearance.lcd")
			: appearance.effect === "blueprint"
				? t("appearance.blueprint")
				: appearance.effect === "thermal"
					? t("appearance.thermal")
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
				aria-label={t("appearance.namedPack", { label: pack.label })}
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
									{owned ? t("appearance.owned") : t("appearance.preview")}
								</span>
							)}
						</div>
						<p>{pack.description}</p>
						<span>
							{contents.equipment.length
								? t("appearance.packContents", {
										characters: t("appearance.characterCount", {
											count: contents.shapes.length,
										}),
										outfits: t("appearance.outfitItemCount", {
											count: contents.equipment.length,
										}),
									})
								: t("appearance.characterCount", {
										count: contents.shapes.length,
									})}
						</span>
					</div>
					{onRequestPack && (
						<button
							type="button"
							className="fl-bot-studio-pack-link"
							aria-label={t("appearance.viewNamedPack", { label: pack.label })}
							onClick={() => onRequestPack(pack)}
						>
							{t("appearance.viewPack")} <span aria-hidden="true">↗</span>
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
			<aside
				className="fl-bot-studio-preview"
				aria-label={t("appearance.yourBotPreview")}
			>
				<div className="fl-bot-studio-preview-heading">
					<span className="fl-bot-studio-eyebrow">
						{t("appearance.yourBOT")}
					</span>
					<span className="fl-bot-studio-series-tag">
						{shapeSeries?.label ?? t("appearance.custom")}
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
						label={t("appearance.characterPreview", { label: shape.label })}
					/>
				</div>
				<div className="fl-bot-studio-character">
					<h3>{shape.label}</h3>
					<p>{shape.description ?? t("appearance.makeThisCharacterYourOwn")}</p>
				</div>
				<div className="fl-bot-studio-actions">
					<button
						type="button"
						aria-label={t("appearance.resetBot")}
						onClick={reset}
					>
						{t("appearance.reset")}
					</button>
					<button
						type="button"
						aria-label={t("appearance.randomizeBot")}
						className="fl-bot-studio-randomize"
						disabled={locked.length === BOT_LOOK_PARTS.length}
						onClick={randomize}
					>
						<Dices aria-hidden="true" />
						{locked.length
							? t("appearance.randomizeRest")
							: t("appearance.randomize")}
					</button>
				</div>
				<fieldset
					className="fl-bot-studio-locks"
					aria-label={t("appearance.keepWhenRandomizing")}
				>
					<legend>{t("appearance.keepWhenRandomizing")}</legend>
					{BOT_LOOK_PARTS.map((part) => {
						const isLocked = locked.includes(part);
						return (
							<button
								key={part}
								type="button"
								aria-label={t(`appearance.lockPart.${part}`)}
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
							playing
								? t("appearance.pausePreviewAnimation")
								: t("appearance.playPreviewAnimation")
						}
						aria-pressed={playing}
						onClick={() => setPlaying(!playing)}
					>
						<span aria-hidden="true">{playing ? "Ⅱ" : "▷"}</span>
						{playing ? t("appearance.pause") : t("appearance.play")}
					</button>
					<fieldset
						className="fl-bot-studio-poses"
						aria-label={t("appearance.previewPose")}
					>
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
						aria-label={t("appearance.previewExpression")}
					>
						<legend>{t("appearance.expression")}</legend>
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
								{t(`appearance.expressions.${key}`)}
							</button>
						))}
					</fieldset>
					<label className="fl-bot-studio-state-picker">
						<span>
							{t("appearance.moodOrStatus")}{" "}
							<small>{t("appearance.previewOnly")}</small>
						</span>
						<select
							aria-label={t("appearance.previewMoodOrStatus")}
							value={previewState}
							onChange={(event) =>
								setPreviewState(event.target.value as PreviewState)
							}
						>
							<option value="custom">{t("appearance.customExpression")}</option>
							<optgroup label={t("appearance.moodsLabel")}>
								{(Object.keys(BOT_MOODS) as BotMood[]).map((key) => (
									<option key={key} value={`mood:${key}`}>
										{t(`appearance.moods.${key}`)}
									</option>
								))}
							</optgroup>
							<optgroup label={t("appearance.agentStatus")}>
								{(Object.keys(BOT_AGENT_STATES) as BotAgentState[])
									.filter((key) => key !== "idle")
									.map((key) => (
										<option key={key} value={`state:${key}`}>
											{t(`appearance.agentStates.${key}`)}
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
								aria-label={t(`appearance.editEquipment.${key}`, {
									item: item?.label ?? t("appearance.none"),
								})}
							>
								<span>{SLOT_LABELS[key]}</span>
								<strong>
									{item?.label ?? t("appearance.add")}
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
									<strong>{t("appearance.previewOnly")}</strong>
									<p>
										{hasUnavailableItems
											? saveEligibility.requirements.some(
													(item) => item.packIds.length > 0,
												)
												? t(
														"appearance.removeUnavailableItemsAndUnlockRequiredPacksToSaveThisBot",
													)
												: t("appearance.removeUnavailableItemsToSaveThisBot")
											: t("appearance.unlockRequiredPacksToSaveThisBot")}
									</p>
								</div>
								<ul
									className="fl-bot-studio-save-requirements"
									aria-label={t("appearance.saveRequirements")}
								>
									{saveEligibility.requirements.map((item, index) => (
										<li
											key={`${item.kind}-${item.slot ?? ""}-${item.id}-${index}`}
										>
											<span className="fl-bot-studio-save-item">
												{item.label}
												{item.kind === "color" && item.slot && (
													<small>
														{t("appearance.slotColor", {
															slot: t(`appearance.colorRoles.${item.slot}`),
														})}
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
																			{t("appearance.or")}
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
																			{pack?.label ??
																				t("appearance.unavailablePack")}
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
																		{t("appearance.or")}
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
																		{pack?.label ??
																			t("appearance.unavailablePack")}
																	</span>
																)}
															</span>
														);
													})}
												</span>
											) : (
												<small className="fl-bot-studio-save-unavailable">
													{t("appearance.removeOrReplaceThisUnavailableItem")}
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
										{t("appearance.removeUnavailableItems")}
									</button>
								)}
							</>
						)}
						{onSave && (
							<button
								type="button"
								className="fl-bot-studio-save-button"
								aria-label={t("appearance.saveBot")}
								disabled={saving || !saveEligibility.canSave}
								onClick={() => {
									void save();
								}}
							>
								{saving ? t("appearance.saving") : t("appearance.saveBot")}
							</button>
						)}
						{saving && savingDraft !== draftKey && (
							<p className="fl-bot-studio-save-message" aria-live="polite">
								{t("appearance.yourLatestEditsAreNotPartOfThisSave")}
							</p>
						)}
						{saveError && (
							<p className="fl-bot-studio-save-message" role="alert">
								{saveError.draft === draftKey
									? t("appearance.couldNotSaveThisBotTryAgain")
									: t(
											"appearance.thePreviousDraftCouldNotBeSavedSaveYourLatestChangesToTryAgain",
										)}
							</p>
						)}
					</div>
				)}
			</aside>
			<div className="fl-bot-studio-workbench">
				<fieldset
					className="fl-bot-studio-sections"
					aria-label={t("appearance.customizationSections")}
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
										? t("appearance.crewContents", {
												packs: t("appearance.packCount", {
													count: packs.length,
												}),
												characters: t("appearance.characterCount", {
													count: shapes.length,
												}),
											})
										: t("appearance.chooseCharacterCount", {
												count: shapes.length,
											})
									: section === "outfit"
										? t("appearance.browseByThemeThenTrySomethingOn")
										: section === "colors"
											? t("appearance.mixColorsGradientsAndPatternsForEachPart")
											: section === "finish"
												? t("appearance.seeEachDetailBeforeYouChoose")
												: t("appearance.chooseTheSkillsYourBotRepresents")}
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
									{t("appearance.resetOutfit")}
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
									aria-label={t("appearance.searchCrew")}
									placeholder={t("appearance.searchByNameOrPersonality")}
									value={search}
									onChange={(event) => setSearch(event.target.value)}
								/>
							</label>
							<details className="fl-bot-studio-filter-drawer">
								<summary>
									{t("appearance.filterCharacters")}{" "}
									<span>
										{series.kind === "all"
											? t("appearance.allPacks")
											: series.kind === "other"
												? t("appearance.otherCharacters")
												: series.kind === "pack"
													? catalog.packs[series.id].label
													: catalog.series[series.id].label}
									</span>
								</summary>
								<fieldset
									className="fl-bot-studio-filters"
									aria-label={t("appearance.characterSeries")}
								>
									<button
										type="button"
										aria-pressed={series.kind === "all"}
										onClick={() => setSeries({ kind: "all" })}
									>
										{t("appearance.all")}
										<span>{shapes.length}</span>
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
											{t("appearance.other")}
										</button>
									)}
								</fieldset>
								{packs.length > 0 && (
									<fieldset
										className="fl-bot-studio-filters"
										aria-label={t("appearance.characterPacks")}
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
											? t("appearance.characterPacksHeading")
											: t("appearance.theWHOLECREW")
										: series.kind === "other"
											? t("appearance.otherCharactersHeading")
											: series.kind === "pack"
												? catalog.packs[series.id].label.toUpperCase()
												: catalog.series[series.id]?.label.toUpperCase()}
								</span>
								<span aria-live="polite">
									{t("appearance.characterCount", {
										count: visibleShapes.length,
									})}
								</span>
							</div>
							{selectionOutsideFilters && (
								<div className="fl-bot-studio-current-selection">
									<span>
										{t("appearance.selectedCharacter", { label: shape.label })}
									</span>
									<button
										type="button"
										className="fl-bot-studio-text-button"
										onClick={() => {
											setSearch("");
											setSeries({ kind: "all" });
										}}
									>
										{t("appearance.showSelected")}
									</button>
								</div>
							)}
							{groupedCrew ? (
								<div className="fl-bot-studio-packs">
									{packs.map(packSection)}
									{unbundledShapes.length > 0 && (
										<section
											className="fl-bot-studio-pack"
											aria-label={t("appearance.otherCharacters")}
										>
											<header className="fl-bot-studio-pack-heading">
												<div className="fl-bot-studio-pack-title">
													<h4>{t("appearance.otherCharacters")}</h4>
													<p>
														{t(
															"appearance.availableIndividuallyOutsideACharacterPack",
														)}
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
									<p>{t("appearance.noCharactersMatchYourSearch")}</p>
									<button
										type="button"
										className="fl-bot-studio-text-button"
										onClick={() => {
											setSearch("");
											setSeries({ kind: "all" });
										}}
									>
										{t("appearance.clearFilters")}
									</button>
								</div>
							)}
							{!groupedCrew &&
								onRequestPack &&
								lockedVisiblePacks.length > 0 && (
									<div className="fl-bot-studio-pack-links">
										<span>
											{t("appearance.exploreThePacksForTheseCharacters")}
										</span>
										{lockedVisiblePacks.map((pack) => (
											<button
												key={pack.id}
												type="button"
												className="fl-bot-studio-pack-link"
												onClick={() => onRequestPack(pack)}
											>
												{t("appearance.viewNamedPack", { label: pack.label })}{" "}
												<span aria-hidden="true">↗</span>
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
									aria-label={t("appearance.searchOutfits")}
									placeholder={t(
										"appearance.searchClothesAccessoriesOrCollections",
									)}
									value={outfitSearch}
									onChange={(event) => setOutfitSearch(event.target.value)}
								/>
							</label>
							<fieldset
								className="fl-bot-studio-filters"
								aria-label={t("appearance.equipmentSlots")}
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
												aria-label={t("appearance.equipped")}
											/>
										)}
									</button>
								))}
							</fieldset>
							{outfitPacks.length > 0 && (
								<details className="fl-bot-studio-filter-drawer">
									<summary>
										{t("appearance.filterOutfits")}{" "}
										<span>
											{outfitCollection.kind === "all"
												? t("appearance.allCollections")
												: outfitCollection.kind === "other"
													? t("appearance.essentials")
													: catalog.packs[outfitCollection.id].label}
										</span>
									</summary>
									<fieldset
										className="fl-bot-studio-filters"
										aria-label={t("appearance.outfitCollections")}
									>
										<button
											type="button"
											aria-pressed={outfitCollection.kind === "all"}
											onClick={() => setOutfitCollection({ kind: "all" })}
										>
											{t("appearance.allCollections")}
										</button>
										<button
											type="button"
											aria-pressed={outfitCollection.kind === "other"}
											onClick={() => setOutfitCollection({ kind: "other" })}
										>
											{t("appearance.essentials")}
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
									{outfitGroups.length > 0
										? t("appearance.outfitContents", {
												items: t("appearance.itemCount", {
													count: visibleEquipment.length,
												}),
												themes: t("appearance.themeCount", {
													count: outfitGroups.length,
												}),
											})
										: t("appearance.itemCount", {
												count: visibleEquipment.length,
											})}
								</span>
							</div>
							<div className="fl-bot-studio-outfit-actions">
								<button
									type="button"
									className="fl-bot-studio-outfit-none"
									aria-label={t("appearance.chooseNone")}
									aria-pressed={!appearance.equipment[slot]}
									onClick={() => selectEquipment()}
								>
									{t(`appearance.noEquipment.${slot}`)}
								</button>
								<span>
									{t("appearance.colorIdeasAppearOnHoverOrKeyboardFocus")}
								</span>
							</div>
							<div className="fl-bot-studio-outfit-groups">
								{outfitGroups.map(({ key, label, pack, equipment }) => (
									<section
										key={key}
										className="fl-bot-studio-outfit-group"
										data-outfit-group={pack?.id ?? "essentials"}
										aria-label={t("appearance.namedOutfits", { label })}
									>
										<header className="fl-bot-studio-outfit-heading">
											<h4>{label}</h4>
											<span>
												{t("appearance.itemCount", { count: equipment.length })}
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
													{t("appearance.viewNamedPack", { label: pack.label })}{" "}
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
											? t("appearance.noItemsMatchTheseFiltersInThisSlot")
											: t("appearance.noItemsForThisCharacterInThisSlotYet")}
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
											{t("appearance.clearOutfitFilters")}
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
										<strong>
											{t("appearance.effectControlsPalette", {
												effect: colorEffect,
											})}
										</strong>
										<p>
											{t(
												"appearance.thisEffectSetsTheColorsOfYourBotAndOutfitRemoveItToUseYourChosenColors",
											)}
										</p>
										<button
											type="button"
											className="fl-bot-studio-text-button"
											onClick={() => update({ effect: "none" })}
										>
											{t("appearance.removeColorEffect")}
										</button>
									</div>
								</div>
							)}
							{appearance.variant !== "original" && (
								<div className="fl-bot-studio-legacy-color">
									<p>
										{t("appearance.savedFinishActive", {
											finish: t(`appearance.variants.${appearance.variant}`),
										})}
									</p>
									<button
										type="button"
										className="fl-bot-studio-text-button"
										onClick={() => update({ variant: "original" })}
									>
										{t("appearance.resetLegacyFinish")}
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
								aria-label={t("appearance.finishCategories")}
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
											aria-label={t("appearance.namedEffects", {
												label: t(`appearance.effectGroups.${group.id}.label`),
											})}
										>
											<div className="fl-bot-studio-effect-heading">
												<h3>
													{t(`appearance.effectGroups.${group.id}.label`)}
												</h3>
												<p>
													{t(`appearance.effectGroups.${group.id}.description`)}
												</p>
											</div>
											<div className="fl-bot-studio-tiles">
												{group.effects.map((effect) => (
													<Tile
														key={effect.id}
														label={t(`appearance.effects.${effect.id}.label`)}
														description={t(
															`appearance.effects.${effect.id}.description`,
														)}
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
									<strong>{t("appearance.darkBackground")}</strong>
									<small>
										{colorEffect || appearance.effect === "neon"
											? t("appearance.thisEffectSetsItsOwnBackground")
											: t("appearance.aDeeperCanvasForYourBot")}
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
								<span>{t("appearance.skillBADGES")}</span>
								<span aria-live="polite">
									{t("appearance.selectedCount", {
										count: appearance.skills.length,
									})}
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
								{t(
									"appearance.yourSelectedSkillsAppearBesideYourBotExtraBadgesFoldIntoACountAtSmallerSizes",
								)}
							</p>
						</>
					)}
				</section>
			</div>
		</div>
	);
}
