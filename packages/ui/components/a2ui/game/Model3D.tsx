"use client";

import {
	ContactShadows,
	Environment,
	OrbitControls,
	PerspectiveCamera,
	useGLTF,
} from "@react-three/drei";
import { Canvas, useFrame, useThree } from "@react-three/fiber";
import { Suspense, useEffect, useMemo, useRef } from "react";
import * as THREE from "three";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { useAssetUrl } from "../hooks/use-asset-url";
import type { BoundValue, Model3DComponent } from "../types";
import { Scene3DProvider, useIsInsideScene3D } from "./Scene3DContext";
import { setModel3DView } from "./model3d-view-registry";

type CameraAngle = "front" | "side" | "top" | "isometric";
type LightingPreset = "neutral" | "warm" | "cool" | "studio" | "dramatic";
type EnvironmentPreset =
	| "studio"
	| "sunset"
	| "dawn"
	| "night"
	| "warehouse"
	| "forest"
	| "apartment"
	| "city"
	| "park"
	| "lobby";
type EnvironmentSource = "local" | "preset" | "polyhaven" | "custom";
type PolyhavenResolution = "1k" | "2k" | "4k" | "8k";

const CAMERA_ANGLES: Record<CameraAngle, [number, number, number]> = {
	front: [0, 0, 1],
	side: [1, 0, 0],
	top: [0, 1, 0],
	isometric: [1, 1, 1],
};

const LIGHTING_PRESETS: Record<
	LightingPreset,
	{
		ambient: number;
		main: number;
		fill: number;
		rim: number;
		mainColor: string;
		fillColor: string;
	}
> = {
	neutral: {
		ambient: 0.5,
		main: 1.0,
		fill: 0.3,
		rim: 0.2,
		mainColor: "#ffffff",
		fillColor: "#ffffff",
	},
	warm: {
		ambient: 0.4,
		main: 1.0,
		fill: 0.4,
		rim: 0.3,
		mainColor: "#fff5e6",
		fillColor: "#ffe4c4",
	},
	cool: {
		ambient: 0.4,
		main: 1.0,
		fill: 0.4,
		rim: 0.3,
		mainColor: "#e6f3ff",
		fillColor: "#cce5ff",
	},
	studio: {
		ambient: 0.6,
		main: 1.2,
		fill: 0.5,
		rim: 0.4,
		mainColor: "#ffffff",
		fillColor: "#f0f0ff",
	},
	dramatic: {
		ambient: 0.2,
		main: 1.5,
		fill: 0.2,
		rim: 0.6,
		mainColor: "#fff8e7",
		fillColor: "#4a4a6a",
	},
};

const POLYHAVEN_HDRI_IDS = [
	"studio_small_03",
	"studio_small_09",
	"brown_photostudio_02",
	"empty_warehouse_01",
	"industrial_sunset_02",
	"sunset_in_the_chalk_quarry",
	"rooftop_night",
	"abandoned_factory_canteen_01",
	"forest_slope",
	"green_point_park",
	"lebombo",
	"spruit_sunrise",
	"syferfontein_18d_clear_puresky",
	"venice_sunset",
	"potsdamer_platz",
] as const;

function getPolyhavenHdriUrl(
	id: string,
	resolution: PolyhavenResolution,
): string {
	return `https://dl.polyhaven.org/file/ph-assets/HDRIs/hdr/${resolution}/${id}_${resolution}.hdr`;
}

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

// R3F placeholder - only use inside Canvas context
function R3FPlaceholder({
	position,
	color = "#666",
	size = 1,
}: {
	position: [number, number, number];
	color?: string;
	size?: number;
}) {
	return (
		<group position={position}>
			<mesh>
				<boxGeometry args={[size, size, size]} />
				<meshStandardMaterial color={color} wireframe />
			</mesh>
		</group>
	);
}

function GroundPlane({
	size,
	offsetY,
	followCamera,
	color,
}: {
	size: number;
	offsetY: number;
	followCamera: boolean;
	color: string;
}) {
	const meshRef = useRef<THREE.Mesh>(null);
	const { camera } = useThree();

	useFrame(() => {
		if (!meshRef.current) return;
		if (followCamera) {
			meshRef.current.position.x = camera.position.x;
			meshRef.current.position.z = camera.position.z;
		}
		meshRef.current.position.y = offsetY;
	});

	return (
		<mesh ref={meshRef} rotation={[-Math.PI / 2, 0, 0]} receiveShadow>
			<planeGeometry args={[size, size]} />
			<meshStandardMaterial color={color} />
		</mesh>
	);
}

