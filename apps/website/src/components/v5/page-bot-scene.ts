import { defaultBotCatalog } from "@flow-like/flow-like-ui/components/bots/catalog";
import type {
	BotPalette,
	BotShapeDefinition,
} from "@flow-like/flow-like-ui/components/bots/types";
import * as THREE from "three";
import { RoundedBoxGeometry } from "three/addons/geometries/RoundedBoxGeometry.js";

export type BotKind = "pip" | "mochi" | "flux" | "chip" | "orbi";
/** What a helper does once it stops walking: one readable, chest-high activity per character. */
export type BotActivity = "build" | "juggle" | "plane" | "scout" | "read";

export interface BotFrame {
	id: string;
	x: number;
	y: number;
	width: number;
	height: number;
	kind: BotKind;
	activity: BotActivity;
	walk: number;
	work: number;
	reach: number;
	lookX: number;
	lookY: number;
	lean: number;
	time: number;
}

interface ActivityInput {
	readonly time: number;
	readonly reach: number;
	readonly contact: number;
	readonly rightIdle: THREE.Vector3;
}

/** Hand targets are in body space; the rig blends them in by `reach`. */
interface ActivityPose {
	readonly left: THREE.Vector3;
	readonly right: THREE.Vector3;
	lookX: number;
	lookY: number;
	yaw: number;
	squint: number;
}

type ActivityRig = (input: ActivityInput, pose: ActivityPose) => void;

interface RigParts {
	readonly id: string;
	readonly body: THREE.Group;
	readonly head: THREE.Group;
	readonly headOffset: number;
	readonly rightEye: THREE.Vector3;
	readonly palette: BotPalette;
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
	shadow: THREE.Mesh;
	activity: ActivityRig;
	previousWalk: number;
	previousTime: number;
	walking: number;
	kind: BotKind;
	activityKind: BotActivity;
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
const TAU = Math.PI * 2;

const clamp = (value: number, min = 0, max = 1) =>
	Math.min(max, Math.max(min, value));
const mix = (from: number, to: number, amount: number) =>
	from + (to - from) * amount;
const smooth = (value: number) => {
	const t = clamp(value);
	return t * t * (3 - 2 * t);
};
const fract = (value: number) => value - Math.floor(value);
const quaternion = (
	x: number,
	y: number,
	z: number,
	order: THREE.EulerOrder = "XYZ",
) => new THREE.Quaternion().setFromEuler(new THREE.Euler(x, y, z, order));

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
	const idleLeft = new THREE.Vector3();
	const idleRight = new THREE.Vector3();
	const pose: ActivityPose = {
		left: new THREE.Vector3(),
		right: new THREE.Vector3(),
		lookX: 0,
		lookY: 0,
		yaw: 0,
		squint: 0,
	};
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
	const paperPlane = geometry("paper-plane", () => {
		const nose = [0.19, 0, 0];
		const keel = [-0.14, 0, 0];
		const fin = [-0.12, -0.065, 0];
		const leftTip = [-0.15, 0.035, -0.15];
		const rightTip = [-0.15, 0.035, 0.15];
		const result = new THREE.BufferGeometry();
		result.setAttribute(
			"position",
			new THREE.Float32BufferAttribute(
				[nose, keel, leftTip, nose, rightTip, keel, nose, fin, keel].flat(),
				3,
			),
		);
		result.computeVertexNormals();
		return result;
	});
	function material(
		name: string,
		options: THREE.MeshStandardMaterialParameters,
	) {
		let result = materials.get(name);
		if (!result) {
			result = new THREE.MeshStandardMaterial(options);
			materials.set(name, result);
		}
		return result as THREE.MeshStandardMaterial;
	}
	const silver = material("silver", {
		color: 0xc5cbd2,
		roughness: 0.25,
		metalness: 0.42,
	});
	const glass = material("glass", {
		color: 0x111e2a,
		roughness: 0.16,
		metalness: 0.25,
	});
	const brass = material("brass", {
		color: 0xe0ad5a,
		roughness: 0.32,
		metalness: 0.25,
	});
	const brassDark = material("brass-dark", {
		color: 0xa8763c,
		roughness: 0.4,
		metalness: 0.25,
	});
	const paper = material("paper", { color: 0xf4e7d2, roughness: 0.9 });
	const folded = material("paper-folded", {
		color: 0xd6e9ff,
		emissive: 0xb9d6ff,
		emissiveIntensity: 0.3,
		roughness: 0.8,
		side: THREE.DoubleSide,
	});
	const trailPaint = material("plane-trail", {
		color: 0xb9d6ff,
		emissive: 0xb9d6ff,
		emissiveIntensity: 0.4,
		transparent: true,
		opacity: 0.7,
	});
	const graphite = material("graphite", { color: 0x62534c, roughness: 0.9 });
	const holoGlass = material("holo-glass", {
		color: 0x2b2530,
		emissive: 0x3a2a30,
		emissiveIntensity: 0.5,
		roughness: 0.3,
		transparent: true,
		opacity: 0.9,
	});
	const holoRim = material("holo-rim", {
		color: 0xffb39a,
		emissive: 0xff987a,
		emissiveIntensity: 0.9,
	});
	const dataCubes = [0x87c8ed, 0xdff7ff, 0xa8d8a2].map((hex) =>
		material(`data-${hex}`, {
			color: hex,
			emissive: hex,
			emissiveIntensity: 0.45,
			roughness: 0.35,
		}),
	);

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

