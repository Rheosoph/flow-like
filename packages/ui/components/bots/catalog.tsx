import type { ReactNode } from "react";
import { defaultBotColorPacks, defaultBotColors } from "./color-catalog";
import { normalizeBotHex } from "./colors";
import { crewEquipment, crewPacks, crewSeries, crewShapes } from "./crew";
import type {
	BotCatalog,
	BotCatalogExtension,
	BotColorDefinition,
	BotColorPackDefinition,
	BotEquipmentDefinition,
	BotPackDefinition,
	BotRenderContext,
	BotShapeDefinition,
	BotSkillDefinition,
} from "./types";
import { BOT_COLOR_SLOTS, BOT_EQUIPMENT_SLOTS } from "./types";

function Headpiece({
	context,
	children,
}: {
	context: BotRenderContext;
	children: ReactNode;
}) {
	const { anchors, palette } = context;
	return (
		<g
			transform={`translate(${anchors.head.x} ${anchors.head.y}) scale(${anchors.head.width / 44})`}
			stroke={palette.ink}
			strokeWidth="1.6"
			strokeLinejoin="round"
			strokeLinecap="round"
		>
			{children}
		</g>
	);
}

function Handpiece({
	context,
	children,
}: {
	context: BotRenderContext;
	children: ReactNode;
}) {
	return (
		<g
			transform={`translate(${context.anchors.hand.join(" ")}) rotate(-9)`}
			stroke={context.palette.ink}
			strokeWidth="1.5"
			strokeLinejoin="round"
			strokeLinecap="round"
		>
			{children}
		</g>
	);
}

function Glasses({
	context: { anchors, palette },
	dark = false,
}: {
	context: BotRenderContext;
	dark?: boolean;
}) {
	const { left, right, radius } = anchors.eyes;
	const rx = radius + 2.4;
	return (
		<g
			stroke={palette.ink}
			strokeWidth="1.8"
			strokeLinecap="round"
			fill={dark ? palette.ink : "none"}
		>
			<ellipse cx={left[0]} cy={left[1]} rx={rx} ry={radius + 1} />
			<ellipse cx={right[0]} cy={right[1]} rx={rx} ry={radius + 1} />
			<path
				d={`M${left[0] + rx} ${left[1]}Q${(left[0] + right[0]) / 2} ${Math.min(left[1], right[1]) - 3} ${right[0] - rx} ${right[1]}`}
				fill="none"
			/>
			<path
				d={`M${left[0] - rx} ${left[1] - 1}h-4M${right[0] + rx} ${right[1] - 1}h4`}
			/>
			{dark && (
				<path
					d={`M${left[0] - 3} ${left[1] - 3}l4 -2M${right[0] - 3} ${right[1] - 3}l4 -2`}
					stroke={palette.sclera}
					opacity="0.65"
				/>
			)}
		</g>
	);
}

function Shirt({
	context: { anchors, palette },
	hood = false,
	turtleneck = false,
}: {
	context: BotRenderContext;
	hood?: boolean;
	turtleneck?: boolean;
}) {
	const y = anchors.neck.y;
	const bottom = anchors.neck.bottom;
	const center = anchors.head.x;
	return (
		<g stroke={palette.ink} strokeWidth="1.3" strokeLinejoin="round">
			<path
				d={`M0 ${y}H${center - 12}Q${center} ${y + 13} ${center + 12} ${y}H100V${bottom + 8}H0Z`}
				fill={palette.clothing}
			/>
			{turtleneck && (
				<path
					d={`M${center - 13} ${y - 1}Q${center} ${y + 4} ${center + 13} ${y - 1}V${y + 6}Q${center} ${y + 11} ${center - 13} ${y + 6}Z`}
					fill={palette.clothing}
				/>
			)}
			{hood && (
				<g fill="none" stroke={palette.sclera} strokeWidth="1.3">
					<path
						d={`M${center - 12} ${y + 1}Q${center} ${y + 13} ${center + 12} ${y + 1}M${center - 6} ${y + 8}v8M${center + 6} ${y + 8}v8`}
					/>
					<path
						d={`M${center - 11} ${bottom - 5}h22l-3 -5H${center - 8}Z`}
						opacity="0.6"
					/>
				</g>
			)}
			{!hood && (
				<path
					d={`M0 ${bottom - 3}H100`}
					stroke={palette.sclera}
					opacity="0.35"
				/>
			)}
		</g>
	);
}

