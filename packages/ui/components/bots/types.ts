import type { CSSProperties, ReactNode } from "react";

export const BOT_EQUIPMENT_SLOTS = ["neck", "eyes", "hand", "head"] as const;
export type BotEquipmentSlot = (typeof BOT_EQUIPMENT_SLOTS)[number];
export type BotFrame = "circle" | "squircle" | "square";
export const BOT_EXPRESSIONS = [
	"neutral",
	"happy",
	"concerned",
	"surprised",
	"curious",
	"sad",
	"wink",
	"love",
	"angry",
	"focused",
	"oops",
] as const;
export type BotExpression = (typeof BOT_EXPRESSIONS)[number];
export type BotMood =
	| "likes-you"
	| "cheerful"
	| "love"
	| "wink"
	| "surprised"
	| "thinking"
	| "sad"
	| "angry"
	| "oops"
	| "sleeping";
export type BotAgentState =
	| "idle"
	| "working"
	| "idea"
	| "done"
	| "error"
	| "needs-you"
	| "offline";
export type BotPose =
	| "idle"
	| "track"
	| "center"
	| "left"
	| "right"
	| "up"
	| "down"
	| "blink"
	| "happy"
	| "sleep"
	| "oops";
export type BotVariant =
	| "original"
	| "shiny"
	| "midnight"
	| "gold"
	| "pastel"
	| "holo";
export type BotBackdrop =
	| "none"
	| "gradient"
	| "dots"
	| "stripes"
	| "waves"
	| "rays"
	| "stars"
	| "confetti";
export type BotHalo = "none" | "flow" | "gold" | "aurora" | "pulse" | "orbit";
export type BotEffect =
	| "none"
	| "pixel"
	| "lcd"
	| "crt"
	| "thermal"
	| "riso"
	| "glitch"
	| "raster"
	| "comic"
	| "watercolor"
	| "blueprint"
	| "sticker"
	| "vinyl"
	| "neon"
	| "offline";
export type BotEmote =
	| "none"
	| "heart"
	| "note"
	| "spark"
	| "alert"
	| "question"
	| "drop"
	| "zzz"
	| "rain"
	| "anger"
	| "loading"
	| "idea"
	| "check"
	| "error";

/** JSON-safe appearance. Persist catalog IDs, never component functions or SVG markup. */
export interface BotAppearance {
	kind: string;
	frame?: BotFrame;
	variant?: BotVariant;
	backdrop?: BotBackdrop;
	halo?: BotHalo;
	effect?: BotEffect;
	emote?: BotEmote;
	dark?: boolean;
	/** The owner muted this bot's notifications. */
	mute_notifications?: boolean;
	/** One catalog item per slot. Missing or unavailable items are omitted. */
	equipment?: Partial<Record<BotEquipmentSlot, string>>;
	skills?: readonly string[];
	colors?: Partial<Record<BotColorSlot, BotColorSelection>>;
}

export interface BotPalette {
	body: string;
	background: string;
	ink: string;
	cheek: string;
	clothing: string;
	accent: string;
	sclera: string;
}

export type BotColorSlot = keyof BotPalette;
export const BOT_COLOR_SLOTS = Object.freeze([
	"body",
	"accent",
	"background",
	"ink",
	"sclera",
	"cheek",
	"clothing",
] as const satisfies readonly BotColorSlot[]);

export type BotColorSelection =
	| { kind: "swatch"; colorId: string }
	| { kind: "hex"; value: string; packId: string };

export type BotPaint =
	| { kind: "solid"; color: string }
	| {
			kind: "gradient";
			colors: readonly string[];
			angle?: number;
			animated?: boolean;
	  }
	| {
			kind: "pattern";
			base: string;
			ink: string;
			motif: "dots" | "stripes" | "stars" | "diamonds" | "hearts";
			/** Pattern cell width in the shared 100-unit SVG rig. */
			scale?: number;
	  };

export interface BotColorDefinition {
	id: string;
	label: string;
	paint: BotPaint;
}

export interface BotColorPackDefinition {
	id: string;
	label: string;
	description: string;
	colorIds: readonly string[];
	access: "included" | "premium";
	allowsCustomHex?: boolean;
}

/** Coordinates are in the shared 100 × 100 view box. */
export interface BotAnchors {
	eyes: {
		left: readonly [number, number];
		right: readonly [number, number];
		radius: number;
	};
	mouth: readonly [number, number];
	head: { x: number; y: number; width: number };
	/** The neckline and bottom of clothing may be overridden for each silhouette. */
	neck: {
		y: number;
		bottom: number;
		/** Garment center and width may differ from the head on narrow or asymmetric bodies. */
		x?: number;
		width?: number;
	};
	hand: readonly [number, number];
}

export interface BotRenderContext {
	anchors: BotAnchors;
	palette: BotPalette;
	/** Solid representatives for gradients, filters and native shape details. */
	solidPalette?: BotPalette;
	/** Roles currently using a gradient or pattern paint. */
	paintedSlots?: readonly BotColorSlot[];
	pose: BotPose;
	expression: BotExpression;
	effect?: BotEffect;
	/** Instance-scoped prefix for any SVG definitions an extension needs. */
	idPrefix: string;
	equipment?: Readonly<Partial<Record<BotEquipmentSlot, string>>>;
}

