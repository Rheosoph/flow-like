"use client";

import { type CSSProperties, useId, useRef } from "react";
import { BOT_TIMING } from "./animation";
import {
	getBotBadges,
	getBotPalette,
	isBotPaletteEffect,
	resolveBotAppearance,
} from "./appearance";
import {
	BotEffectDefs,
	BotEffectOverlay,
	getBotEffectFilter,
} from "./bot-effects";
import { BotEmote } from "./bot-emote";
import { BotEyes, BotFace } from "./bot-face";
import {
	BotPaintBlend,
	BotPaintDefs,
	resolveBotPaintPalette,
} from "./bot-paints";
import { BotPixel } from "./bot-pixel";
import { defaultBotCatalog } from "./catalog";
import { getBotStatePreset } from "./states";
import type {
	BotAvatarProps,
	BotBackdrop,
	BotRenderContext,
	BotVariant,
} from "./types";
import { useBotAnimation } from "./use-bot-animation";
import "./bot-avatar.css";

const timingStyles = Object.fromEntries(
	Object.entries(BOT_TIMING)
		.filter(([key, value]) => key.endsWith("Ms") && typeof value === "number")
		.map(([key, value]) => [`--bot-${key}`, `${value}ms`]),
) as CSSProperties;

function VariantFilter({ id, variant }: { id: string; variant: BotVariant }) {
	return (
		<filter id={id} colorInterpolationFilters="sRGB">
			{variant === "gold" ? (
				<>
					<feColorMatrix type="saturate" values="0" />
					<feComponentTransfer>
						<feFuncR type="table" tableValues=".28 .88 1" />
						<feFuncG type="table" tableValues=".16 .62 .89" />
						<feFuncB type="table" tableValues=".06 .15 .56" />
					</feComponentTransfer>
				</>
			) : variant === "midnight" ? (
				<feColorMatrix values=".35 0 0 0 .07 0 .35 0 0 .06 0 0 .55 0 .17 0 0 0 1 0" />
			) : variant === "pastel" ? (
				<feColorMatrix values=".65 0 0 0 .35 0 .65 0 0 .35 0 0 .65 0 .35 0 0 0 1 0" />
			) : (
				<feColorMatrix
					type="saturate"
					values={variant === "shiny" ? "1.7" : "1"}
				/>
			)}
		</filter>
	);
}

function Backdrop({
	type,
	context,
}: { type: BotBackdrop; context: BotRenderContext }) {
	const { palette, solidPalette, idPrefix } = context;
	return (
		<g
			color={solidPalette?.body ?? palette.body}
			opacity=".2"
			data-backdrop={type}
		>
			{type === "gradient" && (
				<BotPaintBlend
					context={context}
					id="backdrop"
					d="M0 0H100V100H0Z"
					from="accent"
					to="body"
					angle={45}
				/>
			)}
			{(type === "dots" || type === "stripes") && (
				<>
					<defs>
						<pattern
							id={`${idPrefix}-pattern`}
							width="12"
							height="12"
							patternUnits="userSpaceOnUse"
							patternTransform={type === "stripes" ? "rotate(35)" : undefined}
						>
							{type === "dots" ? (
								<circle cx="3" cy="3" r="1.6" fill={palette.body} />
							) : (
								<path d="M0 0V12" stroke={palette.body} strokeWidth="4" />
							)}
						</pattern>
					</defs>
					<path d="M0 0H100V100H0Z" fill={`url(#${idPrefix}-pattern)`} />
				</>
			)}
			{type === "waves" && (
				<g
					className="fl-bot-waves"
					fill="none"
					stroke={palette.body}
					strokeWidth="1.7"
				>
					{[12, 27, 42, 57, 72, 87, 102].map((y) => (
						<path
							key={y}
							d={`M-40 ${y}q10 -7 20 0t20 0t20 0t20 0t20 0t20 0t20 0t20 0`}
						/>
					))}
				</g>
			)}
			{type === "rays" && (
				<g className="fl-bot-rays" fill={palette.body}>
					{[0, 30, 60, 90, 120, 150, 180, 210, 240, 270, 300, 330].map(
						(angle) => (
							<path
								key={angle}
								d="M50 50L43 -25H57Z"
								transform={`rotate(${angle} 50 50)`}
							/>
						),
					)}
				</g>
			)}
			{(type === "stars" || type === "confetti") && (
				<g fill={palette.body}>
					{[
						[19, 22],
						[43, 16],
						[76, 26],
						[87, 51],
						[15, 67],
						[72, 81],
						[37, 88],
						[61, 46],
					].map(([x, y], i) => (
						<g
							key={`${x}-${y}`}
							transform={`translate(${x} ${y}) rotate(${i * 25})`}
						>
							<path
								className={
									type === "stars"
										? `fl-bot-star fl-bot-star-${i % 2}`
										: undefined
								}
								d={
									type === "stars"
										? "M0 -4L1 -1L4 0L1 1L0 4L-1 1L-4 0L-1 -1Z"
										: "M-1 -3H1V3H-1Z"
								}
							/>
						</g>
					))}
				</g>
			)}
		</g>
	);
}

