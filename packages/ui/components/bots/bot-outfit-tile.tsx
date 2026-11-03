"use client";

import { useEffect, useId, useMemo, useRef, useState } from "react";
import { isBotPaletteEffect } from "./appearance";
import { BotAvatar } from "./bot-avatar";
import {
	canUseBotColor,
	getBotColorBase,
	getBotCustomizableColorSlots,
} from "./colors";
import type {
	BotAppearance,
	BotCatalog,
	BotEquipmentDefinition,
	BotExpression,
} from "./types";
import "./bot-outfit-tile.css";

export interface BotOutfitTileProps {
	item: BotEquipmentDefinition;
	appearance: BotAppearance;
	catalog: BotCatalog;
	expression: BotExpression;
	selected: boolean;
	locked: boolean;
	onSelect: () => void;
}

const COLOR_IDEAS = [
	["color-coral", "color-honey"],
	["color-leaf", "color-cream"],
	["color-sky", "color-lavender"],
] as const;

export function BotOutfitTile({
	item,
	appearance,
	catalog,
	expression,
	selected,
	locked,
	onSelect,
}: BotOutfitTileProps) {
	const [hovered, setHovered] = useState(false);
	const [keyboardFocused, setKeyboardFocused] = useState(false);
	const [colorIndex, setColorIndex] = useState(0);
	const pointerFocused = useRef(false);
	const hintId = useId();
	const variations = useMemo(() => {
		const shape = Object.hasOwn(catalog.shapes, appearance.kind)
			? catalog.shapes[appearance.kind]
			: undefined;
		if (!shape || isBotPaletteEffect(appearance.effect ?? "none")) return [];
		const slots = getBotCustomizableColorSlots(shape);
		return COLOR_IDEAS.flatMap(([clothing, accent]) => {
			const colors: NonNullable<BotAppearance["colors"]> = {};
			let swatch: string | undefined;
			for (const [slot, colorId] of [
				["clothing", clothing],
				["accent", accent],
			] as const) {
				const selection = { kind: "swatch", colorId } as const;
				if (!slots.includes(slot) || !canUseBotColor(selection, catalog, []))
					continue;
				colors[slot] = selection;
				swatch ??= getBotColorBase(catalog.colors[colorId].paint);
			}
			return swatch ? [{ id: `${clothing}-${accent}`, colors, swatch }] : [];
		});
	}, [catalog, appearance.kind, appearance.effect]);
	const active = (hovered || keyboardFocused) && variations.length > 0;
	const variation = active
		? variations[colorIndex % variations.length]
		: undefined;

	useEffect(() => {
		if (!active) return;
		const media = window.matchMedia?.("(prefers-reduced-motion: reduce)");
		let timer: ReturnType<typeof setInterval> | undefined;
		const update = () => {
			clearInterval(timer);
			if (!media?.matches && !document.hidden && variations.length > 1) {
				timer = setInterval(
					() => setColorIndex((index) => (index + 1) % variations.length),
					1400,
				);
			}
		};
		update();
		media?.addEventListener("change", update);
		document.addEventListener("visibilitychange", update);
		return () => {
			clearInterval(timer);
			media?.removeEventListener("change", update);
			document.removeEventListener("visibilitychange", update);
		};
	}, [active, variations.length]);

	return (
		<button
			type="button"
			className="fl-bot-studio-tile fl-bot-outfit-tile"
			aria-label={`Choose ${item.label}`}
			aria-pressed={selected}
			aria-describedby={variations.length ? hintId : undefined}
			data-locked={locked || undefined}
			data-color-preview={
				variation ? colorIndex % variations.length : undefined
			}
			onClick={() => onSelect()}
			onPointerEnter={(event) => {
				if (event.pointerType !== "mouse") return;
				setColorIndex(0);
				setHovered(true);
			}}
			onPointerLeave={() => setHovered(false)}
			onPointerCancel={() => setHovered(false)}
			onPointerDown={(event) => {
				pointerFocused.current = true;
				setKeyboardFocused(false);
				if (event.pointerType !== "mouse") setHovered(false);
			}}
			onFocus={() => {
				if (pointerFocused.current) return;
				setColorIndex(0);
				setKeyboardFocused(true);
			}}
			onBlur={() => {
				pointerFocused.current = false;
				setKeyboardFocused(false);
			}}
			onKeyDown={(event) => {
				if (event.key === "Enter" || event.key === " ") {
					pointerFocused.current = false;
					setKeyboardFocused(true);
				}
			}}
		>
			<span className="fl-bot-studio-tile-art" aria-hidden="true">
				<BotAvatar
					appearance={{
						...appearance,
						equipment: { [item.slot]: item.id },
						...(variation
							? { colors: { ...appearance.colors, ...variation.colors } }
							: {}),
					}}
					catalog={catalog}
					size={68}
					expression={expression}
					pose="center"
					animated={false}
					decorative
				/>
			</span>
			<span className="fl-bot-outfit-swatches" aria-hidden="true">
				{variations.map((idea, index) => (
					<span
						key={idea.id}
						style={{ background: idea.swatch }}
						data-active={
							(active && colorIndex % variations.length === index) || undefined
						}
					/>
				))}
			</span>
			<span className="fl-bot-studio-tile-name">{item.label}</span>
			{variations.length > 0 && (
				<span className="fl-bot-outfit-color-hint" id={hintId}>
					Hover or focus for color ideas. Choose your colors in the Colors tab.
				</span>
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
