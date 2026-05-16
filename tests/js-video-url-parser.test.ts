import { describe, expect, test } from "bun:test";
import videoParser from "js-video-url-parser";

describe("js-video-url-parser shim", () => {
	test("parses the video providers used by PlateJS media embeds", () => {
		expect(videoParser.parse("https://youtu.be/dQw4w9WgXcQ")).toEqual({
			provider: "youtube",
			id: "dQw4w9WgXcQ",
			mediaType: "video",
		});
		expect(
			videoParser.parse("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
		).toEqual({
			provider: "youtube",
			id: "dQw4w9WgXcQ",
			mediaType: "video",
		});
		expect(
			videoParser.parse("https://player.vimeo.com/video/123456789"),
		).toEqual({
			provider: "vimeo",
			id: "123456789",
			mediaType: "video",
		});
		expect(
			videoParser.parse("https://www.dailymotion.com/video/x7tgad0"),
		).toEqual({
			provider: "dailymotion",
			id: "x7tgad0",
			mediaType: "video",
		});
		expect(
			videoParser.parse("https://player.youku.com/embed/XNDQ4NDYxMzM2OA=="),
		).toEqual({
			provider: "youku",
			id: "XNDQ4NDYxMzM2OA==",
			mediaType: "video",
		});
		expect(videoParser.parse("https://coub.com/view/2abcde")).toEqual({
			provider: "coub",
			id: "2abcde",
			mediaType: "video",
		});
	});

	test("rejects invalid and oversized input", () => {
		expect(videoParser.parse("not a url")).toBeUndefined();
		expect(videoParser.parse("ftp://youtu.be/dQw4w9WgXcQ")).toBeUndefined();
		expect(
			videoParser.parse(`https://youtu.be/${"a".repeat(2049)}`),
		).toBeUndefined();
	});
});
