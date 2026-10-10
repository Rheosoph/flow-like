import { type Value, createSlateEditor } from "platejs";
import { serializeHtml } from "platejs/static";
import { BaseEditorKit } from "./editor-base-kit";
import { EditorStatic } from "./ui/editor-static";

export const PUBLICATION_CSS = `
@page { size: A4; margin: 18mm; }
* { box-sizing: border-box; }
body { color: #171717; background: white; font: 17px/1.65 Georgia, serif; margin: 0 auto; padding: 2rem; max-width: 52rem; }
h1,h2,h3,h4,h5,h6 { line-height: 1.2; font-family: system-ui,sans-serif; break-after: avoid; }
h1 { font-size: 2.4rem; } h2 { font-size: 1.8rem; } h3 { font-size: 1.4rem; }
p { margin: 0 0 1em; orphans: 3; widows: 3; }
a { color: #164e93; } img,video,iframe { max-width: 100%; } iframe { width: 100%; aspect-ratio: 16/9; border: 0; }
figure { margin: 1.25rem 0; break-inside: avoid; } figcaption,[data-media-credit] { font: .85rem/1.4 system-ui,sans-serif; color: #555; }
blockquote { margin-left: 0; padding-left: 1rem; border-left: 3px solid #777; }
table { border-collapse: collapse; width: 100%; } th,td { border: 1px solid #aaa; padding: .4rem; text-align: left; } thead { display: table-header-group; }
pre { white-space: pre-wrap; overflow-wrap: anywhere; background: #f3f3f3; padding: 1rem; }
.katex-html { display: none; } .katex-mathml math { font-size: 1.15em; }
[data-slate-spacer] { display: none !important; }
@media print { body { max-width: none; margin: 0; padding: 0; } a { color: inherit; } button,[role=toolbar] { display: none !important; } }
`;

/** Asset URLs must already be resolved by preparePublicationValue. */
export async function renderPublicationHtml(
	value: Value,
	title = "Document",
): Promise<string> {
	const editor = createSlateEditor({ plugins: BaseEditorKit, value });
	const body = await serializeHtml(editor, { editorComponent: EditorStatic });
	const escapedTitle = title.replace(
		/[&<>"']/g,
		(character) =>
			({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
				character
			] ?? character,
	);
	return `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>${escapedTitle}</title><style>${PUBLICATION_CSS}</style></head><body>${body}</body></html>`;
}
