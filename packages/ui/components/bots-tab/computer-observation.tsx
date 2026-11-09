"use client";

import { useTranslation } from "@flow-like/locales";
import { useEffect, useState } from "react";
import type {
	IBotComputerView,
	IBotInteractionPoint,
	IBotInteractionRect,
} from "../../state/backend-state/bot-state";

export const OBSERVATION_FRESH_MS = 8000;

/** Use the device's image age, then advance it locally without comparing device clocks. */
export function useComputerFrameAge(view: IBotComputerView | undefined) {
	const [age, setAge] = useState(view?.age_ms ?? 0);
	useEffect(() => {
		const initial = Math.max(0, view?.age_ms ?? 0);
		const received = performance.now();
		setAge(initial);
		if (view?.state !== "active") return;
		const timer = setInterval(
			() => setAge(initial + Math.max(0, performance.now() - received)),
			1000,
		);
		return () => clearInterval(timer);
	}, [view]);
	return age;
}

function visiblePoint(point: IBotInteractionPoint | null | undefined) {
	return point &&
		Number.isFinite(point.x) &&
		Number.isFinite(point.y) &&
		point.x >= 0 &&
		point.x <= 1 &&
		point.y >= 0 &&
		point.y <= 1
		? point
		: null;
}

function visibleRect(rect: IBotInteractionRect | null | undefined) {
	if (
		!rect ||
		![rect.x, rect.y, rect.width, rect.height].every(Number.isFinite) ||
		rect.width <= 0 ||
		rect.height <= 0
	)
		return null;
	const x = Math.max(0, rect.x);
	const y = Math.max(0, rect.y);
	const width = Math.min(1, rect.x + rect.width) - x;
	const height = Math.min(1, rect.y + rect.height) - y;
	return width > 0 && height > 0 ? { x, y, width, height } : null;
}

export function computerObservation(view: IBotComputerView, age: number) {
	const observation = view.observation;
	const fresh = age <= OBSERVATION_FRESH_MS;
	const last = observation?.action;
	const actionAge = last
		? Math.max(0, (observation?.captured_at_ms ?? 0) - last.at_ms) + age
		: Number.POSITIVE_INFINITY;
	const action =
		fresh && !view.taken_over && actionAge <= OBSERVATION_FRESH_MS
			? last
			: null;
	const pointer =
		fresh &&
		observation?.pointer &&
		(observation.pointer.source === "observed" ||
			(!view.taken_over && action)) &&
		visiblePoint(observation.pointer.position)
			? observation.pointer
			: null;
	return {
		pointer,
		action,
		target: visiblePoint(action?.target),
		targetBounds: visibleRect(action?.target_bounds),
		focus: fresh ? observation?.focus : null,
		focusBounds: fresh ? visibleRect(observation?.focus?.bounds) : null,
	};
}

/** The SVG and image share an aspect ratio and contain fit, including their letterboxing. */
export function ObservedComputerImage({
	view,
	alt,
	age,
}: Readonly<{ view: IBotComputerView; alt: string; age: number }>) {
	const { pointer, target, targetBounds, focusBounds } = computerObservation(
		view,
		age,
	);
	const width = view.observation?.width ?? view.width ?? 0;
	const height = view.observation?.height ?? view.height ?? 0;
	const validSize = width > 0 && height > 0;
	const radius = Math.max(width, height) * 0.009;
	const rectangle = (bounds: IBotInteractionRect) => ({
		x: bounds.x * width,
		y: bounds.y * height,
		width: bounds.width * width,
		height: bounds.height * height,
	});
	return (
		<div className="relative h-full min-h-0 w-full" data-computer-image>
			<img
				src={view.image ?? undefined}
				alt={alt}
				draggable={false}
				className="absolute inset-0 h-full w-full select-none object-contain"
			/>
			{validSize && (pointer || target || targetBounds || focusBounds) && (
				<svg
					aria-hidden="true"
					focusable="false"
					data-computer-overlay
					className="pointer-events-none absolute inset-0 h-full w-full"
					viewBox={`0 0 ${width} ${height}`}
					preserveAspectRatio="xMidYMid meet"
					fill="none"
				>
					{focusBounds && (
						<rect
							data-focus-bounds
							{...rectangle(focusBounds)}
							className="stroke-info-solid"
							strokeWidth="2"
							vectorEffect="non-scaling-stroke"
						/>
					)}
					{targetBounds && (
						<rect
							data-target-bounds
							{...rectangle(targetBounds)}
							className="stroke-warning-solid"
							strokeWidth="2"
							strokeDasharray="5 4"
							vectorEffect="non-scaling-stroke"
						/>
					)}
					{target && (
						<circle
							data-action-target
							cx={target.x * width}
							cy={target.y * height}
							r={radius * 1.6}
							className="stroke-warning-solid"
							strokeWidth="2"
							strokeDasharray="4 3"
							vectorEffect="non-scaling-stroke"
						/>
					)}
					{pointer && (
						<circle
							data-pointer-source={pointer.source}
							cx={pointer.position.x * width}
							cy={pointer.position.y * height}
							r={radius}
							className={
								pointer.source === "observed"
									? "fill-info-solid stroke-background"
									: "fill-none stroke-warning-solid"
							}
							fillOpacity="0.35"
							strokeWidth="2"
							strokeDasharray={
								pointer.source === "dispatched" ? "3 3" : undefined
							}
							vectorEffect="non-scaling-stroke"
						/>
					)}
				</svg>
			)}
		</div>
	);
}