const starterEquipment: readonly BotEquipmentDefinition[] = [
	{
		id: "headset",
		label: "Headset",
		slot: "head",
		render: (context) => (
			<Headpiece context={context}>
				<path
					d="M-25 29V18C-25 -10 25 -10 25 18V29"
					fill="none"
					stroke={context.palette.clothing}
					strokeWidth="5"
				/>
				<rect
					x="-28"
					y="19"
					width="8"
					height="16"
					rx="4"
					fill={context.palette.clothing}
				/>
				<rect
					x="20"
					y="19"
					width="8"
					height="16"
					rx="4"
					fill={context.palette.clothing}
				/>
				<path d="M24 34Q24 44 13 43" fill="none" />
				<rect
					x="8"
					y="40"
					width="8"
					height="5"
					rx="2.5"
					fill={context.palette.ink}
				/>
			</Headpiece>
		),
	},
	{
		id: "crown",
		label: "Crown",
		slot: "head",
		render: (context) => (
			<Headpiece context={context}>
				<path
					d="M-21 2L-24 -16L-10 -8L0 -22L10 -8L24 -16L21 2Z"
					fill={context.palette.accent}
				/>
				<path d="M-20 -2H20" fill="none" />
				<path d="M0 -12L4 -8L0 -4L-4 -8Z" fill={context.palette.clothing} />
			</Headpiece>
		),
	},
	{
		id: "hardhat",
		label: "Hard hat",
		slot: "head",
		render: (context) => (
			<Headpiece context={context}>
				<path d="M-23 0C-23 -25 23 -25 23 0Z" fill={context.palette.accent} />
				<path d="M-3 -18V-3M3 -18V-3" fill="none" />
				<rect
					x="-28"
					y="-2"
					width="56"
					height="6"
					rx="3"
					fill={context.palette.accent}
				/>
			</Headpiece>
		),
	},
	{
		id: "graduation",
		label: "Graduation cap",
		slot: "head",
		render: (context) => (
			<Headpiece context={context}>
				<path d="M-16 -8V2Q0 10 16 2V-8" fill={context.palette.clothing} />
				<path d="M-28 -11L0 -22L28 -11L0 0Z" fill={context.palette.clothing} />
				<path d="M0 -11L23 -7V11" stroke={context.palette.accent} fill="none" />
				<path d="M20 9H26L27 18H19Z" fill={context.palette.accent} />
			</Headpiece>
		),
	},
	{
		id: "glasses",
		label: "Glasses",
		slot: "eyes",
		render: (context) => <Glasses context={context} />,
	},
	{
		id: "sunglasses",
		label: "Sunglasses",
		slot: "eyes",
		render: (context) => <Glasses context={context} dark />,
	},
	{
		id: "vr",
		label: "VR headset",
		slot: "eyes",
		render: ({ anchors: { eyes }, palette }) => {
			const x = eyes.left[0] - eyes.radius - 5;
			const width = eyes.right[0] - eyes.left[0] + 2 * eyes.radius + 10;
			const y = Math.min(eyes.left[1], eyes.right[1]) - eyes.radius - 3;
			return (
				<g stroke={palette.ink} strokeWidth="1.6">
					<path d={`M${x - 5} ${y + 9}h${width + 10}`} strokeWidth="5" />
					<rect
						x={x}
						y={y}
						width={width}
						height={eyes.radius * 2 + 6}
						rx="5"
						fill={palette.clothing}
					/>
					<path
						d={`M${x + 8} ${y + 5}h${width - 16}`}
						stroke={palette.sclera}
						strokeLinecap="round"
						opacity="0.6"
					/>
				</g>
			);
		},
	},
	{
		id: "laptop",
		label: "Laptop",
		slot: "hand",
		compactSkill: "device",
		render: (context) => (
			<Handpiece context={context}>
				<rect
					x="-14"
					y="-12"
					width="28"
					height="19"
					rx="2"
					fill={context.palette.clothing}
				/>
				<path d="M-17 7H17L14 11H-14Z" fill={context.palette.sclera} />
				<circle
					cx="0"
					cy="-3"
					r="2.5"
					fill={context.palette.accent}
					stroke="none"
				/>
			</Handpiece>
		),
	},
	{
		id: "magnifier",
		label: "Magnifying glass",
		slot: "hand",
		compactSkill: "search",
		render: (context) => (
			<Handpiece context={context}>
				<path
					d="M-3 1L-12 13"
					stroke={context.palette.clothing}
					strokeWidth="6"
				/>
				<circle cx="3" cy="-6" r="10" fill={context.palette.sclera} />
				<path
					d="M-2 -10Q3 -15 8 -9"
					stroke={context.palette.accent}
					fill="none"
					strokeWidth="2.5"
				/>
			</Handpiece>
		),
	},
	{
		id: "clipboard",
		label: "Clipboard",
		slot: "hand",
		compactSkill: "tasks",
		render: (context) => (
			<Handpiece context={context}>
				<rect
					x="-11"
					y="-16"
					width="22"
					height="30"
					rx="2"
					fill={context.palette.accent}
				/>
				<rect
					x="-8"
					y="-12"
					width="16"
					height="23"
					rx="1"
					fill={context.palette.sclera}
					stroke="none"
				/>
				<rect
					x="-5"
					y="-18"
					width="10"
					height="6"
					rx="2"
					fill={context.palette.clothing}
				/>
				<path d="M-5 -5L-3 -3L1 -7M3 -5H5M-5 2H5M-5 7H3" fill="none" />
			</Handpiece>
		),
	},
	{
		id: "mug",
		label: "Mug",
		slot: "hand",
		render: (context) => (
			<Handpiece context={context}>
				<path d="M8 -5H13Q21 3 11 7H8" fill="none" strokeWidth="3" />
				<path
					d="M-11 -8H11V6Q11 12 0 12T-11 6Z"
					fill={context.palette.sclera}
				/>
				<path
					d="M-4 -13Q-8 -17 -4 -21M3 -13Q-1 -17 3 -21"
					fill="none"
					opacity="0.5"
				/>
				<path
					d="M-4 0Q0 -5 4 0Q5 3 0 6Q-5 3 -4 0Z"
					fill={context.palette.cheek}
					stroke="none"
				/>
			</Handpiece>
		),
	},
	{
		id: "tool",
		label: "Wrench",
		slot: "hand",
		compactSkill: "tools",
		render: (context) => (
			<Handpiece context={context}>
				<path
					d="M-9 13L2 -2C-5 -9 0 -18 8 -17L4 -10L9 -7L14 -13C19 -5 13 2 7 2L-4 17Z"
					fill={context.palette.clothing}
				/>
				<circle
					cx="-6"
					cy="12"
					r="1.5"
					fill={context.palette.sclera}
					stroke="none"
				/>
			</Handpiece>
		),
	},
	{
		id: "letter",
		label: "Letter",
		slot: "hand",
		compactSkill: "mail",
		render: (context) => (
			<Handpiece context={context}>
				<rect
					x="-15"
					y="-11"
					width="30"
					height="22"
					rx="2"
					fill={context.palette.sclera}
				/>
				<path d="M-14 -10L0 1L14 -10M-14 10L-4 1M14 10L4 1" fill="none" />
				<circle cx="0" cy="1" r="3" fill={context.palette.accent} />
			</Handpiece>
		),
	},
	{
		id: "sweater",
		label: "Sweater",
		slot: "neck",
		render: (context) => <Shirt context={context} />,
	},
	{
		id: "turtleneck",
		label: "Turtleneck",
		slot: "neck",
		render: (context) => <Shirt context={context} turtleneck />,
	},
	{
		id: "hoodie",
		label: "Hoodie",
		slot: "neck",
		render: (context) => <Shirt context={context} hood />,
	},
	{
		id: "scarf",
		label: "Scarf",
		slot: "neck",
		render: ({ anchors, palette }) => (
			<g
				fill={palette.clothing}
				stroke={palette.ink}
				strokeWidth="1.3"
				strokeLinejoin="round"
			>
				<path
					d={`M${anchors.head.x - 7} ${anchors.neck.y + 3}l-4 21h12l3 -21Z`}
				/>
				<path
					d={`M0 ${anchors.neck.y - 2}Q${anchors.head.x} ${anchors.neck.y + 8} 100 ${anchors.neck.y - 2}v9Q${anchors.head.x} ${anchors.neck.y + 17} 0 ${anchors.neck.y + 7}Z`}
				/>
				<path
					d={`M${anchors.head.x - 10} ${anchors.neck.y + 19}h11`}
					stroke={palette.accent}
					strokeWidth="3"
				/>
			</g>
		),
	},
	{
		id: "bowtie",
		label: "Bow tie",
		slot: "neck",
		render: ({ anchors, palette }) => (
			<g
				transform={`translate(${anchors.head.x} ${anchors.neck.y + 6})`}
				fill={palette.clothing}
				stroke={palette.ink}
				strokeWidth="1.2"
				strokeLinejoin="round"
			>
				<path d="M-2 -2L-14 -7V7L-2 2M2 -2L14 -7V7L2 2" />
				<rect x="-3" y="-3" width="6" height="6" rx="2" fill={palette.accent} />
			</g>
		),
	},
	{
		id: "tie",
		label: "Tie",
		slot: "neck",
		render: ({ anchors, palette }) => (
			<g
				transform={`translate(${anchors.head.x} ${anchors.neck.y + 2})`}
				fill={palette.clothing}
				stroke={palette.ink}
				strokeWidth="1.2"
				strokeLinejoin="round"
			>
				<path d="M-3 5L-6 17L0 22L6 17L3 5Z" />
				<path d="M-5 0H5L3 6H-3Z" fill={palette.accent} />
			</g>
		),
	},
];

