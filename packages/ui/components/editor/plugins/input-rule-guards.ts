import type { SlateEditor } from "platejs";

export const notInside =
	(...keys: string[]) =>
	({ editor }: { editor: SlateEditor }) =>
		!editor.api.some({
			match: { type: keys.map((key) => editor.getType(key)) },
		});
