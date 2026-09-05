"use client";

import { OrbitControls, PerspectiveCamera } from "@react-three/drei";
import { Canvas } from "@react-three/fiber";
import { Fragment, Suspense, useMemo, useRef } from "react";
import { cn } from "../../../lib/utils";
import type { ComponentProps, RenderChildFn } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, Children, Scene3DComponent } from "../types";
import { Scene3DProvider } from "./Scene3DContext";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

function getChildIds(children: Children | undefined): string[] {
	if (!children) return [];
	if ("explicitList" in children) return children.explicitList;
	return [];
}

export type ControlMode = "orbit" | "fly" | "fixed" | "auto-rotate";
export type FixedView =
	| "front"
	| "back"
	| "left"
	| "right"
	| "top"
	| "bottom"
	| "isometric";

const FIXED_VIEW_POSITIONS: Record<FixedView, [number, number, number]> = {
	front: [0, 0, 5],
	back: [0, 0, -5],
	left: [-5, 0, 0],
	right: [5, 0, 0],
	top: [0, 5, 0],
	bottom: [0, -5, 0],
	isometric: [3.5, 3.5, 3.5],
};

export function A2UIScene3D({
	elementRef,
	component,
	style,
	renderChild,
}: ComponentProps<Scene3DComponent> & { renderChild: RenderChildFn }) {
	const width = useResolved<string>(component.width) ?? "100%";
	const height = useResolved<string>(component.height) ?? "400px";
	const backgroundColor =
		useResolved<string>(component.backgroundColor) ?? "#1a1a2e";
	const childIds = getChildIds(component.children);

	const cameraType = useResolved<string>(component.cameraType) ?? "perspective";
	const cameraPosition = useResolved<[number, number, number]>(
		component.cameraPosition,
	) ?? [0, 0, 5];
	const controlMode =
		useResolved<ControlMode>(component.controlMode) ?? "orbit";
	const fixedView = useResolved<FixedView>(component.fixedView) ?? "front";
	const autoRotateSpeed = useResolved<number>(component.autoRotateSpeed) ?? 30;
	const enableControls = useResolved<boolean>(component.enableControls);
	const enableZoom = useResolved<boolean>(component.enableZoom) ?? true;
	const enablePan = useResolved<boolean>(component.enablePan) ?? true;
	const fov = useResolved<number>(component.fov) ?? 75;
	const near = useResolved<number>(component.near) ?? 0.1;
	const far = useResolved<number>(component.far) ?? 1000;
	const target = useResolved<[number, number, number]>(component.target) ?? [
		0, 0, 0,
	];
	const ambientLight = useResolved<number>(component.ambientLight) ?? 0.5;
	const directionalLight = useResolved<number>(component.directionalLight) ?? 1;
	const showGrid = useResolved<boolean>(component.showGrid) ?? false;
	const showAxes = useResolved<boolean>(component.showAxes) ?? false;

	const effectivePosition = useMemo(() => {
		if (controlMode === "fixed") {
			return FIXED_VIEW_POSITIONS[fixedView] || FIXED_VIEW_POSITIONS.front;
		}
		return cameraPosition;
	}, [controlMode, fixedView, cameraPosition]);

	const shouldEnableControls = useMemo(() => {
		if (enableControls !== undefined) return enableControls;
		return controlMode !== "fixed";
	}, [enableControls, controlMode]);

	return (
		<div
			ref={elementRef}
			className={cn("relative overflow-hidden rounded-lg", resolveStyle(style))}
			style={{
				width,
				height,
				...resolveInlineStyle(style),
			}}
		>
			<Canvas
				shadows
				gl={{ antialias: true, alpha: true }}
				style={{ background: backgroundColor }}
			>
				<Scene3DProvider>
					<Suspense fallback={<LoadingIndicator />}>
						<SceneContent
							cameraType={cameraType}
							cameraPosition={effectivePosition}
							controlMode={controlMode}
							autoRotateSpeed={autoRotateSpeed}
							enableControls={shouldEnableControls}
							enableZoom={enableZoom}
							enablePan={enablePan}
							fov={fov}
							near={near}
							far={far}
							target={target}
							ambientLight={ambientLight}
							directionalLight={directionalLight}
							showGrid={showGrid}
							showAxes={showAxes}
						>
							{childIds.map((id) => (
								<Fragment key={id}>{renderChild(id)}</Fragment>
							))}
						</SceneContent>
					</Suspense>
				</Scene3DProvider>
			</Canvas>
		</div>
	);
}

function LoadingIndicator() {
	return (
		<mesh>
			<boxGeometry args={[0.5, 0.5, 0.5]} />
			<meshStandardMaterial color="#666" wireframe />
		</mesh>
	);
}

interface SceneContentProps {
	cameraType: string;
	cameraPosition: [number, number, number];
	controlMode: ControlMode;
	autoRotateSpeed: number;
	enableControls: boolean;
	enableZoom: boolean;
	enablePan: boolean;
	fov: number;
	near: number;
	far: number;
	target: [number, number, number];
	ambientLight: number;
	directionalLight: number;
	showGrid: boolean;
	showAxes: boolean;
	children: React.ReactNode;
}

function SceneContent({
	cameraType,
	cameraPosition,
	controlMode,
	autoRotateSpeed,
	enableControls,
	enableZoom,
	enablePan,
	fov,
	near,
	far,
	target,
	ambientLight,
	directionalLight,
	showGrid,
	showAxes,
	children,
}: SceneContentProps) {
	const controlsRef = useRef<any>(null);

	return (
		<>
			{cameraType === "perspective" && (
				<PerspectiveCamera
					makeDefault
					position={cameraPosition}
					fov={fov}
					near={near}
					far={far}
				/>
			)}

			{enableControls && (
				<OrbitControls
					ref={controlsRef}
					target={target}
					enableZoom={enableZoom}
					enablePan={enablePan}
					autoRotate={controlMode === "auto-rotate"}
					autoRotateSpeed={autoRotateSpeed / 30}
					enableDamping
					dampingFactor={0.05}
				/>
			)}

			<ambientLight intensity={ambientLight} />
			<directionalLight
				position={[5, 5, 5]}
				intensity={directionalLight}
				castShadow
				shadow-mapSize-width={1024}
				shadow-mapSize-height={1024}
			/>
			<directionalLight
				position={[-5, 3, -5]}
				intensity={directionalLight * 0.3}
			/>

			{showGrid && <gridHelper args={[10, 10, "#444", "#222"]} />}
			{showAxes && <axesHelper args={[5]} />}

			{children}
		</>
	);
}