const starterSkills: readonly BotSkillDefinition[] = [
	{ id: "web", label: "Web", color: "#307da1", glyph: "W" },
	{ id: "code", label: "Code", color: "#7556ad", glyph: "</>" },
	{ id: "chat", label: "Chat", color: "#c36b8a", glyph: "···" },
	{ id: "device", label: "Device", color: "#526f9e", glyph: "▣" },
	{ id: "search", label: "Search", color: "#9b7135", glyph: "⌕" },
	{ id: "tasks", label: "Tasks", color: "#438173", glyph: "✓" },
	{ id: "tools", label: "Tools", color: "#7e637f", glyph: "+" },
	{ id: "mail", label: "Mail", color: "#b96650", glyph: "@" },
	{ id: "files", label: "Files", color: "#887136", glyph: "F" },
	{ id: "data", label: "Data", color: "#477e8c", glyph: "D" },
	{ id: "image", label: "Image", color: "#a56086", glyph: "I" },
	{ id: "music", label: "Music", color: "#8c5eac", glyph: "♪" },
	{ id: "video", label: "Video", color: "#ac635a", glyph: "▶" },
	{ id: "calendar", label: "Calendar", color: "#58865d", glyph: "31" },
	{ id: "automation", label: "Automation", color: "#8c7337", glyph: "↻" },
	{ id: "translate", label: "Translate", color: "#5873a1", glyph: "A" },
	{ id: "shield", label: "Security", color: "#537c70", glyph: "S" },
];