function ViewTracker({
	componentId,
	controlsRef,
	defaultTarget,
}: {
	componentId: string;
	controlsRef: React.MutableRefObject<any>;
	defaultTarget: [number, number, number];
}) {
	const { camera } = useThree();

	useFrame(() => {
		const target = controlsRef.current?.target
			? ([
					controlsRef.current.target.x,
					controlsRef.current.target.y,
					controlsRef.current.target.z,
				] as [number, number, number])
			: defaultTarget;
		setModel3DView(componentId, {
			cameraPosition: [camera.position.x, camera.position.y, camera.position.z],
			cameraTarget: target,
		});
	});

	return null;
}

// Inner component that uses R3F hooks - only render inside Canvas
function Model3DInner({ component }: { component: Model3DComponent }) {
	const src = useResolved<string>(component.src);
	const position = useResolved<[number, number, number]>(
		component.position,
	) ?? [0, 0, 0];
	const rotation = useResolved<[number, number, number]>(
		component.rotation,
	) ?? [0, 0, 0];
	const scaleValue =
		useResolved<number | [number, number, number]>(component.scale) ?? 1;
	const castShadow = useResolved<boolean>(component.castShadow) ?? true;
	const receiveShadow = useResolved<boolean>(component.receiveShadow) ?? true;
	const animationName = useResolved<string>(component.animation);
	const autoRotate = useResolved<boolean>(component.autoRotate) ?? false;
	const rotateSpeed = useResolved<number>(component.rotateSpeed) ?? 1;

	const scale = useMemo(() => {
		if (typeof scaleValue === "number") {
			return [scaleValue, scaleValue, scaleValue] as [number, number, number];
		}
		return scaleValue;
	}, [scaleValue]);

	if (!src) {
		return <R3FPlaceholder position={position} color="#ff6b6b" />;
	}

	return (
		<ModelWithAssetResolution
			rawSrc={src}
			position={position}
			rotation={rotation}
			scale={scale}
			castShadow={castShadow}
			receiveShadow={receiveShadow}
			animationName={animationName}
			autoRotate={autoRotate}
			rotateSpeed={rotateSpeed}
		/>
	);
}

