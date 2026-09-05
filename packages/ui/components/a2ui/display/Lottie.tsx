"use client";

import { useEffect, useRef, useState } from "react";
import { cn } from "../../../lib/utils";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import { useAssetUrl } from "../hooks/use-asset-url";
import { useElementRef } from "../hooks/use-element-ref";
import type { BoundValue, LottieComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

function isInlineAnimation(value: string | undefined): value is string {
	const start = value?.trimStart().charAt(0);
	return start === "{" || start === "[";
}

export function A2UILottie({
	elementRef,
	component,
	style,
}: ComponentProps<LottieComponent>) {
	// `src` doubles as an inline animation document, which is neither a URL nor
	// a path and must never be sent off to be signed.
	const rawSrc = useResolved<string>(component.src);
	const inlineJson = isInlineAnimation(rawSrc) ? rawSrc : undefined;
	const { url: resolvedSrc } = useAssetUrl(inlineJson ? undefined : rawSrc);
	const src = inlineJson ?? resolvedSrc;
	const loop = useResolved<boolean>(component.loop);
	const autoplay = useResolved<boolean>(component.autoplay);
	const speed = useResolved<number>(component.speed);
	const containerRef = useRef<HTMLDivElement>(null);
	const rootRef = useElementRef(elementRef, containerRef);
	const [animationData, setAnimationData] = useState<unknown>(null);
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		if (!src) return;

		const loadAnimation = async () => {
			try {
				if (src.startsWith("http") || src.startsWith("/")) {
					const response = await fetch(src);
					const data = await response.json();
					setAnimationData(data);
				} else {
					setAnimationData(JSON.parse(src));
				}
			} catch (err) {
				console.error("Failed to load Lottie animation:", err);
				setError("Failed to load animation");
			}
		};

		loadAnimation();
	}, [src]);

	useEffect(() => {
		if (!containerRef.current || !animationData) return;

		let animation: { destroy: () => void } | null = null;

		const loadLottie = async () => {
			try {
				// Dynamic import with type assertion for optional dependency
				const lottieModule = await import("lottie-web" as string).catch(
					() => null,
				);
				if (!lottieModule) {
					setError("lottie-web not installed");
					return;
				}

				const lottie = lottieModule.default;
				animation = lottie.loadAnimation({
					container: containerRef.current!,
					renderer: "svg",
					loop: loop ?? true,
					autoplay: autoplay ?? true,
					animationData,
				});

				if (speed && speed !== 1) {
					(animation as unknown as { setSpeed: (s: number) => void }).setSpeed(
						speed,
					);
				}
			} catch (err) {
				console.error("lottie-web error:", err);
				setError("Animation error");
			}
		};

		loadLottie();

		return () => {
			animation?.destroy();
		};
	}, [animationData, autoplay, loop, speed]);

	if (!src) return null;

	if (error) {
		return (
			<div
				ref={elementRef}
				className={cn(
					"flex items-center justify-center text-muted-foreground text-sm",
					resolveStyle(style),
				)}
				style={resolveInlineStyle(style)}
			>
				{error}
			</div>
		);
	}

	return (
		<div
			ref={rootRef}
			className={cn(resolveStyle(style))}
			style={resolveInlineStyle(style)}
		/>
	);
}