	/** Flux taps a floating flow panel; its nodes light up and wire together, then it starts over. */
	function buildActivity({ id, body }: RigParts): ActivityRig {
		const panel = new THREE.Group();
		panel.position.set(0.72, 0.68, 0.8);
		panel.rotation.set(-0.2, -0.2, 0);
		body.add(panel);
		mesh(panel, rounded, holoGlass, 0, 0, 0, 0.82, 0.54, 0.035);
		mesh(panel, rounded, holoRim, 0, 0.245, 0.02, 0.72, 0.018, 0.01);
		const nodes = [
			{ x: -0.25, y: -0.03, hex: 0xff987a },
			{ x: 0.02, y: 0.1, hex: 0xc0aae6 },
			{ x: 0.02, y: -0.15, hex: 0xa2d4ad },
			{ x: 0.27, y: -0.03, hex: 0xefd28e },
		];
		const lit = (name: string, hex: number) => {
			const result = new THREE.MeshStandardMaterial({
				color: hex,
				emissive: hex,
				emissiveIntensity: 0.1,
				roughness: 0.4,
			});
			materials.set(`${id}-${name}`, result);
			return result;
		};
		const nodePaint = nodes.map((node, index) => {
			const result = lit(`node-${index}`, node.hex);
			mesh(panel, rounded, result, node.x, node.y, 0.032, 0.15, 0.11, 0.04);
			return result;
		});
		const links = [
			[0, 1],
			[0, 2],
			[1, 3],
			[2, 3],
		] as const;
		const wirePaint = links.map(([from, to], index) => {
			const result = lit(`wire-${index}`, 0xffb39a);
			const dx = nodes[to].x - nodes[from].x;
			const dy = nodes[to].y - nodes[from].y;
			mesh(
				panel,
				rounded,
				result,
				(nodes[from].x + nodes[to].x) / 2,
				(nodes[from].y + nodes[to].y) / 2,
				0.022,
				Math.hypot(dx, dy),
				0.02,
				0.008,
			).rotation.z = Math.atan2(dy, dx);
			return result;
		});
		panel.updateMatrix();
		const stops = [
			...nodes.map((node) =>
				new THREE.Vector3(node.x, node.y, 0.1).applyMatrix4(panel.matrix),
			),
			new THREE.Vector3(0.95, 0.5, 0.5),
		];
		return ({ time, reach }, pose) => {
			const shown = smooth(reach * 1.5 - 0.3);
			panel.visible = shown > 0.01;
			panel.scale.setScalar(Math.max(0.001, shown));
			const beat = time / 0.8;
			const step = Math.floor(beat) % stops.length;
			const local = fract(beat);
			for (let index = 0; index < nodePaint.length; index++)
				nodePaint[index].emissiveIntensity =
					index < step
						? 0.75
						: index === step
							? 0.1 + 0.8 * smooth(local * 3 - 0.9)
							: 0.1;
			for (let index = 0; index < links.length; index++)
				wirePaint[index].emissiveIntensity =
					links[index][1] < step || (links[index][1] === step && local > 0.45)
						? 0.95
						: 0.04;
			const from = stops[(step + stops.length - 1) % stops.length];
			const to = stops[step];
			const travel = smooth(local / 0.35);
			const press =
				step < nodes.length
					? Math.sin(clamp((local - 0.3) / 0.3) * Math.PI)
					: 0;
			pose.right.lerpVectors(from, to, travel);
			pose.right.z -= press * 0.07;
			pose.left.set(-0.8, 0.64 + Math.sin(time * 1.6) * 0.015, 0.26);
			pose.lookX = 0.6;
			pose.lookY = 0.55;
			pose.yaw = 0.16;
		};
	}