function mergeDefinitions<T extends { id: string }>(
	base: Readonly<Record<string, T>>,
	additions: readonly T[] | undefined,
	category: string,
	copy: (definition: T) => T,
): Readonly<Record<string, T>> {
	const merged = { ...base };
	const seen = new Set<string>();
	for (const definition of additions ?? []) {
		if (typeof definition.id !== "string" || !definition.id.trim()) {
			throw new Error(`A bot ${category} needs a nonempty id.`);
		}
		if (seen.has(definition.id)) {
			throw new Error(`Duplicate bot ${category} id: ${definition.id}`);
		}
		seen.add(definition.id);
		Object.defineProperty(merged, definition.id, {
			value: definition,
			writable: true,
			enumerable: true,
			configurable: true,
		});
	}
	return Object.freeze(
		Object.fromEntries(
			Object.entries(merged).map(([id, definition]) => [id, copy(definition)]),
		),
	);
}

function copyShape(shape: BotShapeDefinition): BotShapeDefinition {
	const { eyes, head, neck, mouth, hand } = shape.anchors;
	if (shape.colorSlots !== undefined) {
		if (!Array.isArray(shape.colorSlots))
			throw new Error(`Invalid color slots for bot shape ${shape.id}.`);
		const seen = new Set<string>();
		for (const slot of shape.colorSlots) {
			if (!(BOT_COLOR_SLOTS as readonly string[]).includes(slot))
				throw new Error(
					`Unknown color slot for bot shape ${shape.id}: ${slot}`,
				);
			if (seen.has(slot))
				throw new Error(
					`Duplicate color slot for bot shape ${shape.id}: ${slot}`,
				);
			seen.add(slot);
		}
	}
	return Object.freeze({
		...shape,
		...(shape.colorSlots
			? { colorSlots: Object.freeze([...shape.colorSlots]) }
			: {}),
		...(shape.bodyOrigin
			? {
					bodyOrigin: Object.freeze([...shape.bodyOrigin]) as readonly [
						number,
						number,
					],
				}
			: {}),
		palette: Object.freeze({ ...shape.palette }),
		anchors: Object.freeze({
			eyes: Object.freeze({
				...eyes,
				left: Object.freeze([...eyes.left]) as readonly [number, number],
				right: Object.freeze([...eyes.right]) as readonly [number, number],
			}),
			head: Object.freeze({ ...head }),
			neck: Object.freeze({ ...neck }),
			mouth: Object.freeze([...mouth]) as readonly [number, number],
			hand: Object.freeze([...hand]) as readonly [number, number],
		}),
	});
}

