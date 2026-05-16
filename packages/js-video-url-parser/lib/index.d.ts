export type VideoProvider = "youtube" | "vimeo" | "dailymotion" | "youku" | "coub";

export interface VideoInfo {
	provider: VideoProvider;
	id: string;
}

export function parse(url: string): VideoInfo | undefined;

declare const parser: {
	parse: typeof parse;
};

export default parser;
