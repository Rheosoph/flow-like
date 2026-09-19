import { expect, test } from "bun:test";
import { createWidgetMediaClient } from "../src/media";
import { createEnvelope } from "../src/protocol";
import { filterPropsPatch } from "../src/mount";
test("media bridge sends only granted ids and receives safe status", async () => {
	const sent: any[] = [];
	const media = createWidgetMediaClient(
		(type, payload) => sent.push({ type, payload }),
		() => ({ media: true, mediaIds: ["radio"] }),
	);
	await expect(media.playMedia("https://arbitrary")).rejects.toThrow(
		"not granted",
	);
	expect(sent).toHaveLength(0);
	const promise = media.playMedia("radio");
	const request = sent[0].payload;
	expect(request.mediaId).toBe("radio");
	media.handle(
		createEnvelope(
			"media:result",
			{ requestId: request.requestId, ok: true },
			"n",
			"i",
		),
	);
	await promise;
	media.handle(
		createEnvelope("media:state", { id: "radio", state: "playing" }, "n", "i"),
	);
	expect(media.$media.get()).toEqual({ id: "radio", state: "playing" });
	media.pauseMedia();
	media.stopMedia();
	expect(sent.slice(-2).map((row) => row.type)).toEqual([
		"media:pause",
		"media:stop",
	]);
	media.dispose();
});
test("reset rejects pending play and removes media grants from props", async () => {
	const media = createWidgetMediaClient(
		() => {},
		() => ({ media: true, mediaIds: ["radio"] }),
	);
	const promise = media.playMedia("radio");
	media.reset();
	await expect(promise).rejects.toThrow();
	expect(
		filterPropsPatch(undefined, {
			publicMediaGrants: [{ id: "radio", url: "private" }],
			name: "view",
		}).accepted,
	).toEqual({ name: "view" });
	media.dispose();
});
