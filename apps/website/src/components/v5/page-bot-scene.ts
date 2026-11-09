import { defaultBotCatalog } from "@flow-like/flow-like-ui/components/bots/catalog";
import type {
	BotPalette,
	BotShapeDefinition,
} from "@flow-like/flow-like-ui/components/bots/types";
import * as THREE from "three";
import { RoundedBoxGeometry } from "three/addons/geometries/RoundedBoxGeometry.js";

export type BotKind = "pip" | "mochi" | "flux" | "chip" | "orbi";

export interface BotFrame {
	id: string;
	x: number;
	y: number;
	width: number;
	height: number;
	kind: BotKind;
	tool: "wrench" | "cable" | "driver" | "inspect" | "pencil";
	walk: number;
	work: number;
	reach: number;
	lookX: number;
	lookY: number;
	lean: number;
	time: number;
}

interface BotRig {
	group: THREE.Group;
	body: THREE.Group;
	head: THREE.Group;
	eyes: THREE.Group[];
	pupils: THREE.Group[];
	moon: THREE.Object3D | null;
	moonRadius: number;
	leftHand: THREE.Group;
	rightHand: THREE.Group;
	feet: [THREE.Group, THREE.Group];
	tool: THREE.Group;
	bolt: THREE.Group;
	sparks: THREE.Mesh[];
	shadow: THREE.Mesh;
	previousWalk: number;
	previousTime: number;
	walking: number;
	kind: BotKind;
	toolKind: BotFrame["tool"];
}

/** How each character's flat silhouette becomes a solid: its box in the avatar's 100-unit space. */
interface BodyModel {
	readonly box: {
		readonly x: number;
		readonly y: number;
		readonly width: number;
		readonly height: number;
	};
	readonly depth: number;
	readonly form: "ellipsoid" | "rounded";
	readonly radius?: number;
}

const BODIES: Readonly<Record<BotKind, BodyModel>> = {
	pip: {
		box: { x: 10, y: 37, width: 80, height: 78 },
		depth: 64,
		form: "ellipsoid",
	},
	mochi: {
		box: { x: 14, y: 31, width: 72, height: 72 },
		depth: 56,
		form: "rounded",
		radius: 26,
	},
	flux: {
		box: { x: 22, y: 30, width: 56, height: 56 },
		depth: 38,
		form: "rounded",
		radius: 18,
	},
	chip: {
		box: { x: 20, y: 34, width: 60, height: 72 },
		depth: 24,
		form: "rounded",
		radius: 5,
	},
	orbi: {
		box: { x: 18, y: 36, width: 64, height: 64 },
		depth: 64,
		form: "ellipsoid",
	},
};

const BODY_SIZE = 1.42;
const BODY_BASE = 0.24;
/** The avatars are flat illustrations; self-light keeps the palette readable in shadow. */
const SKIN_GLOW = 0.22;
/** Faces drawn for 200 px avatars need a little more weight at sprite size. */
const FEATURE = 1.2;
const WORK_X = 0.92;

const clamp = (value: number, min = 0, max = 1) =>
	Math.min(max, Math.max(min, value));

function shapeOf(kind: BotKind): BotShapeDefinition {
	return (
		defaultBotCatalog.shapes[kind] ??
		defaultBotCatalog.shapes[defaultBotCatalog.defaultKind]
	);
}

/** Maps avatar coordinates onto a solid body and finds the surface in front of them. */
function bodyFrame(model: BodyModel) {
	const scale = BODY_SIZE / Math.max(model.box.width, model.box.height);
	const width = model.box.width * scale;
	const height = model.box.height * scale;
	const depth = model.depth * scale;
	const centerX = model.box.x + model.box.width / 2;
	const centerY = model.box.y + model.box.height / 2;
	const middle = BODY_BASE + height / 2;
	const point = (x: number, y: number) =>
		new THREE.Vector2((x - centerX) * scale, middle - (y - centerY) * scale);
	const surface = (at: THREE.Vector2) => {
		if (model.form === "rounded")
			return { z: depth / 2, normal: new THREE.Vector3(0, 0, 1) };
		const a = width / 2;
		const b = height / 2;
		const c = depth / 2;
		const y = at.y - middle;
		const z = c * Math.sqrt(Math.max(0, 1 - (at.x / a) ** 2 - (y / b) ** 2));
		const normal = new THREE.Vector3(at.x / a ** 2, y / b ** 2, z / c ** 2);
		return { z, normal: normal.normalize() };
	};
	return { scale, width, height, depth, middle, point, surface };
}

