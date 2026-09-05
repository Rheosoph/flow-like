import { afterEach, describe, expect, mock, test } from "bun:test";
import {
	completeMediaUpload,
	updateAccountWithAvatar,
	uploadProfileMedia,
} from "./profile-media-upload";

const originalFetch = globalThis.fetch;
afterEach(() => {
	globalThis.fetch = originalFetch;
});

describe("account photo upload", () => {
	test("a rejected upload cannot commit the image or change account fields", async () => {
		globalThis.fetch = mock(
			async () => new Response("denied", { status: 403 }),
		) as typeof fetch;
		const request = mock(async () => ({
			signed_url: "https://storage.test/upload",
			avatar_upload_id: "staged.webp",
		}));
		const data = { name: "Changed name", description: "Unsaved biography" };
		await expect(
			updateAccountWithAvatar(
				data,
				new File(["png"], "photo.png", { type: "image/png" }),
				request,
			),
		).rejects.toThrow("403");
		expect(request.mock.calls).toEqual([[{ avatar_extension: "png" }]]);
		expect(data).toEqual({
			name: "Changed name",
			description: "Unsaved biography",
		});
	});

	test("commits account fields only after a successful image upload", async () => {
		const events: string[] = [];
		globalThis.fetch = mock(async () => {
			events.push("upload");
			return new Response(null, { status: 200 });
		}) as typeof fetch;
		const request = mock(async (body: Record<string, unknown>) => {
			if (body.avatar_extension) {
				events.push("prepare");
				return {
					signed_url: "https://storage.test/upload",
					avatar_upload_id: "staged.webp",
				};
			}
			events.push("finalize");
			return { upload_pending: false };
		});
		await updateAccountWithAvatar(
			{ name: "New name" },
			new File(["image"], "photo.webp"),
			request,
		);
		expect(events).toEqual(["prepare", "upload", "finalize"]);
		expect(request.mock.calls[1]).toEqual([
			{ name: "New name", avatar_upload_id: "staged.webp" },
		]);
	});

	test("missing completion metadata fails before uploading", async () => {
		const upload = mock(async () => new Response(null));
		globalThis.fetch = upload as typeof fetch;
		await expect(
			updateAccountWithAvatar(
				{},
				new File(["image"], "photo.png"),
				async () => ({ signed_url: "https://storage.test/upload" }),
			),
		).rejects.toThrow("did not prepare");
		expect(upload).not.toHaveBeenCalled();
	});

	test("ordinary account updates make a single request", async () => {
		const request = mock(async (_body: Record<string, unknown>) => ({}));
		await updateAccountWithAvatar({ name: "Name" }, undefined, request);
		expect(request.mock.calls).toEqual([[{ name: "Name" }]]);
	});

	test("Azure uploads include the required block blob header", async () => {
		const upload = mock(
			async (_url: string, _init: RequestInit) => new Response(null),
		);
		globalThis.fetch = upload as unknown as typeof fetch;
		await uploadProfileMedia(
			"https://account.blob.core.windows.net/media/photo.webp?sig=token",
			new File(["image"], "photo.webp", { type: "image/webp" }),
		);
		expect(upload.mock.calls[0]?.[1].headers).toEqual({
			"Content-Type": "image/webp",
			"x-ms-blob-type": "BlockBlob",
		});
	});
});

describe("image completion", () => {
	test("waits through transformer pending responses", async () => {
		let calls = 0;
		await completeMediaUpload(async () => ({ upload_pending: ++calls < 3 }), {
			attempts: 4,
			delayMs: 0,
		});
		expect(calls).toBe(3);
	});

	test("a processing timeout preserves the previous image and reports failure", async () => {
		const finalize = mock(async () => ({ upload_pending: true }));
		await expect(
			completeMediaUpload(finalize, { attempts: 2, delayMs: 0 }),
		).rejects.toThrow("previous image is still in use");
		expect(finalize).toHaveBeenCalledTimes(2);
	});

	test("validation or authorization failures stop immediately", async () => {
		const finalize = mock(async () => {
			throw new Error("Invalid image");
		});
		await expect(
			completeMediaUpload(finalize, { attempts: 3, delayMs: 0 }),
		).rejects.toThrow("Invalid image");
		expect(finalize).toHaveBeenCalledTimes(1);
	});
});
