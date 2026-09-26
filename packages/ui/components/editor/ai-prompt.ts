import {
	type AIMode,
	type EditorPrompt,
	type EditorPromptParams,
	getEditorPrompt,
} from "@platejs/ai";
import { AIChatPlugin } from "@platejs/ai/react";
import { serializeMd } from "@platejs/markdown";
import { BlockSelectionPlugin } from "@platejs/selection/react";
import { KEYS, type SlateEditor, type TElement } from "platejs";
import type { PlateEditor } from "platejs/react";

const systemCommon = `\
You are an advanced AI-powered note-taking assistant, designed to enhance productivity and creativity in note management.
Respond directly to user prompts with clear, concise, and relevant content. Maintain a neutral, helpful tone.

Rules:
- <Document> is the entire note the user is working on.
- <Reminder> is a reminder of how you should reply to INSTRUCTIONS. It does not apply to questions.
- Anything else is the user prompt.
- Your response should be tailored to the user's prompt, providing precise assistance to optimize note management.
- For INSTRUCTIONS: Follow the <Reminder> exactly. Provide ONLY the content to be inserted or replaced. No explanations or comments.
- For QUESTIONS: Provide a helpful and concise answer. You may include brief explanations if necessary.
- CRITICAL: DO NOT remove or modify the following custom MDX tags: <u>, <callout>, <kbd>, <toc>, <sub>, <sup>, <mark>, <del>, <date>, <span>, <column>, <column_group>, <file>, <audio>, <video> in <Selection> unless the user explicitly requests this change.
- CRITICAL: Distinguish between INSTRUCTIONS and QUESTIONS. Instructions typically ask you to modify or add content. Questions ask for information or clarification.
- CRITICAL: when asked to write in markdown, do not start with \`\`\`markdown.
`;

const systemDefault = `\
${systemCommon}
- <Block> is the current block of text the user is working on.
- Ensure your output can seamlessly fit into the existing <Block> structure.

<Block>
{block}
</Block>
`;

const systemSelecting = `\
${systemCommon}
- <Block> is the block of text containing the user's selection, providing context.
- Ensure your output can seamlessly fit into the existing <Block> structure.
- <Selection> is the specific text the user has selected in the block and wants to modify or ask about.
- Consider the context provided by <Block>, but only modify <Selection>. Your response should be a direct replacement for <Selection>.
<Block>
{block}
</Block>
<Selection>
{selection}
</Selection>
`;

const systemBlockSelecting = `\
${systemCommon}
- <Selection> represents the full blocks of text the user has selected and wants to modify or ask about.
- Your response should be a direct replacement for the entire <Selection>.
- Maintain the overall structure and formatting of the selected blocks, unless explicitly instructed otherwise.
- CRITICAL: Provide only the content to replace <Selection>. Do not add additional blocks or change the block structure unless specifically requested.
<Selection>
{block}
</Selection>
`;

const userDefault = `
${systemDefault}
<Reminder>
CRITICAL: NEVER write <Block>.
</Reminder>
{prompt}`;

const userSelecting = `
${systemSelecting}
<Reminder>
If this is a question, provide a helpful and concise answer about <Selection>.
If this is an instruction, provide ONLY the text to replace <Selection>. No explanations.
Ensure it fits seamlessly within <Block>. If <Block> is empty, write ONE random sentence.
NEVER write <Block> or <Selection>.
</Reminder>
{prompt} about <Selection>`;

const userBlockSelecting = `
${systemBlockSelecting}
<Reminder>
If this is a question, provide a helpful and concise answer about <Selection>.
If this is an instruction, provide ONLY the content to replace the entire <Selection>. No explanations.
Maintain the overall structure unless instructed otherwise.
NEVER write <Block> or <Selection>.
</Reminder>
{prompt} about <Selection>`;

export const PROMPT_TEMPLATES = {
	systemBlockSelecting,
	systemDefault,
	systemSelecting,
	userBlockSelecting,
	userDefault,
	userSelecting,
};

/** Block-selected blocks, otherwise the highest blocks touched by the selection. */
const blockMarkdown = (editor: SlateEditor) => {
	const entries = editor.getOption(BlockSelectionPlugin, "isSelectingSome")
		? editor.getApi(BlockSelectionPlugin).blockSelection.getNodes()
		: editor.api.nodes({
				mode: "highest",
				match: (node) => editor.api.isBlock(node),
			});
	return serializeMd(editor, {
		value: Array.from(entries, ([node]) => node as TElement),
	});
};

/** A selection inside one block is serialized as a bare paragraph. */
const selectionMarkdown = (editor: SlateEditor) => {
	const fragment = editor.api.fragment<TElement>();
	return serializeMd(editor, {
		value:
			fragment.length === 1
				? [{ children: fragment[0].children, type: KEYS.p }]
				: fragment,
	});
};

const PLACEHOLDERS: ReadonlyArray<
	readonly [string, (editor: SlateEditor) => string]
> = [
	["{block}", blockMarkdown],
	["{editor}", (editor) => serializeMd(editor)],
	["{selection}", selectionMarkdown],
];

const replaceFirst = (text: string, search: string, value: string) => {
	const index = text.indexOf(search);
	return index === -1
		? text
		: text.slice(0, index) + value + text.slice(index + search.length);
};

/**
 * Plate 49 prompt rendering: `{prompt}` is inserted first, then the first occurrence of
 * each placeholder (in the template or the prompt) becomes markdown of the editor.
 * Values are inserted literally and never rescanned, so `$$`, `$&` or a literal
 * `{editor}` in the document reach the model unchanged.
 */
export const renderEditorPrompt = (
	editor: SlateEditor,
	template: string,
	prompt: string,
) => {
	const text = replaceFirst(template, "{prompt}", prompt);
	const hits = PLACEHOLDERS.map(([placeholder, render]) => ({
		index: text.indexOf(placeholder),
		placeholder,
		render,
	}))
		.filter(({ index }) => index !== -1)
		.sort((left, right) => left.index - right.index);

	let rendered = "";
	let cursor = 0;
	for (const { index, placeholder, render } of hits) {
		rendered += text.slice(cursor, index) + render(editor);
		cursor = index + placeholder.length;
	}
	return rendered + text.slice(cursor);
};

const pickTemplate = ({ isBlockSelecting, isSelecting }: EditorPromptParams) =>
	isBlockSelecting
		? PROMPT_TEMPLATES.userBlockSelecting
		: isSelecting
			? PROMPT_TEMPLATES.userSelecting
			: PROMPT_TEMPLATES.userDefault;

/** Wraps a menu prompt in the template matching the selection state at submit time. */
export const withEditorTemplate =
	(prompt: EditorPrompt): EditorPrompt =>
	(params) =>
		renderEditorPrompt(
			params.editor,
			pickTemplate(params),
			getEditorPrompt(params.editor, { prompt }),
		);

/**
 * Submits a templated prompt and reports whether a request was sent. `generate` keeps
 * Insert below and the preview on the AI output.
 */
export const submitEditorAI = (
	editor: PlateEditor,
	prompt: EditorPrompt,
	{ mode }: { mode?: AIMode } = {},
) => {
	if (!prompt) return false;
	editor.getApi(AIChatPlugin).aiChat.submit("", {
		mode,
		prompt: withEditorTemplate(prompt),
		toolName: "generate",
	});
	return true;
};
