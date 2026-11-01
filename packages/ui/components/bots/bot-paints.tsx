import type { CSSProperties } from "react";
import { isBotPaletteEffect } from "./appearance";
import { getBotColorSelectionPaint } from "./colors";
import type {
	BotAppearance,
	BotCatalog,
	BotColorSlot,
	BotPaint,
	BotPalette,
	BotRenderContext,
} from "./types";
import { BOT_COLOR_SLOTS } from "./types";

type ComplexPaint = Exclude<BotPaint, { kind: "solid" }>;
export type BotPaintDefinitions = Partial<Record<BotColorSlot, ComplexPaint>>;

export function getBotPaintId(idPrefix: string, slot: BotColorSlot): string {
	return `${idPrefix}-paint-${slot}`;
}

/** SVG paint servers cannot be used as CSS colors, gradient stops, or HTML backgrounds. */
export function resolveBotPaintPalette(
	appearance: Required<BotAppearance>,
	catalog: BotCatalog,
	solidPalette: BotPalette,
	idPrefix: string,
): {
	palette: BotPalette;
	paints: BotPaintDefinitions;
	paintedSlots: BotColorSlot[];
} {
	const palette = { ...solidPalette };
	const paints: BotPaintDefinitions = {};
	const paintedSlots: BotColorSlot[] = [];
	if (isBotPaletteEffect(appearance.effect)) {
		return { palette, paints, paintedSlots };
	}
	for (const slot of BOT_COLOR_SLOTS) {
		if (
			slot === "background" &&
			(appearance.dark || appearance.effect === "neon")
		)
			continue;
		const selection = appearance.colors[slot];
		if (!selection) continue;
		const paint = getBotColorSelectionPaint(selection, catalog);
		if (!paint || paint.kind === "solid") continue;
		paints[slot] = paint;
		palette[slot] = `url(#${getBotPaintId(idPrefix, slot)})`;
		paintedSlots.push(slot);
	}
	return { palette, paints, paintedSlots };
}

function gradientCoordinates(angle: number) {
	const radians =
		(((Number.isFinite(angle) ? angle : 45) % 360) * Math.PI) / 180;
	const x = Math.cos(radians);
	const y = Math.sin(radians);
	const length = 50 / Math.max(Math.abs(x), Math.abs(y));
	return {
		x1: 50 - x * length,
		y1: 50 - y * length,
		x2: 50 + x * length,
		y2: 50 + y * length,
	};
}

function PaintMotif({
	paint,
	cell,
}: { paint: Extract<BotPaint, { kind: "pattern" }>; cell: number }) {
	const center = cell / 2;
	const radius = cell * 0.24;
	switch (paint.motif) {
		case "stripes":
			return (
				<path
					d={`M${-cell / 2} ${cell}L${cell / 2} 0M${cell / 2} ${cell}L${cell * 1.5} 0`}
					fill="none"
					stroke={paint.ink}
					strokeWidth={cell * 0.27}
				/>
			);
		case "stars":
			return (
				<path
					d={`M${center} ${center - radius}l${radius * 0.28} ${radius * 0.65} ${radius * 0.72} ${radius * 0.35}-${radius * 0.72} ${radius * 0.3}-${radius * 0.28} ${radius * 0.7}-${radius * 0.28}-${radius * 0.7}-${radius * 0.72}-${radius * 0.3} ${radius * 0.72}-${radius * 0.35}Z`}
					fill={paint.ink}
				/>
			);
		case "diamonds":
			return (
				<path
					d={`M${center} ${center - radius}l${radius} ${radius}-${radius} ${radius}-${radius}-${radius}Z`}
					fill={paint.ink}
				/>
			);
		case "hearts":
			return (
				<path
					d="M0 1C-1.8 0-3-1.2-3-2.4C-3-4.4-.8-4.7 0-3.1C.8-4.7 3-4.4 3-2.4C3-1.2 1.8 0 0 1Z"
					transform={`translate(${center} ${center + cell * 0.12}) scale(${cell * 0.09})`}
					fill={paint.ink}
				/>
			);
		default:
			return (
				<circle cx={center} cy={center} r={cell * 0.17} fill={paint.ink} />
			);
	}
}