function requiredHex(value: unknown, source: string): string {
	const normalized = normalizeBotHex(value);
	if (!normalized)
		throw new Error(`Invalid hex color for ${source}; use #RGB or #RRGGBB.`);
	return normalized;
}

function copyColor(color: BotColorDefinition): BotColorDefinition {
	const paint = color.paint;
	if (!paint || typeof paint !== "object")
		throw new Error(`Missing paint for bot color ${color.id}.`);
	const source = `bot color ${color.id}`;
	switch (paint.kind) {
		case "solid":
			return Object.freeze({
				...color,
				paint: Object.freeze({
					kind: "solid",
					color: requiredHex(paint.color, source),
				}),
			});
		case "gradient": {
			if (!Array.isArray(paint.colors) || paint.colors.length < 2)
				throw new Error(`A gradient for ${source} needs at least two colors.`);
			if (
				paint.angle !== undefined &&
				(typeof paint.angle !== "number" || !Number.isFinite(paint.angle))
			)
				throw new Error(`Invalid gradient angle for ${source}.`);
			if (paint.animated !== undefined && typeof paint.animated !== "boolean")
				throw new Error(`Invalid gradient animation flag for ${source}.`);
			return Object.freeze({
				...color,
				paint: Object.freeze({
					kind: "gradient",
					colors: Object.freeze(
						paint.colors.map((value) => requiredHex(value, source)),
					),
					...(paint.angle !== undefined ? { angle: paint.angle } : {}),
					...(paint.animated !== undefined ? { animated: paint.animated } : {}),
				}),
			});
		}
		case "pattern": {
			if (
				!["dots", "stripes", "stars", "diamonds", "hearts"].includes(
					paint.motif,
				)
			)
				throw new Error(`Invalid pattern motif for ${source}.`);
			if (
				paint.scale !== undefined &&
				(typeof paint.scale !== "number" ||
					!Number.isFinite(paint.scale) ||
					paint.scale <= 0)
			)
				throw new Error(`Invalid pattern scale for ${source}.`);
			return Object.freeze({
				...color,
				paint: Object.freeze({
					kind: "pattern",
					base: requiredHex(paint.base, source),
					ink: requiredHex(paint.ink, source),
					motif: paint.motif,
					...(paint.scale !== undefined ? { scale: paint.scale } : {}),
				}),
			});
		}
		default:
			throw new Error(`Unknown paint kind for ${source}.`);
	}
}

function copyColorPack(pack: BotColorPackDefinition): BotColorPackDefinition {
	if (!Array.isArray(pack.colorIds))
		throw new Error(`Bot color pack ${pack.id} needs color IDs.`);
	if (pack.access !== "included" && pack.access !== "premium")
		throw new Error(`Invalid access policy for bot color pack ${pack.id}.`);
	if (
		pack.allowsCustomHex !== undefined &&
		typeof pack.allowsCustomHex !== "boolean"
	)
		throw new Error(
			`Invalid custom hex permission for bot color pack ${pack.id}.`,
		);
	if (!pack.colorIds.length && !pack.allowsCustomHex)
		throw new Error(
			`Bot color pack ${pack.id} must contain colors or allow custom hex.`,
		);
	return Object.freeze({
		...pack,
		colorIds: Object.freeze([...pack.colorIds]),
	});
}

function copyPack(pack: BotPackDefinition): BotPackDefinition {
	return Object.freeze({
		...pack,
		shapeIds: Object.freeze([...pack.shapeIds]),
		...(pack.equipmentIds
			? { equipmentIds: Object.freeze([...pack.equipmentIds]) }
			: {}),
	});
}

function validatePackMembers(
	packId: string,
	ids: readonly string[],
	definitions: Readonly<Record<string, unknown>>,
	category: string,
) {
	const seen = new Set<string>();
	for (const id of ids) {
		if (typeof id !== "string" || !id.trim())
			throw new Error(`Bot pack ${packId} needs nonempty ${category} IDs.`);
		if (seen.has(id))
			throw new Error(`Duplicate ${category} in bot pack ${packId}: ${id}`);
		if (!Object.hasOwn(definitions, id))
			throw new Error(`Unknown ${category} in bot pack ${packId}: ${id}`);
		seen.add(id);
	}
}

