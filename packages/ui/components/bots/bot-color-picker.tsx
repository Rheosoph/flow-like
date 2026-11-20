"use client";

import { useTranslation } from "@flow-like/locales";
import type { TFunction } from "i18next";
import { useEffect, useId, useState } from "react";
import { BotPaintDefs, getBotPaintId } from "./bot-paints";
import {
	canUseBotColor,
	getBotColorBase,
	getBotColorPacks,
	getBotColorSelectionPaint,
	getBotCustomizableColorSlots,
	normalizeBotColorSelection,
	normalizeBotHex,
} from "./colors";
import type {
	BotAppearance,
	BotCatalog,
	BotColorPackDefinition,
	BotColorSelection,
	BotColorSlot,
	BotPaint,
	BotShapeDefinition,
} from "./types";
import "./bot-color-picker.css";

function getRoleLabels(t: TFunction<"bots">): Record<BotColorSlot, string> {
	return {
		body: t("appearance.body"),
		accent: t("appearance.accent"),
		background: t("appearance.background"),
		ink: t("appearance.faceDetails"),
		sclera: t("appearance.eyeWhites"),
		cheek: t("appearance.cheeks"),
		clothing: t("appearance.clothing"),
	};
}

export interface BotColorPickerProps {
	shape: BotShapeDefinition;
	catalog: BotCatalog;
	value?: BotAppearance["colors"];
	/** Omit to make every color available for saving. */
	ownedPackIds?: readonly string[];
	onChange: (
		slot: BotColorSlot,
		selection: BotColorSelection | undefined,
	) => void;
	onReset: () => void;
	onRequestPack?: (pack: BotColorPackDefinition) => void;
	/** Palette effects may temporarily hide these colors; resets stay available. */
	disabled?: boolean;
}

/** Uses the same paint definitions and density as the bot, with motion paused. */
export function BotColorSwatch({ paint }: { paint: BotPaint }) {
	const id = `bot-color-${useId().replace(/:/g, "")}`;
	return (
		<svg
			className="fl-bot-color-swatch"
			viewBox="0 0 100 100"
			aria-hidden="true"
			focusable="false"
		>
			{paint.kind === "solid" ? (
				<rect width="100" height="100" fill={paint.color} />
			) : (
				<>
					<defs>
						<BotPaintDefs
							idPrefix={id}
							paints={{
								body:
									paint.kind === "gradient"
										? { ...paint, animated: false }
										: paint,
							}}
						/>
					</defs>
					<rect
						width="100"
						height="100"
						fill={`url(#${getBotPaintId(id, "body")})`}
					/>
				</>
			)}
		</svg>
	);
}

function CustomHex({
	slot,
	pack,
	initialColor,
	onChange,
}: {
	slot: BotColorSlot;
	pack: BotColorPackDefinition;
	initialColor: string;
	onChange: (selection: BotColorSelection) => void;
}) {
	const { t } = useTranslation("bots");
	const ROLE_LABELS = getRoleLabels(t);
	const [draft, setDraft] = useState(initialColor);
	const id = useId();
	const normalized = normalizeBotHex(draft);
	useEffect(() => setDraft(initialColor), [initialColor]);
	function apply(value = draft) {
		const hex = normalizeBotHex(value);
		if (!hex) return;
		setDraft(hex);
		onChange({ kind: "hex", value: hex, packId: pack.id });
	}
	return (
		<div className="fl-bot-color-custom">
			<label className="fl-bot-color-native">
				<span>{t("appearance.pickAColor")}</span>
				<input
					type="color"
					aria-label={t("appearance.pickCustomColor", {
						slot: ROLE_LABELS[slot],
					})}
					value={normalized ?? initialColor}
					onChange={(event) => apply(event.target.value)}
				/>
			</label>
			<label className="fl-bot-color-hex" htmlFor={`${id}-hex`}>
				<span>{t("appearance.hexColor")}</span>
				<input
					id={`${id}-hex`}
					type="text"
					aria-label={t("appearance.hexColorForSlot", {
						slot: ROLE_LABELS[slot],
					})}
					aria-describedby={`${id}-help`}
					aria-invalid={!normalized}
					autoComplete="off"
					spellCheck={false}
					placeholder="#AABBCC"
					value={draft}
					onChange={(event) => setDraft(event.target.value)}
					onKeyDown={(event) => {
						if (event.key === "Enter") {
							event.preventDefault();
							apply();
						}
					}}
				/>
			</label>
			<button
				type="button"
				className="fl-bot-color-apply"
				disabled={!normalized}
				onClick={() => apply()}
			>
				{t("appearance.applyCustomColor")}
			</button>
			<p id={`${id}-help`} className="fl-bot-color-help">
				{normalized
					? t("appearance.useRGBOrRRGGBB")
					: t("appearance.enterAValid3Or6DigitHexColor")}
			</p>
		</div>
	);
}