	/** Byte juggles three data cubes in a cascade: catch outside, carry in, throw across. */
	function juggleActivity({ body }: RigParts): ActivityRig {
		const cubeSize = 0.2;
		const cubes = dataCubes.map((paint) =>
			mesh(body, rounded, paint, 0, 0, 0, cubeSize),
		);
		const rate = 0.55;
		const handY = 0.9;
		const handZ = 0.44;
		const inner = 0.32;
		const outer = 0.62;
		const held = new THREE.Vector3();
		const juggled = new THREE.Vector3();
		const handAt = (phase: number, side: number, out: THREE.Vector3) => {
			if (phase >= 0.7) {
				const carry = (phase - 0.7) / 0.3;
				return out.set(
					side * mix(outer, inner, smooth(carry)),
					handY - 0.08 * Math.sin(carry * Math.PI),
					handZ,
				);
			}
			const back = phase / 0.7;
			return out.set(
				side * mix(inner, outer, smooth(back)),
				handY + 0.03 * Math.sin(back * Math.PI),
				handZ,
			);
		};
		return ({ time, reach, rightIdle }, pose) => {
			const beat = fract(time * rate * 3);
			handAt(beat, 1, pose.right);
			handAt(fract(beat + 0.5), -1, pose.left);
			let highest = Number.NEGATIVE_INFINITY;
			let watchX = 0;
			for (let index = 0; index < cubes.length; index++) {
				const cube = cubes[index];
				const phase = fract(time * rate + index / 3);
				if (phase < 0.4 || (phase >= 0.5 && phase < 0.9)) {
					const leftward = phase < 0.4;
					const flight = leftward ? phase / 0.4 : (phase - 0.5) / 0.4;
					juggled.set(
						leftward ? mix(inner, -outer, flight) : mix(-inner, outer, flight),
						handY + 0.12 + 4 * flight * (1 - flight),
						handZ + (leftward ? 0.05 : -0.03),
					);
				} else {
					juggled.copy(phase < 0.5 ? pose.left : pose.right);
					juggled.y += 0.12;
				}
				held.copy(rightIdle);
				held.y += 0.15;
				held.z += 0.05;
				cube.position.lerpVectors(held, juggled, reach);
				const size =
					index === 0 ? cubeSize : cubeSize * smooth(reach * 1.5 - 0.3);
				cube.visible = size > 0.004;
				cube.scale.setScalar(Math.max(0.001, size));
				cube.rotation.set(time * 2.1 + index, time * 1.6 + index * 2, 0);
				if (juggled.y > highest) {
					highest = juggled.y;
					watchX = juggled.x;
				}
			}
			pose.lookX = clamp(watchX / 0.45, -1, 1);
			pose.lookY = -0.75;
		};
	}

