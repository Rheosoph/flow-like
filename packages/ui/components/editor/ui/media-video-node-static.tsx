"use client";

import type {
	SlateElementProps,
	TCaptionElement,
	TResizableProps,
	TVideoElement,
} from "platejs";

import { parseVideoUrl } from "@platejs/media";
import { NodeApi, SlateElement } from "platejs";
import LiteYouTubeEmbed from "react-lite-youtube-embed";
import ReactPlayer from "react-player";
import { cn } from "../../../lib";
import { useEditorAssetUrl } from "../hooks/use-editor-asset-url";

function getVideoType(url?: string) {
	if (!url) return { isYoutube: false, isUpload: false, embed: null };
	const parsed = parseVideoUrl(url);
	return {
		isYoutube: !!parsed?.id,
		isUpload: !parsed?.id,
		embed: parsed,
	};
}

export function VideoElementStatic(
	props: SlateElementProps<TVideoElement & TCaptionElement & TResizableProps>,
) {
	const { align = "center", caption, url, width } = props.element;
	const resolvedUrl = useEditorAssetUrl(url);

	const { isYoutube, isUpload, embed } = getVideoType(url);

	return (
		<SlateElement className="py-2.5" {...props}>
			<div style={{ textAlign: align }}>
				<figure
					className="group relative m-0 inline-block cursor-default"
					style={{ width }}
				>
					{isYoutube && embed?.id ? (
						<LiteYouTubeEmbed
							id={embed.id}
							title="youtube"
							wrapperClass={cn(
								"aspect-video rounded-sm",
								"relative block cursor-pointer bg-black bg-cover bg-center contain-content",
								"[&.lyt-activated]:before:absolute [&.lyt-activated]:before:top-0 [&.lyt-activated]:before:h-[60px] [&.lyt-activated]:before:w-full [&.lyt-activated]:before:bg-top [&.lyt-activated]:before:bg-repeat-x [&.lyt-activated]:before:pb-[50px] [&.lyt-activated]:before:[transition:all_0.2s_cubic-bezier(0,0,0.2,1)]",
								"[&.lyt-activated]:before:bg-[url(data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAADGCAYAAAAT+OqFAAAAdklEQVQoz42QQQ7AIAgEF/T/D+kbq/RWAlnQyyazA4aoAB4FsBSA/bFjuF1EOL7VbrIrBuusmrt4ZZORfb6ehbWdnRHEIiITaEUKa5EJqUakRSaEYBJSCY2dEstQY7AuxahwXFrvZmWl2rh4JZ07z9dLtesfNj5q0FU3A5ObbwAAAABJRU5ErkJggg==)]",
								'after:block after:pb-(--aspect-ratio) after:content-[""]',
								"[&_>_iframe]:absolute [&_>_iframe]:top-0 [&_>_iframe]:left-0 [&_>_iframe]:size-full",
								"[&_>_.lty-playbtn]:z-1 [&_>_.lty-playbtn]:h-[46px] [&_>_.lty-playbtn]:w-[70px] [&_>_.lty-playbtn]:rounded-[14%] [&_>_.lty-playbtn]:bg-[#212121] [&_>_.lty-playbtn]:opacity-80 [&_>_.lty-playbtn]:[transition:all_0.2s_cubic-bezier(0,0,0.2,1)]",
								"[&:hover_>_.lty-playbtn]:bg-[red] [&:hover_>_.lty-playbtn]:opacity-100",
								'[&_>_.lty-playbtn]:before:border-y-11 [&_>_.lty-playbtn]:before:border-r-0 [&_>_.lty-playbtn]:before:border-l-19 [&_>_.lty-playbtn]:before:border-[transparent_transparent_transparent_#fff] [&_>_.lty-playbtn]:before:content-[""]',
								"[&_>_.lty-playbtn]:absolute [&_>_.lty-playbtn]:top-1/2 [&_>_.lty-playbtn]:left-1/2 [&_>_.lty-playbtn]:[transform:translate3d(-50%,-50%,0)]",
								"[&_>_.lty-playbtn]:before:absolute [&_>_.lty-playbtn]:before:top-1/2 [&_>_.lty-playbtn]:before:left-1/2 [&_>_.lty-playbtn]:before:[transform:translate3d(-50%,-50%,0)]",
								"[&.lyt-activated]:cursor-[unset]",
								"[&.lyt-activated]:before:pointer-events-none [&.lyt-activated]:before:opacity-0",
								"[&.lyt-activated_>_.lty-playbtn]:pointer-events-none [&.lyt-activated_>_.lty-playbtn]:opacity-0!",
							)}
						/>
					) : (
						<ReactPlayer src={resolvedUrl} width="100%" height="100%" controls />
					)}
					{caption && <figcaption>{NodeApi.string(caption[0])}</figcaption>}
				</figure>
			</div>
			{props.children}
		</SlateElement>
	);
}
