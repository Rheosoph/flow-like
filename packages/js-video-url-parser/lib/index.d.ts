export type VideoProvider =
	| "youtube"
	| "vimeo"
	| "dailymotion"
	| "youku"
	| "coub";
export interface VideoInfo {
	provider: VideoProvider;
	id: string;
	mediaType: "video";
}
export declare function parse(input: string): VideoInfo | undefined;
declare const parser: {
	parse: typeof parse;
};
export default parser;