	/** Post throws a paper plane that loops once around it, then catches it again. */
	function planeActivity({ body }: RigParts): ActivityRig {
		const plane = mesh(body, paperPlane, folded, 0, 0, 0, 1.7);
		const trail = Array.from({ length: 4 }, (_, index) =>
			mesh(body, ball, trailPaint, 0, 0, 0, 0.03 - index * 0.005),
		);
		const orbitAt = (lap: number, out: THREE.Vector3) => {
			const angle = lap * TAU;
			return out.set(
				0.95 * Math.cos(angle),
				1.22 + 0.42 * Math.sin(lap * Math.PI),
				0.74 * Math.sin(angle),
			);
		};
		const period = 5.4;
		const rest = new THREE.Vector3(0.84, 1, 0.3);
		const windUp = new THREE.Vector3(0.84, 1.12, -0.02);
		const release = new THREE.Vector3(0.86, 0.94, 0.4);
		const catching = new THREE.Vector3(0.9, 1.14, 0.06);
		const held = new THREE.Vector3();
		const orbit = new THREE.Vector3();
		const holdTurn = quaternion(0.15, -0.6, 0.25, "YXZ");
		const flightTurn = new THREE.Quaternion();
		const heading = new THREE.Euler(0, 0, 0, "YXZ");
		return ({ time, reach, contact, rightIdle }, pose) => {
			const cycle = fract(time / period);
			let flight = 0;
			let lap = 0;
			if (cycle < 0.14) {
				pose.right.lerpVectors(rest, windUp, smooth(cycle / 0.14));
			} else if (cycle < 0.86) {
				lap = (cycle - 0.14) / 0.72;
				flight = smooth(lap / 0.06) * (1 - smooth((lap - 0.94) / 0.06));
				const angle = lap * TAU;
				orbitAt(lap, orbit);
				heading.set(
					0.45,
					Math.atan2(-0.74 * Math.cos(angle), -0.95 * Math.sin(angle)),
					0.14 * Math.cos(lap * Math.PI),
				);
				flightTurn.setFromEuler(heading);
				if (lap < 0.25)
					pose.right.lerpVectors(windUp, release, smooth(lap / 0.12));
				else
					pose.right.lerpVectors(release, catching, smooth((lap - 0.72) / 0.2));
			} else {
				pose.right.lerpVectors(catching, rest, smooth((cycle - 0.86) / 0.14));
			}
			held.lerpVectors(rightIdle, pose.right, reach);
			held.x += 0.05;
			held.y += 0.1;
			held.z += 0.06;
			const airborne = flight * contact;
			plane.position.lerpVectors(held, orbit, airborne);
			plane.quaternion.slerpQuaternions(holdTurn, flightTurn, airborne);
			for (let index = 0; index < trail.length; index++) {
				const behind = lap - (index + 1) * 0.035;
				trail[index].visible = airborne > 0.6 && behind > 0.04;
				if (trail[index].visible) orbitAt(behind, trail[index].position);
			}
			pose.lookX = mix(0.35, clamp(orbit.x / 0.85, -1, 1), airborne);
			pose.lookY = mix(0.1, -0.45, airborne);
		};
	}