/** Shared SVG rig. All catalog art uses the same coordinate system and animated parents. */
export function BotAvatar({
	appearance = {},
	catalog = defaultBotCatalog,
	pose = "idle",
	expression,
	mood,
	agentState = "idle",
	gx,
	gy,
	size = 200,
	animated = true,
	ring,
	label,
	decorative = false,
	className = "",
	style,
}: BotAvatarProps) {
	const instanceId = useId();
	const idPrefix = `bot-${instanceId.replace(/[^a-zA-Z0-9_-]/g, "")}`;
	const elementRef = useRef<HTMLDivElement>(null);
	const svgRef = useRef<SVGSVGElement>(null);
	const preset = getBotStatePreset(mood, agentState);
	const resolved = resolveBotAppearance(
		preset
			? {
					...appearance,
					emote: preset.emote,
					effect: preset.effect ?? appearance.effect,
				}
			: appearance,
		catalog,
	);
	const resolvedPose = preset?.pose ?? pose;
	const shape = catalog.shapes[resolved.kind];
	const pixels = Number.isFinite(size) ? Math.max(16, size) : 200;
	const motion = useBotAnimation({
		pose: resolvedPose,
		gx,
		gy,
		animated,
		elementRef,
	});
	const resolvedExpression =
		preset?.expression ??
		expression ??
		(resolvedPose === "happy"
			? "happy"
			: resolvedPose === "oops"
				? "oops"
				: "neutral");
	const solidPalette = getBotPalette(shape.palette, resolved, catalog);
	const { palette, paints, paintedSlots } = resolveBotPaintPalette(
		resolved,
		catalog,
		solidPalette,
		idPrefix,
	);
	const context: BotRenderContext = {
		anchors: shape.anchors,
		palette,
		solidPalette,
		paintedSlots,
		pose: resolvedPose,
		expression: resolvedExpression,
		effect: resolved.effect,
		idPrefix,
		equipment: resolved.equipment,
	};
	const faceContext =
		resolved.effect === "thermal" && shape.eyeStyle === "light"
			? { ...context, palette: { ...palette, sclera: palette.ink } }
			: context;
	const { visible, overflow } = getBotBadges(resolved, catalog, pixels);
	const paletteEffect = isBotPaletteEffect(resolved.effect);
	const variant = paletteEffect ? "original" : resolved.variant;
	const filter =
		variant !== "original" && variant !== "holo"
			? `url(#${idPrefix}-variant)`
			: undefined;
	const radius =
		resolved.frame === "circle" ? 50 : resolved.frame === "squircle" ? 28 : 0;
	const eye = shape.anchors.eyes;
	const roundEyes =
		!shape.eyeStyle ||
		shape.eyeStyle === "round" ||
		shape.eyeStyle === "sleepy";
	const gazeScale = roundEyes ? Math.min(1, eye.radius / 8) : 1;
	const gaze = { x: motion.gaze.x * gazeScale, y: motion.gaze.y * gazeScale };
	const closed = resolvedPose === "sleep" || resolvedPose === "blink";
	const emote =
		resolvedPose === "sleep" && resolved.emote === "none"
			? "zzz"
			: resolved.emote;
	const eyeCenter = (eye.left[1] + eye.right[1]) / 2;
	const equipment = (slot: "neck" | "eyes" | "hand" | "head") => {
		const id = resolved.equipment[slot];
		return id ? catalog.equipment[id]?.render(context) : null;
	};
	const badgeLabels = [...visible, ...overflow].map((skill) => skill.label);
	const accessibleLabel =
		label ??
		`${shape.label} bot${preset ? `; ${preset.label}` : ""}${badgeLabels.length ? `; skills: ${badgeLabels.join(", ")}` : ""}`;
	const badgeCount = visible.length + (overflow.length ? 1 : 0);
	const vars = {
		...timingStyles,
		"--bot-size": `${pixels}px`,
		"--bot-radius": `${radius}%`,
		"--bot-badge-ring": ring ?? (resolved.dark ? "#181824" : "#ffffff"),
		"--bot-gaze-x": `${gaze.x}px`,
		"--bot-gaze-y": `${gaze.y}px`,
		"--bot-face-x": `${gaze.x * BOT_TIMING.faceFollow}px`,
		"--bot-face-y": `${gaze.y * BOT_TIMING.faceFollow}px`,
		"--bot-eye-center": `${eyeCenter}px`,
		"--bot-body-center": `${shape.bodyOrigin?.[0] ?? shape.anchors.head.x}px ${shape.bodyOrigin?.[1] ?? (shape.anchors.head.y + shape.anchors.neck.bottom) / 2}px`,
		...style,
	} as CSSProperties;
	return (
		<div
			ref={elementRef}
			className={`fl-bot ${className}`}
			style={vars}
			role={decorative ? undefined : "img"}
			aria-label={decorative ? undefined : accessibleLabel}
			aria-hidden={decorative || undefined}
			data-motion={motion.motionEnabled ? "on" : "off"}
			data-effect={resolved.effect}
			data-frame={resolved.frame}
			data-kind={shape.id}
			data-expression={resolvedExpression}
			data-pose={resolvedPose}
			data-mood={mood}
			data-agent-state={agentState}
			data-paint-slots={
				paintedSlots.length ? paintedSlots.join(" ") : undefined
			}
		>
			<svg
				ref={svgRef}
				viewBox="0 0 100 100"
				width={pixels}
				height={pixels}
				aria-hidden="true"
				focusable="false"
				className="fl-bot-svg"
			>
				<defs>
					<BotPaintDefs paints={paints} idPrefix={idPrefix} />
					<clipPath id={`${idPrefix}-frame`}>
						<rect width="100" height="100" rx={radius} />
					</clipPath>
					<clipPath id={`${idPrefix}-body`}>
						<path
							d={shape.silhouette}
							className="fl-bot-body"
							data-body-motion={shape.bodyMotion}
						/>
					</clipPath>
					{shape.clothingClip && (
						<clipPath id={`${idPrefix}-clothing`}>
							<path
								d={shape.clothingClip}
								clipRule="evenodd"
								className="fl-bot-body"
								data-body-motion={shape.bodyMotion}
							/>
						</clipPath>
					)}
					<VariantFilter id={`${idPrefix}-variant`} variant={variant} />
					<BotEffectDefs
						effect={resolved.effect}
						idPrefix={idPrefix}
						palette={solidPalette}
					/>
				</defs>
				<g clipPath={`url(#${idPrefix}-frame)`}>
					<g data-part="bg">
						<rect width="100" height="100" fill={palette.background} />
						<Backdrop type={resolved.backdrop} context={context} />
					</g>
					<g
						className="fl-bot-effect-rig"
						filter={getBotEffectFilter(resolved.effect, idPrefix)}
					>
						<g
							data-part="hop"
							className="fl-bot-hop"
							data-hopping={motion.hopping}
							data-sleeping={resolvedPose === "sleep"}
						>
							<g
								data-part="character"
								className="fl-bot-character"
								data-character-motion={
									shape.bodyMotion === "float" ? "float" : undefined
								}
							>
								<g
									data-part="body"
									className="fl-bot-body"
									data-body-motion={shape.bodyMotion}
								>
									<g
										className="fl-bot-skin"
										data-variant={variant}
										filter={filter}
									>
										{shape.renderBackDetails?.(context)}
										<path
											d={shape.silhouette}
											fill={palette.body}
											data-pixel-outline="true"
										/>
										{shape.renderDetails?.(context)}
									</g>
								</g>
								{variant === "gold" && (
									<g clipPath={`url(#${idPrefix}-body)`} fill={palette.sclera}>
										{[
											[30, 52],
											[72, 46],
											[66, 84],
										].map(([x, y]) => (
											<path
												key={`${x}-${y}`}
												className="fl-bot-gold-sparkle"
												d={`M${x} ${y - 3}l1 2 2 1-2 1-1 2-1-2-2-1 2-1Z`}
											/>
										))}
									</g>
								)}
								<g
									data-part="neck"
									clipPath={`url(#${idPrefix}-${shape.clothingClip ? "clothing" : "body"})`}
								>
									{equipment("neck")}
								</g>
								<g
									data-part="eyes"
									className="fl-bot-eyes"
									data-blinking={!closed && motion.blinking}
								>
									<BotEyes shape={shape} context={faceContext} />
								</g>
								<g data-part="face" className="fl-bot-face">
									<BotFace shape={shape} context={faceContext} />
									<g data-slot="eyes">{equipment("eyes")}</g>
								</g>
								<g data-part="hand">{pixels >= 56 && equipment("hand")}</g>
								<g data-part="head">{equipment("head")}</g>
							</g>
						</g>
						<g
							data-part="emote"
							className="fl-bot-emote"
							data-visible={emote !== "none" && motion.emoteVisible}
						>
							{emote !== "none" && <BotEmote emote={emote} context={context} />}
						</g>
					</g>
					<BotEffectOverlay
						effect={resolved.effect}
						idPrefix={idPrefix}
						palette={solidPalette}
					/>
				</g>
			</svg>
			{resolved.effect === "pixel" && (
				<BotPixel
					sourceRef={svgRef}
					animated={motion.motionEnabled}
					frame={resolved.frame}
					ink={solidPalette.ink}
				/>
			)}
			{resolved.halo !== "none" && (
				<div
					className="fl-bot-halo"
					data-halo={resolved.halo}
					aria-hidden="true"
				>
					<div className="fl-bot-halo-spin" />
				</div>
			)}
			{badgeCount > 0 && (
				<div className="fl-bot-badges" aria-hidden="true">
					{[
						...visible.map((skill) => ({
							key: skill.id,
							title: skill.label,
							glyph: skill.glyph,
							color: skill.color,
						})),
						...(overflow.length
							? [
									{
										key: "overflow",
										title: overflow.map((skill) => skill.label).join(", "),
										glyph: `+${overflow.length}`,
										color: solidPalette.ink,
									},
								]
							: []),
					].map((badge, index) => {
						const angle =
							((badgeCount === 1 ? 25 : -25 + index * (65 / (badgeCount - 1))) *
								Math.PI) /
							180;
						return (
							<span
								key={badge.key}
								className="fl-bot-badge"
								title={badge.title}
								data-skill={badge.key}
								style={{
									left: `${50 + Math.cos(angle) * 49}%`,
									top: `${50 + Math.sin(angle) * 49}%`,
									background: paletteEffect
										? solidPalette.clothing
										: badge.color,
								}}
							>
								{badge.glyph}
							</span>
						);
					})}
				</div>
			)}
		</div>
	);
}