/** All helpers share one WebGL context. Each frame occupies its own CSS-pixel viewport. */
export function createPageBotScene(canvas: HTMLCanvasElement) {
	const renderer = new THREE.WebGLRenderer({
		canvas,
		alpha: true,
		antialias: true,
		stencil: false,
		powerPreference: "low-power",
		premultipliedAlpha: true,
	});
	renderer.autoClear = false;
	renderer.setClearColor(0x000000, 0);
	renderer.outputColorSpace = THREE.SRGBColorSpace;
	renderer.toneMapping = THREE.NeutralToneMapping;
	renderer.toneMappingExposure = 1.05;

	const scene = new THREE.Scene();
	scene.add(new THREE.HemisphereLight(0xffffff, 0xb9b3c4, 1.8));
	const key = new THREE.DirectionalLight(0xfff6ec, 1.7);
	key.position.set(-3, 5, 6);
	scene.add(key);
	const rim = new THREE.DirectionalLight(0xe6f0ff, 1.3);
	rim.position.set(4, 3, -1);
	scene.add(rim);

	const camera = new THREE.OrthographicCamera(-1.3, 1.3, 1.55, -1.55, 0.1, 30);
	// This projection puts y=0 at 82% of each viewport's height.
	camera.position.set(0, 2, 7);
	camera.lookAt(0, 1, 0);

	const geometries = new Map<string, THREE.BufferGeometry>();
	const materials = new Map<string, THREE.Material>();
	const rigs = new Map<string, BotRig>();
	const textures: THREE.Texture[] = [];
	const front = new THREE.Vector3(0, 0, 1);
	let currentWidth = 0;
	let currentHeight = 0;
	let currentDpr = 0;
	let disposed = false;

	function geometry(name: string, make: () => THREE.BufferGeometry) {
		let result = geometries.get(name);
		if (!result) {
			result = make();
			geometries.set(name, result);
		}
		return result;
	}
	const rounded = geometry(
		"rounded",
		() => new RoundedBoxGeometry(1, 1, 1, 2, 0.15),
	);
	const ball = geometry("ball", () => new THREE.SphereGeometry(1, 20, 14));
	const cylinder = geometry(
		"cylinder",
		() => new THREE.CylinderGeometry(1, 1, 1, 10),
	);
	const cone = geometry("cone", () => new THREE.ConeGeometry(1, 1, 8));
	const hexagon = geometry("hex", () => new THREE.CylinderGeometry(1, 1, 1, 6));
	const sparkGeometry = geometry(
		"spark",
		() => new THREE.OctahedronGeometry(1, 0),
	);
	const ring = (radius: number, tube: number, arc = Math.PI * 2) =>
		geometry(
			`ring-${radius}-${tube}-${arc}`,
			() => new THREE.TorusGeometry(radius, tube, 5, 16, arc),
		);
	function material(
		name: string,
		options: THREE.MeshStandardMaterialParameters,
	) {
		let result = materials.get(name);
		if (!result) {
			result = new THREE.MeshStandardMaterial(options);
			materials.set(name, result);
		}
		return result;
	}
	const charcoal = material("charcoal", {
		color: 0x272b34,
		roughness: 0.42,
		metalness: 0.2,
	});
	const rubber = material("rubber", { color: 0x35313a, roughness: 0.8 });
	const silver = material("silver", {
		color: 0xc5cbd2,
		roughness: 0.25,
		metalness: 0.42,
	});
	const visor = material("visor", {
		color: 0x111e2a,
		roughness: 0.16,
		metalness: 0.25,
	});
	const reflection = material("reflection", {
		color: 0xcde4ee,
		transparent: true,
		opacity: 0.2,
		depthWrite: false,
		roughness: 0.2,
	});
	const warmLight = material("light", {
		color: 0xffd5a0,
		emissive: 0xffb45f,
		emissiveIntensity: 0.8,
	});
	const paper = material("paper", { color: 0xf4e7d2, roughness: 0.9 });
	const graphite = material("graphite", { color: 0x62534c, roughness: 0.9 });
	const yellow = material("yellow", { color: 0xeab65e, roughness: 0.36 });

	const shadowPixels = new Uint8Array(32 * 32 * 4);
	for (let y = 0; y < 32; y++) {
		for (let x = 0; x < 32; x++) {
			const radius = Math.hypot((x - 15.5) / 15.5, (y - 15.5) / 15.5);
			const offset = (y * 32 + x) * 4;
			shadowPixels[offset + 3] = Math.round(
				Math.max(0, 1 - radius) ** 1.6 * 110,
			);
		}
	}
	const shadowTexture = new THREE.DataTexture(shadowPixels, 32, 32);
	shadowTexture.needsUpdate = true;
	shadowTexture.magFilter = THREE.LinearFilter;
	shadowTexture.minFilter = THREE.LinearFilter;
	textures.push(shadowTexture);
	const shadowMaterial = new THREE.MeshBasicMaterial({
		map: shadowTexture,
		transparent: true,
		depthWrite: false,
		toneMapped: false,
	});
	materials.set("shadow", shadowMaterial);

	function mesh(
		parent: THREE.Object3D,
		shape: THREE.BufferGeometry,
		paint: THREE.Material,
		x: number,
		y: number,
		z: number,
		sx = 1,
		sy = sx,
		sz = sx,
	) {
		const object = new THREE.Mesh(shape, paint);
		object.position.set(x, y, z);
		object.scale.set(sx, sy, sz);
		parent.add(object);
		return object;
	}

	function paint(
		hex: string,
		options: THREE.MeshStandardMaterialParameters = {},
	) {
		const key = `${hex}-${JSON.stringify(options)}`;
		return material(key, {
			color: new THREE.Color(hex),
			roughness: 0.6,
			metalness: 0,
			...options,
		});
	}

	function toolFor(kind: BotFrame["tool"], trim: THREE.Material) {
		const tool = new THREE.Group();
		if (kind === "wrench") {
			mesh(tool, rounded, silver, 0, -0.12, 0.12, 0.07, 0.32, 0.045);
			mesh(
				tool,
				ring(0.075, 0.032, Math.PI * 1.5),
				silver,
				0,
				-0.33,
				0.12,
			).rotation.z = -Math.PI / 4;
			mesh(tool, ring(0.033, 0.015), silver, 0, 0.055, 0.12);
		} else if (kind === "driver") {
			mesh(tool, rounded, trim, 0, -0.035, 0.11, 0.12, 0.2, 0.12);
			mesh(tool, rounded, rubber, 0, -0.035, 0.17, 0.035, 0.14, 0.025);
			mesh(tool, cylinder, silver, 0, -0.24, 0.11, 0.023, 0.25, 0.023);
			mesh(tool, rounded, silver, 0, -0.375, 0.11, 0.048, 0.035, 0.02);
		} else if (kind === "cable") {
			const curve = geometry(
				"cable",
				() =>
					new THREE.TubeGeometry(
						new THREE.CatmullRomCurve3([
							new THREE.Vector3(0, -0.19, 0.1),
							new THREE.Vector3(-0.17, -0.1, 0.1),
							new THREE.Vector3(-0.25, -0.29, 0.1),
							new THREE.Vector3(-0.12, -0.41, 0.1),
						]),
						12,
						0.024,
						5,
						false,
					),
			);
			mesh(tool, curve, trim, 0, 0, 0);
			mesh(tool, rounded, charcoal, 0, -0.23, 0.11, 0.15, 0.15, 0.12);
			mesh(tool, rounded, silver, 0, -0.34, 0.11, 0.08, 0.09, 0.06);
			mesh(tool, rounded, warmLight, 0, -0.22, 0.18, 0.07, 0.025, 0.01);
		} else if (kind === "inspect") {
			mesh(tool, cylinder, trim, 0, -0.1, 0.12, 0.04, 0.22, 0.04);
			mesh(tool, ring(0.12, 0.025), silver, 0, -0.3, 0.12);
			const lens = material("lens", {
				color: 0x8ad0df,
				transparent: true,
				opacity: 0.35,
				roughness: 0.08,
				depthWrite: false,
			});
			mesh(tool, ball, lens, 0, -0.3, 0.12, 0.105, 0.105, 0.014);
			mesh(
				tool,
				rounded,
				reflection,
				-0.04,
				-0.265,
				0.14,
				0.04,
				0.075,
				0.01,
			).rotation.z = -0.5;
		} else {
			mesh(tool, cylinder, yellow, 0, -0.1, 0.12, 0.036, 0.33, 0.036);
			mesh(tool, cylinder, silver, 0, 0.045, 0.12, 0.038, 0.05, 0.038);
			mesh(tool, ball, trim, 0, 0.075, 0.12, 0.037, 0.04, 0.037);
			mesh(tool, cone, paper, 0, -0.31, 0.12, 0.036, 0.09, 0.036).rotation.z =
				Math.PI;
			mesh(
				tool,
				cone,
				graphite,
				0,
				-0.36,
				0.12,
				0.012,
				0.035,
				0.012,
			).rotation.z = Math.PI;
		}
		return tool;
	}

	function bodyGeometry(kind: BotKind, model: BodyModel, palette: BotPalette) {
		const frame = bodyFrame(model);
		return geometry(`body-${kind}`, () => {
			if (model.form === "ellipsoid") {
				const sphere = new THREE.SphereGeometry(1, 40, 28);
				sphere.scale(frame.width / 2, frame.height / 2, frame.depth / 2);
				return sphere;
			}
			const box = new RoundedBoxGeometry(
				frame.width,
				frame.height,
				frame.depth,
				8,
				(model.radius ?? 0) * frame.scale,
			);
			if (kind === "flux") {
				// The Flow-Like icon blends from the accent at the top left into the body color.
				const from = new THREE.Color(palette.accent);
				const to = new THREE.Color(palette.body);
				const position = box.getAttribute("position");
				const colors = new Float32Array(position.count * 3);
				const color = new THREE.Color();
				for (let index = 0; index < position.count; index++) {
					const t = clamp(
						(position.getX(index) / frame.width -
							position.getY(index) / frame.height +
							1) /
							2,
					);
					color.copy(from).lerp(to, t ** 0.85);
					color.toArray(colors, index * 3);
				}
				box.setAttribute("color", new THREE.BufferAttribute(colors, 3));
			}
			return box;
		});
	}

	/** Places a face part on the body surface, turned to follow its curve. */
	function onSurface(
		parent: THREE.Object3D,
		frame: ReturnType<typeof bodyFrame>,
		x: number,
		y: number,
		lift = 0.004,
	) {
		const at = frame.point(x, y);
		const { z, normal } = frame.surface(at);
		const holder = new THREE.Group();
		holder.position.set(at.x, at.y, z).addScaledVector(normal, lift);
		holder.quaternion.setFromUnitVectors(front, normal);
		parent.add(holder);
		return holder;
	}

	/** A rounded line through two points, or a quadratic curve through three. */
	function stroke(
		points: readonly [number, number][],
		width: number,
		key: string,
	) {
		return geometry(key, () => {
			const [start, middle, end] = points.map(
				([x, y]) => new THREE.Vector3(x, y, 0),
			);
			const curve = end
				? new THREE.QuadraticBezierCurve3(start, middle, end)
				: new THREE.LineCurve3(start, middle);
			return new THREE.TubeGeometry(curve, 16, width / 2, 6, false);
		});
	}

	function buildFace(
		head: THREE.Group,
		kind: BotKind,
		shape: BotShapeDefinition,
		frame: ReturnType<typeof bodyFrame>,
	) {
		const { palette, anchors } = shape;
		const s = frame.scale * FEATURE;
		const style = shape.eyeStyle ?? "round";
		const round = style === "round" || style === "sleepy";
		const light = style === "light";
		const eyeHex = light ? palette.sclera : palette.ink;
		const pupilPaint = paint(eyeHex, {
			roughness: 0.25,
			emissive: light ? new THREE.Color(eyeHex) : new THREE.Color(0),
			emissiveIntensity: light ? 0.35 : 0,
		});
		const sclera = paint(palette.sclera, { roughness: 0.35 });
		const radius = anchors.eyes.radius;
		const eyes: THREE.Group[] = [];
		const pupils: THREE.Group[] = [];
		for (const [x, y] of [anchors.eyes.left, anchors.eyes.right]) {
			const eye = new THREE.Group();
			onSurface(head, frame, x, y).add(eye);
			const pupil = new THREE.Group();
			eye.add(pupil);
			if (round) {
				mesh(
					eye,
					ball,
					sclera,
					0,
					0,
					0,
					radius * s,
					radius * 1.12 * s,
					radius * 0.3 * s,
				);
				mesh(
					pupil,
					ball,
					pupilPaint,
					0,
					-0.5 * s,
					radius * 0.2 * s,
					radius * 0.49 * s,
					radius * 0.59 * s,
					radius * 0.2 * s,
				);
				mesh(
					pupil,
					ball,
					sclera,
					-radius * 0.16 * s,
					radius * 0.2 * s,
					radius * 0.34 * s,
					radius * 0.17 * s,
				);
			} else {
				mesh(
					pupil,
					ball,
					pupilPaint,
					0,
					0,
					0,
					radius * 0.8 * s,
					radius * 1.12 * s,
					radius * 0.32 * s,
				);
				if (!light)
					mesh(
						pupil,
						ball,
						sclera,
						-radius * 0.22 * s,
						radius * 0.36 * s,
						radius * 0.26 * s,
						radius * 0.22 * s,
					);
			}
			eyes.push(eye);
			pupils.push(pupil);
		}

		const [mouthX, mouthY] = anchors.mouth;
		const mouthStyle = shape.mouthStyle ?? "smile";
		if (mouthStyle !== "none") {
			const mouthScale = (shape.mouthScale ?? 1) * s;
			const path: [number, number][] =
				mouthStyle === "line"
					? [
							[-3, 0],
							[3, 0],
						]
					: [
							[-3, -1],
							[0, 3],
							[3, -1],
						];
			const mouth = onSurface(head, frame, mouthX, mouthY);
			const tube = mesh(
				mouth,
				stroke(
					path.map(([px, py]) => [px * mouthScale, -py * mouthScale]),
					1.7 * mouthScale,
					`mouth-${kind}`,
				),
				paint(palette[shape.mouthColor ?? "ink"], {
					roughness: 0.3,
					emissive: light
						? new THREE.Color(palette[shape.mouthColor ?? "ink"])
						: new THREE.Color(0),
					emissiveIntensity: light ? 0.3 : 0,
				}),
				0,
				0,
				0,
			);
			tube.renderOrder = 1;
		}

		if (shape.cheeks) {
			const blush = paint(palette.cheek, {
				transparent: true,
				opacity: 0.75,
				depthWrite: false,
			});
			for (const [x, side] of [
				[anchors.eyes.left[0] - 6, -1],
				[anchors.eyes.right[0] + 6, 1],
			] as const) {
				const cheek = onSurface(head, frame, x, mouthY - 1, 0.006);
				mesh(
					cheek,
					ball,
					blush,
					0,
					0,
					0,
					3.5 * s,
					1.8 * s,
					0.4 * s,
				).rotation.z = side * 0.05;
			}
		}
		return { eyes, pupils };
	}

	function buildDetails(
		head: THREE.Group,
		kind: BotKind,
		palette: BotPalette,
		frame: ReturnType<typeof bodyFrame>,
	) {
		const s = frame.scale;
		if (kind === "flux") {
			const glow = paint(palette.sclera, {
				transparent: true,
				opacity: 0.72,
				roughness: 0.2,
			});
			const points = (
				[
					[29, 49],
					[30, 37],
					[39, 36],
				] as const
			).map(([x, y]): [number, number] => {
				const at = frame.point(x, y);
				return [at.x, at.y];
			});
			mesh(
				head,
				stroke(points, 3 * s, "flux-shine"),
				glow,
				0,
				0,
				frame.depth / 2 + 0.006,
			);
		}
		if (kind === "chip") {
			const pins = paint(palette.accent, { roughness: 0.3, metalness: 0.35 });
			const pinDepth = frame.depth * 0.55;
			for (const x of [30, 42, 54, 66]) {
				const at = frame.point(x + 2, 30);
				mesh(head, rounded, pins, at.x, at.y, 0, 4 * s, 8 * s, pinDepth);
			}
			for (const y of [47, 59, 71, 83]) {
				for (const x of [16, 84]) {
					const at = frame.point(x, y + 2);
					mesh(head, rounded, pins, at.x, at.y, 0, 8 * s, 4 * s, pinDepth);
				}
			}
		}
		if (kind === "orbi") {
			const spots = paint(palette.ink, {
				transparent: true,
				opacity: 0.14,
				depthWrite: false,
			});
			for (const [x, y, r] of [
				[33, 55, 6],
				[65, 78, 4],
			] as const) {
				mesh(
					onSurface(head, frame, x, y, 0.003),
					ball,
					spots,
					0,
					0,
					0,
					r * s,
					r * s,
					r * 0.15 * s,
				);
			}
			const orbit = new THREE.Group();
			const center = frame.point(50, 65);
			orbit.position.set(center.x, center.y, 0);
			// Tilted so the ring's front passes below the planet's middle, as in the avatar.
			orbit.rotation.x = -Math.acos(11 / 40);
			orbit.rotation.z = -0.08;
			head.add(orbit);
			const radius = 40 * s;
			mesh(
				orbit,
				geometry(
					"orbi-ring",
					() => new THREE.TorusGeometry(radius, 1.9 * s, 10, 64),
				),
				paint(palette.accent, { roughness: 0.45 }),
				0,
				0,
				0,
			);
			const moon = mesh(
				orbit,
				ball,
				paint(palette.sclera, { roughness: 0.5 }),
				radius,
				0,
				0,
				3.2 * s,
			);
			return { moon, moonRadius: radius };
		}
		return { moon: null, moonRadius: 0 };
	}

	function createRig(frame: BotFrame): BotRig {
		const shape = shapeOf(frame.kind);
		const palette = shape.palette;
		const model = BODIES[frame.kind];
		const bodyFrameInfo = bodyFrame(model);
		const group = new THREE.Group();
		const body = new THREE.Group();
		group.add(body);

		const bodyColor = new THREE.Color(palette.body);
		const dark = bodyColor.getHSL({ h: 0, s: 0, l: 0 }).l < 0.3;
		const head = new THREE.Group();
		head.position.set(0, bodyFrameInfo.middle, 0);
		body.add(head);
		const inner = new THREE.Group();
		inner.position.set(0, -bodyFrameInfo.middle, 0);
		head.add(inner);
		const skin =
			frame.kind === "flux"
				? material("skin-flux", {
						color: 0xffffff,
						vertexColors: true,
						roughness: 0.42,
						metalness: 0,
					})
				: paint(palette.body, {
						roughness: dark ? 0.45 : 0.6,
						emissive: new THREE.Color(palette.body),
						emissiveIntensity: dark ? SKIN_GLOW * 2.6 : SKIN_GLOW,
					});
		mesh(
			inner,
			bodyGeometry(frame.kind, model, palette),
			skin,
			0,
			bodyFrameInfo.middle,
			0,
		);
		const { eyes, pupils } = buildFace(inner, frame.kind, shape, bodyFrameInfo);
		const { moon, moonRadius } = buildDetails(
			inner,
			frame.kind,
			palette,
			bodyFrameInfo,
		);

		const handHex = dark
			? palette.accent
			: frame.kind === "flux"
				? palette.accent
				: `#${bodyColor.clone().lerp(new THREE.Color(0xffffff), 0.22).getHexString()}`;
		const footHex = `#${bodyColor
			.clone()
			.lerp(new THREE.Color(palette.ink), dark ? 0.15 : 0.32)
			.getHexString()}`;
		const handPaint = paint(handHex, { roughness: 0.5 });
		const footPaint = paint(footHex, { roughness: 0.7 });

		const hand = () => {
			const object = new THREE.Group();
			mesh(object, ball, handPaint, 0, 0, 0, 0.115, 0.11, 0.11);
			body.add(object);
			return object;
		};
		const leftHand = hand();
		const rightHand = hand();
		const tool = toolFor(frame.tool, handPaint);
		rightHand.add(tool);

		const feet = [-1, 1].map((side) => {
			const foot = new THREE.Group();
			foot.position.set(side * 0.27, 0.2, 0.04);
			mesh(foot, ball, footPaint, 0, -0.08, 0.05, 0.16, 0.1, 0.21);
			body.add(foot);
			return foot;
		}) as [THREE.Group, THREE.Group];

		const bolt = new THREE.Group();
		bolt.position.set(WORK_X, 0.018, 0.18);
		group.add(bolt);
		if (frame.tool === "pencil") {
			mesh(bolt, rounded, paper, 0, 0, 0, 0.43, 0.02, 0.34).rotation.y = -0.2;
			for (let index = 0; index < 3; index++)
				mesh(
					bolt,
					rounded,
					graphite,
					-0.03,
					0.014,
					-0.08 + index * 0.07,
					0.18 - index * 0.035,
					0.006,
					0.009,
				);
		} else if (frame.tool === "cable") {
			mesh(bolt, rounded, silver, 0, 0.02, 0, 0.22, 0.09, 0.18);
			mesh(bolt, rounded, visor, 0, 0.07, 0, 0.12, 0.02, 0.08);
		} else {
			mesh(bolt, hexagon, silver, 0, 0.015, 0, 0.105, 0.055, 0.105);
			mesh(bolt, rounded, charcoal, 0, 0.046, 0, 0.105, 0.012, 0.018);
		}
		const shadow = mesh(
			group,
			geometry("plane", () => new THREE.PlaneGeometry(1, 1)),
			shadowMaterial,
			0.07,
			0.002,
			0,
			1.75,
			0.95,
			1,
		);
		shadow.rotation.x = -Math.PI / 2;
		const sparks = Array.from({ length: 4 }, () => {
			const particle = mesh(
				group,
				sparkGeometry,
				warmLight,
				0,
				0,
				0,
				0.018,
				0.055,
				0.018,
			);
			particle.visible = false;
			return particle;
		});
		group.visible = false;
		scene.add(group);
		return {
			group,
			body,
			head,
			eyes,
			pupils,
			moon,
			moonRadius,
			leftHand,
			rightHand,
			feet,
			tool,
			bolt,
			sparks,
			shadow,
			previousWalk: frame.walk,
			previousTime: frame.time,
			walking: 0,
			kind: frame.kind,
			toolKind: frame.tool,
		};
	}

	function updateRig(rig: BotRig, frame: BotFrame) {
		const work = clamp(frame.work);
		const reach = clamp(frame.reach);
		const contact = work * reach;
		const delta = frame.time - rig.previousTime;
		if (delta > 0) {
			const target = clamp(Math.abs(frame.walk - rig.previousWalk) / delta / 4);
			rig.walking += (target - rig.walking) * (1 - Math.exp(-delta * 14));
			rig.previousWalk = frame.walk;
			rig.previousTime = frame.time;
		}
		// Repainting a paused frame must retain the exact pose, including a lifted foot.
		const walking = rig.walking * (1 - contact * 0.95);
		const stride = Math.sin(frame.walk);
		const bob = Math.abs(stride) * 0.05 * walking * (1 - contact);
		const squash =
			(Math.abs(Math.cos(frame.walk)) * 0.035 * walking + contact * 0.02) *
			(1 - 0.5 * reach);
		const tilt = clamp(frame.lean, -1, 1) * 0.1;
		const lookX = clamp(frame.lookX, -1, 1);
		const lookY = clamp(frame.lookY, -1, 1);
		rig.body.position.y = bob;
		rig.body.rotation.z = tilt;
		rig.body.rotation.y =
			clamp(frame.lean, -1, 1) * walking * (1 - reach) * 0.38;
		rig.head.scale.set(1 + squash, 1 - squash, 1 + squash * 0.5);
		rig.head.rotation.y = lookX * 0.26;
		rig.head.rotation.x = lookY * 0.12 + reach * 0.14;
		rig.head.rotation.z = Math.sin(frame.time * 1.3) * 0.025 * (1 - work);
		for (const pupil of rig.pupils) {
			pupil.position.x = lookX * 0.03;
			pupil.position.y = -lookY * 0.02;
		}
		const blink = (frame.time + frame.id.length * 0.47) % 5.3;
		for (const eye of rig.eyes) eye.scale.y = blink < 0.11 ? 0.15 : 1;
		if (rig.moon) {
			const angle = frame.time * 0.9;
			rig.moon.position.set(
				Math.cos(angle) * rig.moonRadius,
				Math.sin(angle) * rig.moonRadius,
				0,
			);
		}
		rig.feet[0].rotation.x = stride * 0.32 * walking;
		rig.feet[1].rotation.x = -stride * 0.32 * walking;
		rig.feet[0].position.y = 0.2 + Math.max(0, stride) * 0.07 * walking;
		rig.feet[1].position.y = 0.2 + Math.max(0, -stride) * 0.07 * walking;
		rig.leftHand.position.set(
			-0.84 - reach * 0.04,
			0.72 +
				Math.max(0, stride) * 0.12 * walking +
				Math.sin(frame.time * 1.6) * 0.015,
			0.18 + stride * 0.12 * walking,
		);

		const writing =
			frame.tool === "pencil" ? Math.sin(frame.time * 6) * 0.035 * contact : 0;
		const handX = 0.86 + reach * 0.06 + writing;
		const handY =
			0.66 - reach * 0.3 + Math.sin(frame.time * 8) * 0.012 * contact;
		// Inverse body lean keeps the working tip on the page surface.
		const localX = handX * Math.cos(tilt) + (handY - bob) * Math.sin(tilt);
		const localY = -handX * Math.sin(tilt) + (handY - bob) * Math.cos(tilt);
		rig.rightHand.position.set(localX, localY, 0.16);
		rig.rightHand.rotation.z = -tilt + (1 - reach) * -0.25;
		rig.tool.rotation.y =
			frame.tool === "driver" || frame.tool === "wrench"
				? Math.sin(frame.time * 7) * 0.32 * contact
				: 0;
		rig.bolt.rotation.y =
			frame.tool === "wrench" || frame.tool === "driver"
				? Math.sin(frame.time * 7) * 0.08 * contact
				: 0;
		rig.shadow.scale.x = 1.75 - bob;
		const canSpark =
			contact > 0.65 && ["wrench", "driver", "cable"].includes(frame.tool);
		for (let index = 0; index < rig.sparks.length; index++) {
			const particle = rig.sparks[index];
			const progress = (frame.time * 2.8 + index * 0.19) % 1;
			particle.visible = canSpark && progress < 0.52;
			if (!particle.visible) continue;
			const angle = index * 1.7 + 0.4;
			particle.position.set(
				WORK_X + Math.cos(angle) * progress * 0.28,
				0.065 + Math.sin(progress * Math.PI) * 0.18,
				0.2 + Math.sin(angle) * progress * 0.17,
			);
			particle.rotation.z = -angle;
			particle.scale.setScalar(0.025 * (1 - progress));
			particle.scale.y *= 2.7;
		}
	}

	return {
		render(
			frames: readonly BotFrame[],
			width: number,
			height: number,
			dpr: number,
		) {
			if (
				disposed ||
				width <= 0 ||
				height <= 0 ||
				renderer.getContext().isContextLost()
			)
				return;
			const pixelRatio = clamp(dpr || 1, 0.75, 1.5);
			if (
				width !== currentWidth ||
				height !== currentHeight ||
				pixelRatio !== currentDpr
			) {
				currentWidth = width;
				currentHeight = height;
				currentDpr = pixelRatio;
				renderer.setPixelRatio(pixelRatio);
				renderer.setSize(width, height, false);
			}
			renderer.setScissorTest(false);
			renderer.setViewport(0, 0, width, height);
			renderer.clear(true, true, false);
			renderer.setScissorTest(true);
			for (const frame of frames) {
				if (
					frame.width <= 0 ||
					frame.height <= 0 ||
					frame.x >= width ||
					frame.y >= height ||
					frame.x + frame.width <= 0 ||
					frame.y + frame.height <= 0
				)
					continue;
				let rig = rigs.get(frame.id);
				if (!rig || rig.kind !== frame.kind || rig.toolKind !== frame.tool) {
					if (rig) scene.remove(rig.group);
					rig = createRig(frame);
					rigs.set(frame.id, rig);
				}
				updateRig(rig, frame);
				const ratio = frame.width / frame.height;
				camera.left = -1.55 * ratio;
				camera.right = 1.55 * ratio;
				camera.updateProjectionMatrix();
				renderer.setViewport(
					frame.x,
					height - frame.y - frame.height,
					frame.width,
					frame.height,
				);
				const left = Math.max(0, frame.x);
				const top = Math.max(0, frame.y);
				const right = Math.min(width, frame.x + frame.width);
				const bottom = Math.min(height, frame.y + frame.height);
				renderer.setScissor(left, height - bottom, right - left, bottom - top);
				renderer.clearDepth();
				rig.group.visible = true;
				renderer.render(scene, camera);
				rig.group.visible = false;
			}
			renderer.setScissorTest(false);
		},
		dispose() {
			if (disposed) return;
			disposed = true;
			rigs.clear();
			scene.clear();
			for (const value of geometries.values()) value.dispose();
			for (const value of materials.values()) value.dispose();
			for (const value of textures) value.dispose();
			renderer.dispose();
			renderer.forceContextLoss();
		},
	};
}