	/** Scout scans the horizon through a spyglass and squints with the other eye. */
	function scoutActivity({
		head,
		headOffset,
		rightEye,
	}: RigParts): ActivityRig {
		const scope = new THREE.Group();
		head.add(scope);
		const tube = (
			parent: THREE.Object3D,
			radius: number,
			length: number,
			start: number,
			paint: THREE.Material,
		) => {
			mesh(
				parent,
				cylinder,
				paint,
				start + length / 2,
				0,
				0,
				radius,
				length,
				radius,
			).rotation.z = -Math.PI / 2;
		};
		tube(scope, 0.052, 0.18, -0.02, brassDark);
		tube(scope, 0.062, 0.03, 0.13, silver);
		const middle = new THREE.Group();
		scope.add(middle);
		tube(middle, 0.068, 0.22, 0, brass);
		tube(middle, 0.077, 0.03, 0.2, silver);
		const objective = new THREE.Group();
		scope.add(objective);
		tube(objective, 0.088, 0.24, 0, brass);
		tube(objective, 0.098, 0.035, 0.22, brassDark);
		tube(objective, 0.078, 0.01, 0.25, glass);
		const lookout = rightEye.clone().add(new THREE.Vector3(0.02, 0, 0.08));
		const aimed = quaternion(0, -0.5, 0.2);
		const carried = quaternion(0, -0.3, -1.25);
		const axis = new THREE.Vector3(1, 0, 0).applyQuaternion(aimed);
		const grip = lookout.clone().addScaledVector(axis, 0.3);
		grip.y += headOffset - 0.07;
		const sweep = [0.04, 0.3, 0.14, 0.36];
		const carry = new THREE.Vector3();
		return ({ time, reach, rightIdle }, pose) => {
			const extend = smooth(reach * 1.4 - 0.3);
			middle.position.x = mix(0.02, 0.15, extend);
			objective.position.x = mix(0.04, 0.36, extend);
			carry.copy(rightIdle);
			carry.y -= headOffset;
			carry.z += 0.04;
			scope.position.lerpVectors(carry, lookout, reach);
			scope.quaternion.slerpQuaternions(carried, aimed, reach);
			const beat = time / 1.7;
			const step = Math.floor(beat);
			pose.yaw = mix(
				sweep[(step + sweep.length - 1) % sweep.length],
				sweep[step % sweep.length],
				smooth(fract(beat) / 0.4),
			);
			pose.right.copy(grip);
			pose.left.set(-0.82, 0.58 + Math.sin(time * 1.6) * 0.015, 0.2);
			pose.lookX = 0.95;
			pose.lookY = -0.2;
			pose.squint = 1;
		};
	}

	/** Loop reads an open book, eyes running along the lines, and turns a page now and then. */
	function readActivity({ body, palette }: RigParts): ActivityRig {
		const book = new THREE.Group();
		book.scale.setScalar(1.3);
		body.add(book);
		const cover = paint(palette.accent, { roughness: 0.55 });
		const half = (side: number) => {
			const group = new THREE.Group();
			book.add(group);
			mesh(group, rounded, cover, side * 0.165, -0.012, 0, 0.33, 0.022, 0.42);
			mesh(group, rounded, paper, side * 0.155, 0.008, 0, 0.3, 0.022, 0.38);
			for (let line = 0; line < 4; line++)
				mesh(
					group,
					rounded,
					graphite,
					side * (line === 3 ? 0.12 : 0.155),
					0.021,
					-0.12 + line * 0.075,
					line === 3 ? 0.14 : 0.21,
					0.004,
					0.016,
				);
			return group;
		};
		const left = half(-1);
		const right = half(1);
		const leaf = new THREE.Group();
		book.add(leaf);
		mesh(leaf, rounded, paper, 0.152, 0.026, 0, 0.29, 0.008, 0.37);
		const carried = quaternion(0, Math.PI / 2, -0.15);
		const reading = quaternion(0.62, 0, 0);
		const lap = new THREE.Vector3(0, 0.56, 1.12);
		const carry = new THREE.Vector3();
		return ({ time, reach, rightIdle }, pose) => {
			const spread = mix(Math.PI / 2 - 0.03, 0.2, smooth(reach * 1.3 - 0.2));
			right.rotation.z = spread;
			left.rotation.z = -spread;
			const turn = smooth((fract(time / 3.8) - 0.55) / 0.3);
			leaf.visible = reach > 0.9 && turn > 0.001 && turn < 0.999;
			leaf.rotation.z = mix(spread, Math.PI - spread, turn);
			carry.copy(rightIdle);
			carry.y += 0.06;
			carry.z += 0.08;
			book.position.lerpVectors(carry, lap, reach);
			book.quaternion.slerpQuaternions(carried, reading, reach);
			pose.left.set(-0.5, 0.52, 1.12);
			pose.right.set(0.5, 0.52, 1.12);
			const line = fract(time * 0.42);
			pose.lookX =
				line < 0.85
					? mix(-0.45, 0.45, line / 0.85)
					: mix(0.45, -0.45, (line - 0.85) / 0.15);
			pose.lookY = 0.65;
		};
	}