/** Uses rig coordinates so paints also work on horizontal eyelid and mouth strokes. */
export function BotPaintDefs({
	paints,
	idPrefix,
}: { paints: BotPaintDefinitions; idPrefix: string }) {
	return (
		<>
			{BOT_COLOR_SLOTS.map((slot) => {
				const paint = paints[slot];
				if (!paint) return null;
				const id = getBotPaintId(idPrefix, slot);
				if (paint.kind === "gradient") {
					return (
						<linearGradient
							key={slot}
							id={id}
							data-paint-slot={slot}
							data-paint-kind="gradient"
							data-paint-animated={paint.animated || undefined}
							gradientUnits="userSpaceOnUse"
							{...gradientCoordinates(paint.angle ?? 45)}
						>
							{paint.colors.map((color, index) => (
								<stop
									key={`${index}-${color}`}
									offset={
										paint.colors.length > 1
											? index / (paint.colors.length - 1)
											: 0
									}
									stopColor={color}
									className={paint.animated ? "fl-bot-paint-stop" : undefined}
									style={
										paint.animated
											? ({
													"--bot-paint-stop-0": color,
													"--bot-paint-stop-1":
														paint.colors[(index + 1) % paint.colors.length],
													"--bot-paint-stop-2":
														paint.colors[(index + 2) % paint.colors.length],
													"--bot-paint-stop-3":
														paint.colors[(index + 3) % paint.colors.length],
												} as CSSProperties)
											: undefined
									}
								/>
							))}
						</linearGradient>
					);
				}
				const cell =
					typeof paint.scale === "number" &&
					Number.isFinite(paint.scale) &&
					paint.scale > 0
						? paint.scale
						: 10;
				return (
					<pattern
						key={slot}
						id={id}
						data-paint-slot={slot}
						data-paint-kind="pattern"
						data-paint-motif={paint.motif}
						patternUnits="userSpaceOnUse"
						width={cell}
						height={cell}
					>
						<rect width={cell} height={cell} fill={paint.base} />
						<PaintMotif paint={paint} cell={cell} />
					</pattern>
				);
			})}
		</>
	);
}

/** Blends palette roles through opacity, allowing either role to contain a complex paint. */
export function BotPaintBlend({
	context,
	id,
	d,
	from,
	to,
	angle = 90,
	end = 1,
	opacity = 1,
}: {
	context: BotRenderContext;
	id: string;
	d: string;
	from: BotColorSlot;
	to: BotColorSlot;
	angle?: number;
	end?: number;
	opacity?: number;
}) {
	const maskId = `${context.idPrefix}-${id}-blend-mask`;
	const fadeId = `${context.idPrefix}-${id}-blend-fade`;
	const coordinates = gradientCoordinates(angle);
	return (
		<g data-paint-blend={`${from}-${to}`}>
			<defs>
				<linearGradient
					id={fadeId}
					x1={`${coordinates.x1}%`}
					y1={`${coordinates.y1}%`}
					x2={`${coordinates.x2}%`}
					y2={`${coordinates.y2}%`}
				>
					<stop stopColor="#ffffff" stopOpacity={opacity} />
					<stop offset={end} stopColor="#ffffff" stopOpacity="0" />
				</linearGradient>
				<mask
					id={maskId}
					maskUnits="userSpaceOnUse"
					x="0"
					y="0"
					width="100"
					height="100"
				>
					<path d={d} fill={`url(#${fadeId})`} />
				</mask>
			</defs>
			<path data-blend-base={to} d={d} fill={context.palette[to]} />
			<path
				data-blend-overlay={from}
				d={d}
				fill={context.palette[from]}
				mask={`url(#${maskId})`}
			/>
		</g>
	);
}
