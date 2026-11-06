import * as THREE from "three";
import { RoundedBoxGeometry } from "three/addons/geometries/RoundedBoxGeometry.js";

export interface BotFrame {
	id: string;
	x: number;
	y: number;
	width: number;
	height: number;
	color: string;
	tool: "wrench" | "cable" | "driver" | "inspect" | "pencil";
	walk: number;
	work: number;
	reach: number;
	lookX: number;
	lookY: number;
	lean: number;
	time: number;
}

interface Arm {
	side: number;
	upper: THREE.Mesh;
	lower: THREE.Mesh;
	elbow: THREE.Mesh;
	hand: THREE.Group;
}

interface BotRig {
	group: THREE.Group;
	body: THREE.Group;
	head: THREE.Group;
	eyes: THREE.Group;
	antenna: THREE.Group;
	leftArm: Arm;
	rightArm: Arm;
	legs: [THREE.Group, THREE.Group];
	tool: THREE.Group;
	bolt: THREE.Group;
	sparks: THREE.Mesh[];
	shadow: THREE.Mesh;
	previousWalk: number;
	previousTime: number;
	walking: number;
	color: string;
	kind: BotFrame["tool"];
}

const clamp = (value: number, min = 0, max = 1) =>
	Math.min(max, Math.max(min, value));

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
	renderer.toneMapping = THREE.ACESFilmicToneMapping;
	renderer.toneMappingExposure = 1.18;

	const scene = new THREE.Scene();
	scene.add(new THREE.HemisphereLight(0xfff5e5, 0x393646, 2.1));
	const key = new THREE.DirectionalLight(0xfff0dc, 3.2);
	key.position.set(-3, 5, 6);
	scene.add(key);
	const rim = new THREE.DirectionalLight(0xe0eeff, 2.1);
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
	const up = new THREE.Vector3(0, 1, 0);
	const direction = new THREE.Vector3();
	const start = new THREE.Vector3();
	const middle = new THREE.Vector3();
	const end = new THREE.Vector3();
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
	const ball = geometry("ball", () => new THREE.SphereGeometry(1, 12, 8));
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
	const eyeMaterial = material("eyes", {
		color: 0xc7faff,
		emissive: 0x7adcdf,
		emissiveIntensity: 1.1,
		roughness: 0.2,
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

	function arm(parent: THREE.Group, side: number, paint: THREE.Material): Arm {
		mesh(parent, ball, silver, side * 0.49, 1.2, 0, 0.13);
		const upper = mesh(parent, cylinder, paint, 0, 0, 0, 0.105, 0.4, 0.105);
		const lower = mesh(parent, cylinder, charcoal, 0, 0, 0, 0.085, 0.4, 0.085);
		const elbow = mesh(parent, ball, silver, 0, 0, 0, 0.11);
		const hand = new THREE.Group();
		mesh(hand, rounded, paint, 0, 0, 0, 0.18, 0.19, 0.18);
		mesh(hand, rounded, charcoal, -0.055, -0.06, 0.085, 0.055, 0.11, 0.06);
		mesh(hand, rounded, charcoal, 0.03, -0.06, 0.085, 0.055, 0.11, 0.06);
		parent.add(hand);
		return { side, upper, lower, elbow, hand };
	}

	function toolFor(kind: BotFrame["tool"], paint: THREE.Material) {
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
			mesh(tool, rounded, paint, 0, -0.035, 0.11, 0.12, 0.2, 0.12);
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
			mesh(tool, curve, paint, 0, 0, 0);
			mesh(tool, rounded, charcoal, 0, -0.23, 0.11, 0.15, 0.15, 0.12);
			mesh(tool, rounded, silver, 0, -0.34, 0.11, 0.08, 0.09, 0.06);
			mesh(tool, rounded, warmLight, 0, -0.22, 0.18, 0.07, 0.025, 0.01);
		} else if (kind === "inspect") {
			mesh(tool, cylinder, paint, 0, -0.1, 0.12, 0.04, 0.22, 0.04);
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
			mesh(tool, ball, paint, 0, 0.075, 0.12, 0.037, 0.04, 0.037);
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

	function createRig(frame: BotFrame): BotRig {
		const group = new THREE.Group();
		const body = new THREE.Group();
		group.add(body);
		const color = new THREE.Color(frame.color);
		const paint = material(`paint-${frame.color}`, {
			color,
			roughness: 0.25,
			metalness: 0.16,
		});
		const trim = material(`trim-${frame.color}`, {
			color: color.clone().lerp(new THREE.Color(0xffffff), 0.5),
			roughness: 0.32,
			metalness: 0.15,
		});
		mesh(body, rounded, paint, 0, 0.98, 0, 0.88, 0.81, 0.59);
		mesh(body, rounded, trim, 0, 1.01, 0.31, 0.52, 0.43, 0.035);
		mesh(body, rounded, charcoal, 0, 0.69, 0, 0.83, 0.12, 0.61);
		mesh(body, rounded, silver, 0, 0.69, 0.33, 0.19, 0.13, 0.04);
		mesh(body, rounded, paint, 0, 1.03, 0.345, 0.25, 0.13, 0.03).rotation.z =
			-0.18;
		mesh(body, ball, warmLight, -0.25, 1.23, 0.3, 0.037);
		const head = new THREE.Group();
		head.position.set(0, 1.67, 0);
		body.add(head);
		mesh(head, rounded, paint, 0, 0, 0, 1.08, 0.68, 0.73);
		mesh(head, rounded, trim, 0, 0.04, 0.36, 0.98, 0.47, 0.08);
		mesh(head, rounded, visor, 0, 0.03, 0.41, 0.88, 0.37, 0.075);
		const eyes = new THREE.Group();
		eyes.position.set(0, 0.045, 0.46);
		head.add(eyes);
		mesh(eyes, rounded, eyeMaterial, -0.19, 0.025, 0, 0.09, 0.145, 0.025);
		mesh(eyes, rounded, eyeMaterial, 0.19, 0.025, 0, 0.09, 0.145, 0.025);
		mesh(
			head,
			ring(0.075, 0.012, Math.PI),
			eyeMaterial,
			0,
			-0.025,
			0.46,
		).rotation.z = Math.PI;
		mesh(
			head,
			rounded,
			reflection,
			-0.17,
			0.15,
			0.458,
			0.3,
			0.025,
			0.008,
		).rotation.z = -0.13;
		mesh(head, cylinder, charcoal, -0.55, 0, 0, 0.15, 0.075, 0.15).rotation.z =
			Math.PI / 2;
		mesh(head, cylinder, charcoal, 0.55, 0, 0, 0.15, 0.075, 0.15).rotation.z =
			Math.PI / 2;
		mesh(head, cylinder, silver, 0.59, 0, 0, 0.075, 0.025, 0.075).rotation.z =
			Math.PI / 2;
		const antenna = new THREE.Group();
		antenna.position.set(0.23, 0.34, 0);
		head.add(antenna);
		mesh(antenna, cylinder, silver, 0, 0.09, 0, 0.023, 0.2, 0.023);
		mesh(antenna, ball, trim, 0, 0.21, 0, 0.07);
		mesh(antenna, ball, warmLight, 0, 0.21, 0.04, 0.035);
		const legs = [-1, 1].map((side) => {
			const leg = new THREE.Group();
			leg.position.set(side * 0.24, 0.61, 0);
			mesh(leg, ball, silver, 0, -0.01, 0, 0.1);
			mesh(leg, cylinder, charcoal, 0, -0.23, 0, 0.09, 0.39, 0.09);
			mesh(leg, rounded, trim, 0, -0.28, 0.02, 0.2, 0.23, 0.21);
			mesh(leg, rounded, rubber, 0, -0.51, 0.095, 0.32, 0.2, 0.45);
			mesh(leg, rounded, paint, 0, -0.49, 0.21, 0.3, 0.12, 0.2);
			body.add(leg);
			return leg;
		}) as [THREE.Group, THREE.Group];
		const leftArm = arm(body, -1, paint);
		const rightArm = arm(body, 1, paint);
		const tool = toolFor(frame.tool, paint);
		rightArm.hand.add(tool);

		const bolt = new THREE.Group();
		bolt.position.set(0.79, 0.018, 0.18);
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
			1.65,
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
			antenna,
			leftArm,
			rightArm,
			legs,
			tool,
			bolt,
			sparks,
			shadow,
			previousWalk: frame.walk,
			previousTime: frame.time,
			walking: 0,
			color: frame.color,
			kind: frame.tool,
		};
	}

	function segment(object: THREE.Mesh, from: THREE.Vector3, to: THREE.Vector3) {
		direction.subVectors(to, from);
		object.position.copy(from).addScaledVector(direction, 0.5);
		object.scale.y = direction.length();
		object.quaternion.setFromUnitVectors(up, direction.normalize());
	}
	function placeArm(arm: Arm, x: number, y: number, z: number) {
		start.set(arm.side * 0.49, 1.2, 0);
		end.set(x, y, z);
		const dx = x - start.x;
		const dy = y - start.y;
		const distance = Math.max(0.01, Math.hypot(dx, dy));
		const length = 0.44;
		const bend = Math.sqrt(Math.max(0, length ** 2 - (distance / 2) ** 2));
		middle.set(
			(start.x + x) / 2 - (dy / distance) * bend * arm.side,
			(start.y + y) / 2 + (dx / distance) * bend * arm.side,
			z * 0.55,
		);
		segment(arm.upper, start, middle);
		segment(arm.lower, middle, end);
		arm.elbow.position.copy(middle);
		arm.hand.position.copy(end);
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
		const bob = Math.abs(stride) * 0.035 * walking * (1 - contact);
		const tilt = clamp(frame.lean, -1, 1) * 0.1;
		rig.body.position.y = bob;
		rig.body.rotation.z = tilt;
		rig.body.rotation.y =
			clamp(frame.lean, -1, 1) * walking * (1 - reach) * 0.38;
		rig.head.rotation.y = clamp(frame.lookX, -1, 1) * 0.22;
		rig.head.rotation.x = clamp(frame.lookY, -1, 1) * 0.14 + reach * 0.11;
		rig.head.rotation.z = Math.sin(frame.time * 1.3) * 0.018 * (1 - work);
		rig.eyes.position.x = clamp(frame.lookX, -1, 1) * 0.035;
		rig.eyes.position.y = 0.045 - clamp(frame.lookY, -1, 1) * 0.025;
		const blink = (frame.time + frame.id.length * 0.47) % 5.3;
		rig.eyes.scale.y = blink < 0.11 ? 0.2 : 1;
		rig.antenna.rotation.z =
			-stride * 0.09 * walking + Math.sin(frame.time * 2) * 0.035;
		rig.legs[0].rotation.x = stride * 0.18 * walking;
		rig.legs[1].rotation.x = -stride * 0.18 * walking;
		rig.legs[0].rotation.z = stride * 0.24 * walking;
		rig.legs[1].rotation.z = -stride * 0.24 * walking;
		rig.legs[0].position.y = 0.61 + Math.max(0, stride) * 0.045 * walking;
		rig.legs[1].position.y = 0.61 + Math.max(0, -stride) * 0.045 * walking;
		placeArm(
			rig.leftArm,
			-0.62 - reach * 0.07,
			0.62 + Math.max(0, stride) * 0.15 * walking,
			0.02 + stride * 0.14 * walking,
		);
		rig.leftArm.hand.rotation.z = -0.12 - reach * 0.2;

		const writing =
			frame.tool === "pencil" ? Math.sin(frame.time * 6) * 0.035 * contact : 0;
		const handX = 0.65 + reach * 0.14 + writing;
		const handY =
			0.69 - reach * 0.28 + Math.sin(frame.time * 8) * 0.012 * contact;
		// Inverse body lean keeps the working tip on the page surface.
		const localX = handX * Math.cos(tilt) + (handY - bob) * Math.sin(tilt);
		const localY = -handX * Math.sin(tilt) + (handY - bob) * Math.cos(tilt);
		placeArm(rig.rightArm, localX, localY, 0.07);
		rig.rightArm.hand.rotation.z = -tilt + (1 - reach) * -0.25;
		rig.tool.rotation.y =
			frame.tool === "driver" || frame.tool === "wrench"
				? Math.sin(frame.time * 7) * 0.32 * contact
				: 0;
		rig.bolt.rotation.y =
			frame.tool === "wrench" || frame.tool === "driver"
				? Math.sin(frame.time * 7) * 0.08 * contact
				: 0;
		rig.shadow.scale.x = 1.65 - bob;
		const canSpark =
			contact > 0.65 && ["wrench", "driver", "cable"].includes(frame.tool);
		for (let index = 0; index < rig.sparks.length; index++) {
			const particle = rig.sparks[index];
			const progress = (frame.time * 2.8 + index * 0.19) % 1;
			particle.visible = canSpark && progress < 0.52;
			if (!particle.visible) continue;
			const angle = index * 1.7 + 0.4;
			particle.position.set(
				0.79 + Math.cos(angle) * progress * 0.28,
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
				if (!rig || rig.color !== frame.color || rig.kind !== frame.tool) {
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