export function BotColorPicker({
	shape,
	catalog,
	value,
	ownedPackIds,
	onChange,
	onReset,
	onRequestPack,
	disabled = false,
}: BotColorPickerProps) {
	const { t } = useTranslation("bots");
	const ROLE_LABELS = getRoleLabels(t);
	const slots = getBotCustomizableColorSlots(shape);
	const packs = Object.values(catalog.colorPacks ?? {});
	const [openSlot, setOpenSlot] = useState<BotColorSlot | undefined>(slots[0]);
	const [requestedPackId, setPackId] = useState<string | undefined>(
		packs.find((pack) => pack.access === "included")?.id ?? packs[0]?.id,
	);
	const activePack =
		packs.find((pack) => pack.id === requestedPackId) ?? packs[0];
	const id = useId();
	const activeSlot =
		openSlot && slots.includes(openSlot) ? openSlot : undefined;

	function select(slot: BotColorSlot, selection: BotColorSelection) {
		if (disabled || !slots.includes(slot)) return;
		const normalized = normalizeBotColorSelection(selection, catalog);
		if (normalized && getBotColorSelectionPaint(normalized, catalog))
			onChange(slot, normalized);
	}

	function toggle(slot: BotColorSlot) {
		setOpenSlot(activeSlot === slot ? undefined : slot);
		const selection = value?.[slot];
		const packId =
			selection?.kind === "hex"
				? selection.packId
				: selection?.kind === "swatch"
					? getBotColorPacks(selection.colorId, catalog)[0]?.id
					: undefined;
		if (packId) setPackId(packId);
	}

	return (
		<div className="fl-bot-color-picker">
			<div className="fl-bot-color-heading">
				<p>
					{t(
						"appearance.chooseAColorForEachPartSharedColorsAlsoTintMatchingDetailsAndEquipment",
					)}
				</p>
				<button
					type="button"
					className="fl-bot-studio-text-button"
					disabled={!Object.keys(value ?? {}).length}
					onClick={onReset}
				>
					{t("appearance.resetColors")}
				</button>
			</div>
			{!slots.length && (
				<p className="fl-bot-color-help">
					{t("appearance.thisCharacterKeepsItsOriginalColors")}
				</p>
			)}
			<div className="fl-bot-color-roles">
				{slots.map((slot) => {
					const selected = value?.[slot];
					const paint = selected
						? getBotColorSelectionPaint(selected, catalog)
						: undefined;
					const shownPaint: BotPaint = paint ?? {
						kind: "solid",
						color: shape.palette[slot],
					};
					const selectedSwatch =
						selected?.kind === "swatch" &&
						Object.hasOwn(catalog.colors ?? {}, selected.colorId)
							? catalog.colors[selected.colorId]
							: undefined;
					const label =
						selectedSwatch?.label ??
						(selected?.kind === "hex"
							? selected.value
							: t("appearance.original"));
					const previewOnly =
						!!selected && !canUseBotColor(selected, catalog, ownedPackIds);
					const expanded = activeSlot === slot;
					const packOwned =
						ownedPackIds === undefined ||
						activePack?.access === "included" ||
						!!(activePack && ownedPackIds?.includes(activePack.id));
					const hasAvailableColors = activePack?.colorIds.some((colorId) =>
						canUseBotColor({ kind: "swatch", colorId }, catalog, ownedPackIds),
					);
					return (
						<div key={slot} className="fl-bot-color-role" data-open={expanded}>
							<div className="fl-bot-color-role-heading">
								<button
									type="button"
									className="fl-bot-color-role-toggle"
									aria-label={t("appearance.editColor", {
										slot: ROLE_LABELS[slot],
									})}
									aria-expanded={expanded}
									aria-controls={`${id}-${slot}`}
									onClick={() => toggle(slot)}
								>
									<span className="fl-bot-color-current">
										<BotColorSwatch paint={shownPaint} />
									</span>
									<span className="fl-bot-color-role-name">
										<strong>{ROLE_LABELS[slot]}</strong>
										<small>
											{previewOnly
												? t("appearance.previewColor", { label })
												: label}
										</small>
									</span>
									<span aria-hidden="true" className="fl-bot-color-expand">
										{expanded ? "−" : "+"}
									</span>
								</button>
								<button
									type="button"
									className="fl-bot-color-original"
									aria-label={t("appearance.resetColor", {
										slot: ROLE_LABELS[slot],
									})}
									disabled={!selected}
									onClick={() => onChange(slot, undefined)}
								>
									{t("appearance.original")}
								</button>
							</div>
							{expanded && (
								<section
									id={`${id}-${slot}`}
									className="fl-bot-color-editor"
									aria-label={t("appearance.slotColors", {
										slot: ROLE_LABELS[slot],
									})}
								>
									<fieldset
										className="fl-bot-color-packs"
										aria-label={t("appearance.colorPacks")}
									>
										{packs.map((pack) => {
											const owned =
												pack.access === "premium" &&
												ownedPackIds?.includes(pack.id);
											return (
												<button
													key={pack.id}
													type="button"
													aria-label={t("appearance.namedColorPack", {
														label: pack.label,
													})}
													aria-pressed={activePack?.id === pack.id}
													onClick={() => setPackId(pack.id)}
												>
													<span
														className="fl-bot-color-pack-preview"
														aria-hidden="true"
													>
														{pack.colorIds.slice(0, 3).map((colorId) =>
															Object.hasOwn(catalog.colors, colorId) ? (
																<span key={colorId}>
																	<BotColorSwatch
																		paint={catalog.colors[colorId].paint}
																	/>
																</span>
															) : null,
														)}
														{pack.allowsCustomHex && (
															<span className="fl-bot-color-custom-mark">
																#
															</span>
														)}
													</span>
													<span>
														<strong>{pack.label}</strong>
														<small>
															{ownedPackIds === undefined ||
															pack.access === "included"
																? t("appearance.included")
																: owned
																	? t("appearance.owned")
																	: t("appearance.premium")}
														</small>
													</span>
												</button>
											);
										})}
									</fieldset>
									{activePack && (
										<>
											<div className="fl-bot-color-pack-description">
												<p>{activePack.description}</p>
												{activePack.access === "premium" && onRequestPack && (
													<button
														type="button"
														className="fl-bot-studio-pack-link"
														aria-label={t("appearance.viewNamedColorPack", {
															label: activePack.label,
														})}
														onClick={() => onRequestPack(activePack)}
													>
														{t("appearance.viewPack")}{" "}
														<span aria-hidden="true">↗</span>
													</button>
												)}
											</div>
											{!packOwned && (
												<p className="fl-bot-color-lock-note">
													{hasAvailableColors
														? t(
																"appearance.previewEveryColorUnlockThePackToSaveColorsMarkedPreviewOnly",
															)
														: activePack.allowsCustomHex
															? t(
																	"appearance.tryAnyCustomColorUnlockThisPackToSaveIt",
																)
															: t(
																	"appearance.previewEveryColorInThisPackUnlockItToSaveYourChoices",
																)}
												</p>
											)}
											{activePack.colorIds.length > 0 && (
												<div className="fl-bot-color-swatches">
													{activePack.colorIds.flatMap((colorId) => {
														const swatch = Object.hasOwn(
															catalog.colors,
															colorId,
														)
															? catalog.colors[colorId]
															: undefined;
														if (!swatch) return [];
														const choice: BotColorSelection = {
															kind: "swatch",
															colorId,
														};
														const canSave = canUseBotColor(
															choice,
															catalog,
															ownedPackIds,
														);
														return [
															<button
																key={colorId}
																type="button"
																aria-label={t("appearance.chooseSlotColor", {
																	label: swatch.label,
																	slot: ROLE_LABELS[slot],
																})}
																aria-pressed={
																	selected?.kind === "swatch" &&
																	selected.colorId === colorId
																}
																disabled={disabled}
																data-preview-only={!canSave || undefined}
																onClick={() => select(slot, choice)}
															>
																<span className="fl-bot-color-swatch-chip">
																	<BotColorSwatch paint={swatch.paint} />
																</span>
																<span>{swatch.label}</span>
																{!canSave && (
																	<small>{t("appearance.previewOnly")}</small>
																)}
															</button>,
														];
													})}
												</div>
											)}
											{activePack.allowsCustomHex && !disabled && (
												<CustomHex
													key={`${slot}-${activePack.id}`}
													slot={slot}
													pack={activePack}
													initialColor={
														normalizeBotHex(getBotColorBase(shownPaint)) ??
														"#808080"
													}
													onChange={(selection) => select(slot, selection)}
												/>
											)}
										</>
									)}
								</section>
							)}
						</div>
					);
				})}
			</div>
		</div>
	);
}
