import dynamicIconImports from "lucide-react/dynamicIconImports";

export const FLOW_LIKE_NOTIFICATION_ICON = "/app-logo.webp";

export type NotificationIconSource = {
	kind: "image" | "lucide" | "emoji";
	value: string;
};

const emojiSegments = new Intl.Segmenter(undefined, {
	granularity: "grapheme",
});

function isNotificationEmoji(value: string): boolean {
	if (new TextEncoder().encode(value).length > 64) return false;
	let count = 0;
	for (const { segment } of emojiSegments.segment(value)) {
		if (++count > 8 || /\p{White_Space}/u.test(segment)) return false;
		if (
			!segment.includes("\u20e3") &&
			!Array.from(segment).some(
				(scalar) =>
					(scalar.codePointAt(0) ?? 0) > 127 && /\p{Emoji}/u.test(scalar),
			)
		)
			return false;
	}
	return count > 0;
}

/** Interpret notification artwork without turning arbitrary text into a URL. */
export function notificationIconSource(
	icon?: unknown,
): NotificationIconSource | undefined {
	const value = typeof icon === "string" ? icon.trim() : undefined;
	if (!value || value.length > 4_194_304) return undefined;
	if (Object.hasOwn(dynamicIconImports, value))
		return { kind: "lucide", value };
	if (/\p{Cc}/u.test(value)) return undefined;
	if (/^data:image\/(?:png|jpe?g|gif|webp|avif|svg\+xml)[;,]/i.test(value))
		return { kind: "image", value };
	if (value.startsWith("/") && !value.startsWith("//") && !value.includes("\\"))
		return { kind: "image", value };
	try {
		const url = new URL(value);
		if (
			!url.username &&
			!url.password &&
			(url.protocol === "https:" ||
				url.protocol === "http:" ||
				(url.protocol === "asset:" && url.hostname === "localhost"))
		)
			return { kind: "image", value };
	} catch {
		if (
			/^(?:[\w-]+\/)*[\w.-]+\.(?:avif|gif|jpe?g|png|svg|webp)(?:[?#][^\s]*)?$/i.test(
				value,
			)
		)
			return { kind: "image", value };
	}
	if (value.length <= 64 && isNotificationEmoji(value))
		return { kind: "emoji", value };
	return undefined;
}

/** Old notifications stored the product logo when no custom icon was supplied. */
export function notificationIconCandidates(
	icon?: unknown,
	appIcon?: unknown,
): NotificationIconSource[] {
	const explicit = typeof icon === "string" ? icon.trim() : undefined;
	const candidates = [
		explicit === FLOW_LIKE_NOTIFICATION_ICON ? undefined : explicit,
		appIcon,
		FLOW_LIKE_NOTIFICATION_ICON,
	];
	const seen = new Set<string>();
	return candidates.flatMap((candidate) => {
		const source = notificationIconSource(candidate);
		if (!source || seen.has(source.value)) return [];
		seen.add(source.value);
		return [source];
	});
}

export function remoteNotificationIcon(
	data: Record<string, unknown>,
): string | undefined {
	const options = data.fcm_options;
	for (const candidate of [
		data.icon,
		data.image,
		options && typeof options === "object"
			? (options as Record<string, unknown>).image
			: undefined,
	]) {
		if (typeof candidate !== "string") continue;
		const source = notificationIconSource(candidate);
		if (source) return source.value;
	}
	return undefined;
}
