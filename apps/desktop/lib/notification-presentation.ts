export type LocalNotificationOptions = {
	title: string;
	body?: string;
	attachments?: { id: string; url: string }[];
};

type NotificationContent = {
	title: string;
	description?: string;
	icon?: string;
};

// iOS attachments must point at a native file. A WebView blob URL cannot be
// opened by UserNotifications, so prepare the file before scheduling the alert.
export async function presentLocalNotification(
	content: NotificationContent,
	{
		isIOS,
		prepareAttachment,
		send,
	}: {
		isIOS: boolean;
		prepareAttachment: (source: string) => Promise<string>;
		send: (options: LocalNotificationOptions) => Promise<void>;
	},
): Promise<void> {
	const options: LocalNotificationOptions = {
		title: content.title,
		body: content.description,
	};
	if (isIOS && content.icon?.trim()) {
		try {
			const url = await prepareAttachment(content.icon);
			options.attachments = [{ id: "notification-image", url }];
		} catch {
			// An unavailable image must not prevent the text alert.
		}
	}
	try {
		await send(options);
	} catch (error) {
		if (!options.attachments) throw error;
		await send({ title: options.title, body: options.body });
	}
}

export function notificationImageSource(icon?: string): string | undefined {
	const source = icon?.trim();
	if (!source) return undefined;
	if (/^data:image\/(png|jpeg|gif|webp);base64,/i.test(source)) return source;
	try {
		const url = new URL(source);
		if (url.username || url.password) return undefined;
		if (
			url.protocol === "https:" ||
			(url.protocol === "asset:" && url.hostname === "localhost") ||
			(url.protocol === "http:" && url.hostname === "asset.localhost")
		) {
			return source;
		}
	} catch {
		return undefined;
	}
	return undefined;
}

export function remoteNotificationImage(
	data: Record<string, unknown>,
): string | undefined {
	const fcmOptions = data.fcm_options;
	const candidates = [
		data.image,
		fcmOptions && typeof fcmOptions === "object"
			? (fcmOptions as Record<string, unknown>).image
			: undefined,
		data.icon,
	];
	for (const candidate of candidates) {
		if (typeof candidate !== "string") continue;
		const source = notificationImageSource(candidate);
		if (source) return source;
	}
	return undefined;
}

export function shouldShowRemotePushToast(platform: string | null): boolean {
	// The iOS notification delegate already presents foreground push banners.
	return platform !== "IOS";
}
