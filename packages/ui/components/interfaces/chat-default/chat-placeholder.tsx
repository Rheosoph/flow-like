"use client";

import { useEffect, useRef, useState } from "react";
import {
	resolveChatPlaceholderBubbleState,
	resolveChatPlaceholderTypingMotion,
	resolveChatPlaceholderVisual,
} from "../../../lib/chat-appearance";
import type { IComposerActivityChannel } from "../../../lib/composer-activity";
import {
	isStoragePrefix,
	presignSinglePath,
} from "../../../lib/presign-assets";
import type { IEventPayloadChat } from "../../../lib/schema/flow/event-payload-chat";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import { BubbleOrbFilm } from "../../global-chat/bubble-orb-film";
import { ChatEmptyOrb } from "./chat-empty-orb";

interface IChatPlaceholderProps {
	readonly config?: Partial<IEventPayloadChat>;
	readonly appId?: string;
	readonly size?: number;
	readonly className?: string;
	/**
	 * The composer's draft channel. Supply it and the mark reacts while the user types, provided
	 * the interface enabled `placeholder_typing_motion`.
	 */
	readonly activity?: IComposerActivityChannel;
}

/**
 * Resolves a storage path to a signed URL. Plain URLs are handed straight back, so an external
 * image keeps working with no app storage involved.
 */
function useResolvedImage(source: string, appId?: string) {
	const backend = useBackend();
	const [resolved, setResolved] = useState<string | undefined>(() =>
		source && !isStoragePrefix(source) ? source : undefined,
	);

	useEffect(() => {
		if (!source) {
			setResolved(undefined);
			return;
		}
		if (!isStoragePrefix(source)) {
			setResolved(source);
			return;
		}
		if (!appId) {
			setResolved(undefined);
			return;
		}
		let cancelled = false;
		void presignSinglePath(appId, source, backend.storageState).then((url) => {
			if (cancelled) return;
			setResolved(url && !isStoragePrefix(url) ? url : undefined);
		});
		return () => {
			cancelled = true;
		};
	}, [source, appId, backend.storageState]);

	return resolved;
}

/** The bubble needs a sized, positioned host — the film reads its box and pointer from it. */
function BubblePlaceholder({
	state,
	size,
	activity,
	typingMotion,
}: {
	state: ReturnType<typeof resolveChatPlaceholderBubbleState>;
	size: number;
	activity?: IComposerActivityChannel;
	typingMotion: boolean;
}) {
	// A stable ref object, not `{current: el}` — the film keys its GL context on this identity and
	// would tear the context down and rebuild it on every render.
	const hostRef = useRef<HTMLDivElement>(null);

	return (
		<div
			ref={hostRef}
			aria-hidden="true"
			className="relative shrink-0 overflow-visible"
			style={{ width: size, height: size }}
		>
			<BubbleOrbFilm
				hostRef={hostRef}
				state={state}
				ackNonce={0}
				activity={activity}
				typingMotion={typingMotion}
			/>
		</div>
	);
}

/**
 * The empty-chat mark. Which one you get is the interface's own setting, so an app can show the
 * planet, the FlowPilot orb in any of its poses, its own avatar, or nothing at all.
 */
export function ChatPlaceholder({
	config = {},
	appId,
	size = 168,
	className,
	activity,
}: IChatPlaceholderProps) {
	const visual = resolveChatPlaceholderVisual(config.placeholder_visual);
	const typingMotion = resolveChatPlaceholderTypingMotion(
		config.placeholder_typing_motion,
	);
	const source =
		typeof config.placeholder_image === "string"
			? config.placeholder_image.trim()
			: "";
	// Hooks can't sit behind the branch below, so the image always resolves; it is simply unused
	// unless the interface asked for one.
	const image = useResolvedImage(visual === "image" ? source : "", appId);

	if (visual === "none") return null;

	if (visual === "image") {
		if (!image) return null;
		return (
			<img
				src={image}
				alt=""
				aria-hidden="true"
				className={cn(
					"shrink-0 rounded-full object-cover ring-1 ring-border/60",
					className,
				)}
				style={{ width: size, height: size }}
			/>
		);
	}

	if (visual === "bubble") {
		return (
			<div className={cn("shrink-0", className)}>
				<BubblePlaceholder
					state={resolveChatPlaceholderBubbleState(
						config.placeholder_bubble_state,
					)}
					size={size}
					activity={activity}
					typingMotion={typingMotion}
				/>
			</div>
		);
	}

	return (
		<ChatEmptyOrb
			size={size}
			className={className}
			activity={activity}
			typingMotion={typingMotion}
		/>
	);
}
