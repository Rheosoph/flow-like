import { describe, expect, it, vi } from "vitest";
import {
	notificationImageSource,
	presentLocalNotification,
	remoteNotificationImage,
	shouldShowRemotePushToast,
} from "../notification-presentation";

const content = {
	title: "Export ready",
	description: "Open your report",
	icon: "https://assets.example/report.png",
};

describe("notification presentation", () => {
	it("waits for a native file before attaching an iOS notification image", async () => {
		const send = vi.fn().mockResolvedValue(undefined);
		let finish: (path: string) => void = () => {};
		const prepareAttachment = vi.fn().mockImplementation(
			() =>
				new Promise<string>((resolve) => {
					finish = resolve;
				}),
		);
		const pending = presentLocalNotification(content, {
			isIOS: true,
			prepareAttachment,
			send,
		});
		expect(send).not.toHaveBeenCalled();
		finish("file:///cache/report.png");
		await pending;
		expect(send).toHaveBeenCalledExactlyOnceWith({
			title: content.title,
			body: content.description,
			attachments: [
				{ id: "notification-image", url: "file:///cache/report.png" },
			],
		});
	});

	it("keeps the text when image download or validation fails", async () => {
		const send = vi.fn().mockResolvedValue(undefined);
		await presentLocalNotification(content, {
			isIOS: true,
			prepareAttachment: vi.fn().mockRejectedValue(new Error("invalid image")),
			send,
		});
		expect(send).toHaveBeenCalledExactlyOnceWith({
			title: content.title,
			body: content.description,
		});
	});

	it("retries text once if iOS rejects the prepared attachment", async () => {
		const send = vi
			.fn()
			.mockRejectedValueOnce(new Error("attachment removed"))
			.mockResolvedValue(undefined);
		await presentLocalNotification(content, {
			isIOS: true,
			prepareAttachment: async () => "file:///cache/report.png",
			send,
		});
		expect(send).toHaveBeenCalledTimes(2);
		expect(send.mock.calls[1][0]).toEqual({
			title: content.title,
			body: content.description,
		});
	});

	it("propagates text scheduling failures so the caller can show an in-app alert", async () => {
		await expect(
			presentLocalNotification(content, {
				isIOS: false,
				prepareAttachment: vi.fn(),
				send: vi.fn().mockRejectedValue(new Error("permission revoked")),
			}),
		).rejects.toThrow("permission revoked");
	});

	it("skips iOS-only preparation for other platforms and empty image inputs", async () => {
		const prepareAttachment = vi.fn();
		const send = vi.fn().mockResolvedValue(undefined);
		await presentLocalNotification(content, {
			isIOS: false,
			prepareAttachment,
			send,
		});
		await presentLocalNotification(
			{ ...content, icon: " " },
			{
				isIOS: true,
				prepareAttachment,
				send,
			},
		);
		expect(prepareAttachment).not.toHaveBeenCalled();
		expect(send).toHaveBeenCalledTimes(2);
	});

	it("uses one foreground presentation for iOS pushes", () => {
		expect(shouldShowRemotePushToast("IOS")).toBe(false);
		expect(shouldShowRemotePushToast("ANDROID")).toBe(true);
		expect(shouldShowRemotePushToast("DESKTOP")).toBe(true);
	});

	it("reads provider image fields when push metadata omits the legacy icon", () => {
		expect(
			remoteNotificationImage({ fcm_options: { image: content.icon } }),
		).toBe(content.icon);
		expect(remoteNotificationImage({ image: content.icon })).toBe(content.icon);
		expect(remoteNotificationImage({ icon: content.icon })).toBe(content.icon);
		expect(
			remoteNotificationImage({ fcm_options: { image: 42 } }),
		).toBeUndefined();
	});

	it("accepts raster and Tauri image sources but rejects executable or credentialed URLs", () => {
		for (const source of [
			content.icon,
			"asset://localhost/%2Ftmp%2Fa.png",
			"http://asset.localhost/%2Ftmp%2Fa.png",
			"data:image/png;base64,aGVsbG8=",
		])
			expect(notificationImageSource(source)).toBe(source);
		for (const source of [
			"javascript:alert(1)",
			"file:///etc/passwd",
			"https://user:secret@assets.example/image.png",
			"data:text/html;base64,eA==",
			"data:image/svg+xml;base64,eA==",
			"http://untrusted.example/image.png",
			" ",
		])
			expect(notificationImageSource(source)).toBeUndefined();
	});
});