type Translate = ReturnType<typeof useTranslation>["t"];

function actionLabel(t: Translate, kind: string): string {
	switch (kind) {
		case "click":
			return String(t("computer.action.click", "Click"));
		case "double_click":
			return String(t("computer.action.double_click", "Double click"));
		case "right_click":
			return String(t("computer.action.right_click", "Right click"));
		case "move":
			return String(t("computer.action.move", "Move pointer"));
		case "mouse_move":
			return String(t("computer.action.mouse_move", "Move pointer"));
		case "hover":
			return String(t("computer.action.hover", "Hover"));
		case "drag":
			return String(t("computer.action.drag", "Drag"));
		case "scroll":
			return String(t("computer.action.scroll", "Scroll"));
		case "type":
			return String(t("computer.action.type", "Type"));
		case "text":
			return String(t("computer.action.text", "Type"));
		case "key":
			return String(t("computer.action.key", "Press key"));
		case "press":
			return String(t("computer.action.press", "Press key"));
		case "chord":
			return String(t("computer.action.chord", "Press keys"));
		case "select":
			return String(t("computer.action.select", "Select"));
		case "focus":
			return String(t("computer.action.focus", "Focus window"));
		case "navigate":
			return String(t("computer.action.navigate", "Navigate"));
		case "open":
			return String(t("computer.action.open", "Open page"));
		case "resize":
			return String(t("computer.action.resize", "Resize window"));
		case "move_window":
			return String(t("computer.action.move_window", "Move window"));
		case "minimize":
			return String(t("computer.action.minimize", "Minimize window"));
		case "maximize":
			return String(t("computer.action.maximize", "Maximize window"));
		case "close":
			return String(t("computer.action.close", "Close"));
		case "launch":
			return String(t("computer.action.launch", "Launch app"));
		case "other":
			return String(t("computer.action.other", "Action"));
		default:
			return String(t("computer.action.other", "Action"));
	}
}

export function ComputerObservationStatus({
	view,
	age,
}: Readonly<{ view: IBotComputerView; age: number }>) {
	const { t } = useTranslation("bots");
	const { pointer, action, target, targetBounds, focus } = computerObservation(
		view,
		age,
	);
	return (
		<div className="flex flex-wrap gap-x-3 gap-y-1 text-[11px] text-muted-foreground">
			{view.age_ms != null && (
				<span data-frame-age>
					{age < 1000
						? t("computer.frameNow", "Image just captured")
						: t("computer.frameAge", "Image {{seconds}}s old", {
								seconds: Math.floor(age / 1000),
							})}
				</span>
			)}
			{pointer && (
				<span>
					{pointer.source === "observed"
						? t("computer.observedCursor", "Observed cursor")
						: t("computer.dispatchedPointer", "Last dispatched pointer")}
				</span>
			)}
			{(target || targetBounds) && (
				<span>{t("computer.intendedTarget", "Intended target")}</span>
			)}
			{focus?.label && (
				<span className="max-w-full truncate" title={focus.label}>
					{t("computer.focus", "Focus: {{label}}", { label: focus.label })}
				</span>
			)}
			{action && (
				<output>
					{actionLabel(t, action.kind)}
					{" · "}
					{action.phase === "running"
						? t("computer.actionRunning", "In progress")
						: action.phase === "failed"
							? t("computer.actionFailed", "Failed")
							: t("computer.actionCompleted", "Completed")}
				</output>
			)}
		</div>
	);
}