// Standalone canvas wrapper for when Model3D is used outside Scene3D
function StandaloneModel3D({
	component,
	componentId,
}: {
	component: Model3DComponent;
	componentId: string;
}) {
	// Viewer options
	const viewerHeight = useResolved<string>(component.viewerHeight) ?? "100%";
	const backgroundColor =
		useResolved<string>(component.backgroundColor) ?? "transparent";
	const cameraDistance = useResolved<number>(component.cameraDistance) ?? 3;
	const fov = useResolved<number>(component.fov) ?? 50;
	const cameraAngle =
		useResolved<CameraAngle>(component.cameraAngle) ?? "front";
	const cameraPositionOverride = useResolved<[number, number, number]>(
		component.cameraPosition,
	);
	const cameraTargetOverride = useResolved<[number, number, number]>(
		component.cameraTarget,
	);

	// Control options
	const enableControls = useResolved<boolean>(component.enableControls) ?? true;
	const enableZoom = useResolved<boolean>(component.enableZoom) ?? true;
	const enablePan = useResolved<boolean>(component.enablePan) ?? false;
	const autoRotateCamera =
		useResolved<boolean>(component.autoRotateCamera) ?? false;
	const cameraRotateSpeed =
		useResolved<number>(component.cameraRotateSpeed) ?? 2;
	const controlsRef = useRef<any>(null);

	// Lighting options
	const lightingPreset =
		useResolved<LightingPreset>(component.lightingPreset) ?? "studio";
	const ambientLightOverride = useResolved<number>(component.ambientLight);
	const directionalLightOverride = useResolved<number>(
		component.directionalLight,
	);
	const fillLightOverride = useResolved<number>(component.fillLight);
	const rimLightOverride = useResolved<number>(component.rimLight);
	const lightColor = useResolved<string>(component.lightColor);

	// Environment options
	const showGround = useResolved<boolean>(component.showGround) ?? false;
	const groundColor = useResolved<string>(component.groundColor) ?? "#1a1a2e";
	const groundSize = useResolved<number>(component.groundSize) ?? 200;
	const groundOffsetY = useResolved<number>(component.groundOffsetY) ?? -0.5;
	const groundFollowCamera =
		useResolved<boolean>(component.groundFollowCamera) ?? true;
	const enableReflections =
		useResolved<boolean>(component.enableReflections) ?? true;
	const environment =
		useResolved<EnvironmentPreset>(component.environment) ?? "studio";
	const environmentSource =
		useResolved<EnvironmentSource>(component.environmentSource) ?? "local";
	const useHdrBackground =
		useResolved<boolean>(component.useHdrBackground) ?? false;
	const polyhavenHdri =
		useResolved<string>(component.polyhavenHdri) ?? "studio_small_03";
	const polyhavenResolution =
		useResolved<PolyhavenResolution>(component.polyhavenResolution) ?? "1k";
	const hdriPath = useResolved<string>(component.hdriUrl);
	const { url: resolvedHdriUrl } = useAssetUrl(hdriPath, { localFiles: true });

	// Calculate camera position from angle and distance
	const computedCameraPosition = useMemo(() => {
		const angleVector = CAMERA_ANGLES[cameraAngle] || CAMERA_ANGLES.front;
		const normalized = new THREE.Vector3(...angleVector).normalize();
		return [
			normalized.x * cameraDistance,
			normalized.y * cameraDistance,
			normalized.z * cameraDistance,
		] as [number, number, number];
	}, [cameraAngle, cameraDistance]);
	const cameraPosition = cameraPositionOverride ?? computedCameraPosition;
	const cameraTarget =
		cameraTargetOverride ?? ([0, 0, 0] as [number, number, number]);

	// Get lighting values from preset, with overrides
	const lighting = LIGHTING_PRESETS[lightingPreset] || LIGHTING_PRESETS.studio;
	const ambientIntensity = ambientLightOverride ?? lighting.ambient;
	const mainIntensity = directionalLightOverride ?? lighting.main;
	const fillIntensity = fillLightOverride ?? lighting.fill;
	const rimIntensity = rimLightOverride ?? lighting.rim;
	const mainLightColor = lightColor ?? lighting.mainColor;
	const fillLightColor = lightColor ?? lighting.fillColor;
	const polyhavenId = POLYHAVEN_HDRI_IDS.includes(
		polyhavenHdri as (typeof POLYHAVEN_HDRI_IDS)[number],
	)
		? polyhavenHdri
		: "studio_small_03";
	const polyhavenUrl = getPolyhavenHdriUrl(polyhavenId, polyhavenResolution);
	const localHdriPath = `/hdri/${polyhavenId}_1k.hdr`;
	const hdriUrl =
		environmentSource === "polyhaven"
			? polyhavenUrl
			: environmentSource === "custom"
				? (resolvedHdriUrl ?? hdriPath)
				: environmentSource === "local"
					? localHdriPath
					: undefined;

	const heightStyle =
		viewerHeight && viewerHeight !== "auto" && viewerHeight !== "parent"
			? viewerHeight
			: undefined;

	return (
		<div
			className="w-full h-full rounded-lg overflow-hidden"
			style={{
				height: heightStyle,
				backgroundColor:
					backgroundColor === "transparent" ? undefined : backgroundColor,
			}}
		>
			<Canvas
				shadows
				gl={{ antialias: true, alpha: backgroundColor === "transparent" }}
			>
				<Scene3DProvider>
					<Suspense
						fallback={<R3FPlaceholder position={[0, 0, 0]} color="#666" />}
					>
						<PerspectiveCamera
							makeDefault
							position={cameraPosition}
							fov={fov}
						/>
						<ViewTracker
							componentId={componentId}
							controlsRef={controlsRef}
							defaultTarget={cameraTarget}
						/>
						{/* Lighting setup */}
						<ambientLight intensity={ambientIntensity} color={mainLightColor} />
						<directionalLight
							position={[5, 5, 5]}
							intensity={mainIntensity}
							color={mainLightColor}
							castShadow
							shadow-mapSize-width={1024}
							shadow-mapSize-height={1024}
						/>
						<directionalLight
							position={[-3, 2, -3]}
							intensity={fillIntensity}
							color={fillLightColor}
						/>
						<directionalLight
							position={[0, 3, -5]}
							intensity={rimIntensity}
							color="#ffffff"
						/>

						{/* Environment for reflections/background */}
						{enableReflections && environmentSource === "preset" && (
							<Environment
								key={`preset:${environment}:${useHdrBackground}`}
								preset={environment}
								background={useHdrBackground}
							/>
						)}
						{enableReflections && environmentSource !== "preset" && hdriUrl && (
							<Environment
								key={`hdri:${environmentSource}:${hdriUrl}:${useHdrBackground}`}
								files={hdriUrl}
								background={useHdrBackground}
							/>
						)}

						{/* Ground plane with contact shadows */}
						{showGround && (
							<>
								<GroundPlane
									size={groundSize}
									offsetY={groundOffsetY}
									followCamera={groundFollowCamera}
									color={groundColor}
								/>
								<ContactShadows
									position={[0, groundOffsetY + 0.01, 0]}
									opacity={0.4}
									scale={groundSize}
									blur={2}
									far={groundSize / 2}
								/>
							</>
						)}

						{/* The actual model */}
						<Model3DInner component={component} />

						{/* Controls */}
						{enableControls && (
							<OrbitControls
								ref={controlsRef}
								target={cameraTarget}
								enablePan={enablePan}
								enableZoom={enableZoom}
								autoRotate={autoRotateCamera}
								autoRotateSpeed={cameraRotateSpeed}
								minDistance={1}
								maxDistance={20}
							/>
						)}
					</Suspense>
				</Scene3DProvider>
			</Canvas>
		</div>
	);
}