/** Extensions replace matching IDs and leave their source definitions untouched. */
export function createBotCatalog(
	extension: BotCatalogExtension,
	base: BotCatalog = defaultBotCatalog,
): BotCatalog {
	const shapes = mergeDefinitions(
		base.shapes,
		extension.shapes,
		"shape",
		copyShape,
	);
	const skills = mergeDefinitions(
		base.skills,
		extension.skills,
		"skill",
		(skill) => Object.freeze({ ...skill }),
	);
	const equipment = mergeDefinitions(
		base.equipment,
		extension.equipment,
		"equipment",
		(item) =>
			Object.freeze({
				...item,
				...(item.kinds ? { kinds: Object.freeze([...item.kinds]) } : {}),
			}),
	);
	const series = mergeDefinitions(
		base.series,
		extension.series,
		"series",
		(item) => Object.freeze({ ...item }),
	);
	const packs = mergeDefinitions(base.packs, extension.packs, "pack", copyPack);
	const colors = mergeDefinitions(
		base.colors ?? {},
		extension.colors,
		"color",
		copyColor,
	);
	const colorPacks = mergeDefinitions(
		base.colorPacks ?? {},
		extension.colorPacks,
		"color pack",
		copyColorPack,
	);
	for (const pack of Object.values(colorPacks)) {
		if (Object.hasOwn(packs, pack.id))
			throw new Error(
				`Bot color pack ${pack.id} shares an ID with a character pack.`,
			);
		validatePackMembers(pack.id, pack.colorIds, colors, "color");
	}
	for (const pack of Object.values(packs)) {
		if (!pack.shapeIds.length && !pack.equipmentIds?.length)
			throw new Error(`Bot pack ${pack.id} must contain content.`);
		validatePackMembers(pack.id, pack.shapeIds, shapes, "shape");
		validatePackMembers(
			pack.id,
			pack.equipmentIds ?? [],
			equipment,
			"equipment",
		);
		if (!Object.hasOwn(shapes, pack.coverKind))
			throw new Error(
				`Unknown cover shape for bot pack ${pack.id}: ${pack.coverKind}`,
			);
		if (!pack.shapeIds.includes(pack.coverKind))
			throw new Error(
				`The cover shape must belong to bot pack ${pack.id}: ${pack.coverKind}`,
			);
	}
	for (const shape of Object.values(shapes)) {
		if (shape.series && !Object.hasOwn(series, shape.series))
			throw new Error(
				`Unknown series for bot shape ${shape.id}: ${shape.series}`,
			);
	}
	const defaultKind = extension.defaultKind ?? base.defaultKind;
	if (!Object.hasOwn(shapes, defaultKind)) {
		throw new Error(`Unknown default bot shape: ${defaultKind}`);
	}
	for (const item of Object.values(equipment)) {
		if (!(BOT_EQUIPMENT_SLOTS as readonly string[]).includes(item.slot)) {
			throw new Error(
				`Unknown slot for bot equipment ${item.id}: ${item.slot}`,
			);
		}
		if (typeof item.render !== "function") {
			throw new Error(`Bot equipment ${item.id} needs a render function.`);
		}
		if (
			item.compactSkill !== undefined &&
			(item.slot !== "hand" || !Object.hasOwn(skills, item.compactSkill))
		) {
			throw new Error(
				`Invalid compact skill for bot equipment ${item.id}: ${item.compactSkill}`,
			);
		}
		for (const kind of item.kinds ?? []) {
			if (!Object.hasOwn(shapes, kind)) {
				throw new Error(`Unknown shape for bot equipment ${item.id}: ${kind}`);
			}
		}
	}
	return Object.freeze({
		defaultKind,
		series,
		packs,
		shapes,
		equipment,
		skills,
		colors,
		colorPacks,
	});
}

export const defaultBotCatalog: BotCatalog = createBotCatalog(
	{
		shapes: crewShapes,
		series: crewSeries,
		packs: crewPacks,
		equipment: [...starterEquipment, ...crewEquipment],
		skills: starterSkills,
		colors: defaultBotColors,
		colorPacks: defaultBotColorPacks,
		defaultKind: "mochi",
	},
	{
		defaultKind: "mochi",
		series: {},
		packs: {},
		shapes: {},
		equipment: {},
		skills: {},
		colors: {},
		colorPacks: {},
	},
);