export interface BotSeriesDefinition {
	id: string;
	label: string;
	description: string;
}

export interface BotShapeDefinition {
	id: string;
	label: string;
	series?: string;
	description?: string;
	palette: BotPalette;
	/** Omit to allow all palette roles; an empty array disables color editing. */
	colorSlots?: readonly BotColorSlot[];
	anchors: BotAnchors;
	/** Used for the painted body and the default clothing clip. */
	silhouette: string;
	/** Garment boundary, including holes or a torso that excludes surrounding appendages. */
	clothingClip?: string;
	bodyMotion?: "rotate" | "float";
	bodyOrigin?: readonly [number, number];
	/** Painted surface behind the eyes and mouth for color contrast checks. */
	faceColor?: "body" | "accent" | "clothing" | "sclera";
	/** Colors used by exposed details, such as leaves, that must stand out from the background. */
	backgroundDetailColors?: readonly BotColorSlot[];
	eyeStyle?: "round" | "dots" | "light" | "sleepy";
	/** Closed lids may need a lighter stroke over dark eye patches. */
	closedEyeColor?: keyof BotPalette;
	mouthStyle?: "smile" | "none" | "line" | "beak" | "cat" | "o";
	mouthScale?: number;
	/** Choose a readable mouth color when the face sits on a visor or a mask. */
	mouthColor?: keyof BotPalette;
	cheeks?: boolean;
	/** Parts behind the body, such as the far side of a planetary ring. */
	renderBackDetails?: (context: BotRenderContext) => ReactNode;
	/** Optional details behind the face, such as antennae, ears or a leaf. */
	renderDetails?: (context: BotRenderContext) => ReactNode;
	/** Face markings follow the gaze but stay upright when the body rotates. */
	renderFaceDetails?: (context: BotRenderContext) => ReactNode;
}

export interface BotEquipmentDefinition {
	id: string;
	label: string;
	slot: BotEquipmentSlot;
	/** A hand item becomes this skill below 56 px. Omit to hide it instead. */
	compactSkill?: string;
	/** Omit to support every shape. */
	kinds?: readonly string[];
	/** Shapes whose anatomy conflicts with this item. These exclusions take precedence over kinds. */
	excludedKinds?: readonly string[];
	render: (context: BotRenderContext) => ReactNode;
}

export interface BotSkillDefinition {
	id: string;
	label: string;
	color: string;
	/** A compact glyph displayed in a badge, normally one or two characters. */
	glyph: string;
}

/** A content bundle. Pricing, purchases and ownership are supplied by the host. */
export interface BotPackDefinition {
	id: string;
	label: string;
	description: string;
	coverKind: string;
	shapeIds: readonly string[];
	equipmentIds?: readonly string[];
}

export interface BotCatalog {
	readonly defaultKind: string;
	readonly series: Readonly<Record<string, BotSeriesDefinition>>;
	readonly packs: Readonly<Record<string, BotPackDefinition>>;
	readonly shapes: Readonly<Record<string, BotShapeDefinition>>;
	readonly equipment: Readonly<Record<string, BotEquipmentDefinition>>;
	readonly skills: Readonly<Record<string, BotSkillDefinition>>;
	readonly colors: Readonly<Record<string, BotColorDefinition>>;
	readonly colorPacks: Readonly<Record<string, BotColorPackDefinition>>;
}

export interface BotCatalogExtension {
	defaultKind?: string;
	series?: readonly BotSeriesDefinition[];
	packs?: readonly BotPackDefinition[];
	shapes?: readonly BotShapeDefinition[];
	equipment?: readonly BotEquipmentDefinition[];
	skills?: readonly BotSkillDefinition[];
	colors?: readonly BotColorDefinition[];
	colorPacks?: readonly BotColorPackDefinition[];
}

export interface BotAvatarProps {
	appearance?: Partial<BotAppearance>;
	catalog?: BotCatalog;
	pose?: BotPose;
	/** Facial expression changes independently of blinking, gaze and body motion. */
	expression?: BotExpression;
	/** Keep an explicit facial reaction while the agent state still controls pose and activity. */
	expressionOverridesState?: boolean;
	/** A temporary pose, expression and bubble; leaves saved appearance intact. */
	mood?: BotMood;
	/** An active agent state takes priority over mood and pose, and normally over expression. */
	agentState?: BotAgentState;
	/** Normalized gaze in [-1, 1]. Track follows the pointer when these are omitted. */
	gx?: number;
	gy?: number;
	size?: number;
	/** Pauses every animation. OS reduced-motion preferences always take precedence. */
	animated?: boolean;
	/** Badge border color, matched to the surrounding surface. */
	ring?: string;
	label?: string;
	decorative?: boolean;
	className?: string;
	style?: CSSProperties;
}