export function A2UIModel3D({
	component,
	componentId,
}: ComponentProps<Model3DComponent>) {
	const isInsideScene3D = useIsInsideScene3D();

	// If not inside Scene3D's Canvas context, create a standalone canvas
	if (!isInsideScene3D) {
		return (
			<StandaloneModel3D component={component} componentId={componentId} />
		);
	}

	// Inside Canvas - render R3F component directly
	return <Model3DInner component={component} />;
}

interface ModelWithAssetResolutionProps {
	rawSrc: string;
	position: [number, number, number];
	rotation: [number, number, number];
	scale: [number, number, number];
	castShadow: boolean;
	receiveShadow: boolean;
	animationName?: string;
	autoRotate: boolean;
	rotateSpeed: number;
}

function ModelWithAssetResolution(props: ModelWithAssetResolutionProps) {
	const { url: resolvedSrc, isLoading } = useAssetUrl(props.rawSrc, {
		localFiles: true,
	});

	if (isLoading) {
		return <R3FPlaceholder position={props.position} color="#666" />;
	}

	if (!resolvedSrc) {
		return <R3FPlaceholder position={props.position} color="#ff6b6b" />;
	}

	return (
		<GLTFModelLoader
			src={resolvedSrc}
			position={props.position}
			rotation={props.rotation}
			scale={props.scale}
			castShadow={props.castShadow}
			receiveShadow={props.receiveShadow}
			animationName={props.animationName}
			autoRotate={props.autoRotate}
			rotateSpeed={props.rotateSpeed}
		/>
	);
}

interface GLTFModelLoaderProps {
	src: string;
	position: [number, number, number];
	rotation: [number, number, number];
	scale: [number, number, number];
	castShadow: boolean;
	receiveShadow: boolean;
	animationName?: string;
	autoRotate: boolean;
	rotateSpeed: number;
}

function GLTFModelLoader(props: GLTFModelLoaderProps) {
	const groupRef = useRef<THREE.Group>(null);
	const mixerRef = useRef<THREE.AnimationMixer | null>(null);

	const gltf = useGLTF(props.src);
	const { scene, animations } = gltf;

	const clonedScene = useMemo(() => {
		if (!scene) return null;
		const clone = scene.clone(true);
		clone.traverse((child) => {
			if ((child as THREE.Mesh).isMesh) {
				const mesh = child as THREE.Mesh;
				mesh.castShadow = props.castShadow;
				mesh.receiveShadow = props.receiveShadow;
			}
		});
		return clone;
	}, [scene, props.castShadow, props.receiveShadow]);

	useEffect(() => {
		if (animations && animations.length > 0 && clonedScene) {
			const mixer = new THREE.AnimationMixer(clonedScene);
			mixerRef.current = mixer;

			const clipToPlay = props.animationName
				? animations.find((clip) => clip.name === props.animationName)
				: animations[0];

			if (clipToPlay) {
				const action = mixer.clipAction(clipToPlay);
				action.play();
			}

			return () => {
				mixer.stopAllAction();
				mixerRef.current = null;
			};
		}
	}, [animations, clonedScene, props.animationName]);

	useFrame((_, delta) => {
		if (mixerRef.current) {
			mixerRef.current.update(delta);
		}

		if (props.autoRotate && groupRef.current) {
			groupRef.current.rotation.y += delta * props.rotateSpeed;
		}
	});

	if (!clonedScene) {
		return <R3FPlaceholder position={props.position} color="#ff6b6b" />;
	}

	return (
		<group
			ref={groupRef}
			position={props.position}
			rotation={props.rotation}
			scale={props.scale}
		>
			<primitive object={clonedScene} />
		</group>
	);
}