	const activities: Readonly<
		Record<BotActivity, (parts: RigParts) => ActivityRig>
	> = {
		build: buildActivity,
		juggle: juggleActivity,
		plane: planeActivity,
		scout: scoutActivity,
		read: readActivity,
	};

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

		const feet = [-1, 1].map((side) => {
			const foot = new THREE.Group();
			foot.position.set(side * 0.27, 0.2, 0.04);
			mesh(foot, ball, footPaint, 0, -0.08, 0.05, 0.16, 0.1, 0.21);
			body.add(foot);
			return foot;
		}) as [THREE.Group, THREE.Group];

		group.updateMatrixWorld(true);
		const rightEye = head.worldToLocal(
			(eyes[1] ?? eyes[0] ?? head).getWorldPosition(new THREE.Vector3()),
		);
		const activity = activities[frame.activity]({
			id: frame.id,
			body,
			head,
			headOffset: bodyFrameInfo.middle,
			rightEye,
			palette,
		});
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
			shadow,
			activity,
			previousWalk: frame.walk,
			previousTime: frame.time,
			walking: 0,
			kind: frame.kind,
			activityKind: frame.activity,
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
		const walking = rig.walking * (1 - reach * 0.95);
		const stride = Math.sin(frame.walk);
		const bob = Math.abs(stride) * 0.05 * walking;
		const squash = Math.abs(Math.cos(frame.walk)) * 0.035 * walking;
		const lean = clamp(frame.lean, -1, 1);
		const tilt = lean * 0.1 * (1 - reach * 0.7);
		const swing = Math.sin(frame.time * 1.6) * 0.015;
		idleLeft.set(
			-0.84,
			0.72 + Math.max(0, stride) * 0.12 * walking + swing,
			0.18 + stride * 0.12 * walking,
		);
		idleRight.set(
			0.84,
			0.72 + Math.max(0, -stride) * 0.12 * walking - swing,
			0.18 - stride * 0.12 * walking,
		);
		pose.left.copy(idleLeft);
		pose.right.copy(idleRight);
		pose.lookX = 0;
		pose.lookY = 0;
		pose.yaw = 0;
		pose.squint = 0;
		rig.activity(
			{ time: frame.time, reach, contact, rightIdle: idleRight },
			pose,
		);
		rig.leftHand.position.lerpVectors(idleLeft, pose.left, reach);
		rig.rightHand.position.lerpVectors(idleRight, pose.right, reach);
		const focus = contact * 0.85;
		const lookX = mix(clamp(frame.lookX, -1, 1), pose.lookX, focus);
		const lookY = mix(clamp(frame.lookY, -1, 1), pose.lookY, focus);
		rig.body.position.y = bob;
		rig.body.rotation.z = tilt;
		rig.body.rotation.y = lean * walking * 0.38 + pose.yaw * contact;
		rig.head.scale.set(1 + squash, 1 - squash, 1 + squash * 0.5);
		rig.head.rotation.y = lookX * 0.26;
		rig.head.rotation.x = lookY * 0.12;
		rig.head.rotation.z = Math.sin(frame.time * 1.3) * 0.025 * (1 - work);
		for (const pupil of rig.pupils) {
			pupil.position.x = lookX * 0.03;
			pupil.position.y = -lookY * 0.02;
		}
		const blink = (frame.time + frame.id.length * 0.47) % 5.3;
		for (const eye of rig.eyes) eye.scale.y = blink < 0.11 ? 0.15 : 1;
		if (rig.eyes[0]) rig.eyes[0].scale.y *= mix(1, 0.25, pose.squint * contact);
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
		rig.shadow.scale.x = 1.75 - bob;
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
				if (
					!rig ||
					rig.kind !== frame.kind ||
					rig.activityKind !== frame.activity
				) {
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
