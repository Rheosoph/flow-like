import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { BotAvatar } from "./bot-avatar";
import type { BotAppearance } from "./types";

const AVATAR_PIXELS = 256;
const SVG_NAMESPACE = "http://www.w3.org/2000/svg";

/**
 * The bot's look as a standalone, still SVG on a square tile. Notification avatars crop to
 * their own shape, so the frame is left to them.
 */
export function botAvatarSvg(appearance: BotAppearance): string {
	const markup = renderToStaticMarkup(
		createElement(BotAvatar, {
			appearance: { ...appearance, frame: "square" },
			animated: false,
			decorative: true,
			size: AVATAR_PIXELS,
		}),
	);
	const start = markup.indexOf("<svg");
	const end = markup.lastIndexOf("</svg>");
	if (start === -1 || end === -1)
		throw new Error(`Bot look '${appearance.kind}' rendered no SVG`);
	return markup
		.slice(start, end + "</svg>".length)
		.replace("<svg", `<svg xmlns="${SVG_NAMESPACE}"`);
}

/** A PNG data URL of the bot's look, for the picture push notifications show. */
export async function botAvatarPng(appearance: BotAppearance): Promise<string> {
	const source = URL.createObjectURL(
		new Blob([botAvatarSvg(appearance)], { type: "image/svg+xml" }),
	);
	try {
		const image = new Image();
		image.src = source;
		await image.decode();
		const canvas = document.createElement("canvas");
		canvas.width = AVATAR_PIXELS;
		canvas.height = AVATAR_PIXELS;
		const context = canvas.getContext("2d");
		if (!context)
			throw new Error(
				`Cannot draw bot look '${appearance.kind}': no 2D canvas context`,
			);
		context.drawImage(image, 0, 0, AVATAR_PIXELS, AVATAR_PIXELS);
		return canvas.toDataURL("image/png");
	} finally {
		URL.revokeObjectURL(source);
	}
}
