"use client";

import { useTranslation } from "@flow-like/locales";
import { MaximizeIcon } from "lucide-react";
import { useEffect, useRef } from "react";
import type { CallVideoView } from "../../hooks/use-conversation-call";
import { cn } from "../../lib/utils";
import { Button } from "../ui/button";

export function CallVideoTile({
	video,
	name,
}: Readonly<{ video: CallVideoView; name: string }>) {
	const { t } = useTranslation("bots");
	const element = useRef<HTMLVideoElement>(null);
	const container = useRef<HTMLElement>(null);
	useEffect(() => {
		const node = element.current;
		if (!node) return;
		node.srcObject = video.stream;
		void node.play()?.catch(() => {});
		return () => {
			node.srcObject = null;
		};
	}, [video.stream]);
	const label = video.local
		? video.kind === "screen"
			? t("crew.call.yourScreen", "Your screen")
			: t("crew.call.yourCamera", "Your camera")
		: video.kind === "screen"
			? t("crew.call.screenLabel", "{{name}}'s screen", { name })
			: t("crew.call.cameraLabel", "{{name}}'s camera", { name });
	return (
		<figure
			ref={container}
			data-slot="call-video-tile"
			data-kind={video.kind}
			className={cn(
				"relative m-0 min-w-0 overflow-hidden rounded-lg bg-black",
				video.kind === "screen" && "col-span-full",
			)}
		>
			<video
				ref={element}
				autoPlay
				playsInline
				muted
				aria-label={label}
				className={cn(
					"aspect-video w-full max-h-[50vh] object-contain",
					video.local && video.kind === "camera" && "-scale-x-100",
				)}
			/>
			<figcaption className="absolute bottom-2 left-2 max-w-[80%] truncate rounded bg-black/70 px-2 py-1 text-xs text-white">
				{label}
				{video.local && ` · ${t("crew.call.you", "You")}`}
			</figcaption>
			<Button
				type="button"
				variant="ghost"
				size="icon"
				className="absolute right-1 top-1 text-white hover:bg-white/20"
				aria-label={t("crew.call.fullscreen", "Expand {{name}}", {
					name: label,
				})}
				onClick={() => {
					void container.current?.requestFullscreen?.().catch(() => {});
				}}
			>
				<MaximizeIcon aria-hidden />
			</Button>
		</figure>
	);
}
