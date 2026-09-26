"use client";

import {
	KEYS,
	type SlateEditor,
	type TextSubstitutionPattern,
	createSlatePlugin,
	createTextSubstitutionInputRule,
} from "platejs";

import { notInside } from "./input-rule-guards";

const smartQuotes: TextSubstitutionPattern[] = [
	{ format: ["“", "”"], match: '"' },
	{ format: ["‘", "’"], match: "'" },
];

const punctuation: TextSubstitutionPattern[] = [
	{ format: "—", match: "--" },
	{ format: "…", match: "..." },
	{ format: "»", match: ">>" },
	{ format: "«", match: "<<" },
];

const legal: TextSubstitutionPattern[] = [
	{ format: "™", match: ["(tm)", "(TM)"] },
	{ format: "®", match: ["(r)", "(R)"] },
	{ format: "©", match: ["(c)", "(C)"] },
];

const legalHtml: TextSubstitutionPattern[] = [
	{ format: "™", match: "&trade;" },
	{ format: "®", match: "&reg;" },
	{ format: "©", match: "&copy;" },
	{ format: "§", match: "&sect;" },
];

const arrows: TextSubstitutionPattern[] = [
	{ format: "→", match: "->" },
	{ format: "←", match: "<-" },
	{ format: "⇒", match: "=>" },
	{ format: "⇐", match: ["<=", "≤="] },
];

const comparisons: TextSubstitutionPattern[] = [
	{ format: "≯", match: "!>" },
	{ format: "≮", match: "!<" },
	{ format: "≥", match: ">=" },
	{ format: "≤", match: "<=" },
	{ format: "≱", match: "!>=" },
	{ format: "≰", match: "!<=" },
];

const equality: TextSubstitutionPattern[] = [
	{ format: "≠", match: "!=" },
	{ format: "≡", match: "==" },
	{ format: "≢", match: ["!==", "≠="] },
	{ format: "≈", match: "~=" },
	{ format: "≉", match: "!~=" },
];

// No "//" → "÷": it turned every typed URL into "https:÷…".
const operators: TextSubstitutionPattern[] = [
	{ format: "±", match: "+-" },
	{ format: "‰", match: "%%" },
	{ format: "‱", match: ["%%%", "‰%"] },
];

const fractions: TextSubstitutionPattern[] = [
	{ format: "½", match: "1/2" },
	{ format: "⅓", match: "1/3" },
	{ format: "¼", match: "1/4" },
	{ format: "⅕", match: "1/5" },
	{ format: "⅙", match: "1/6" },
	{ format: "⅐", match: "1/7" },
	{ format: "⅛", match: "1/8" },
	{ format: "⅑", match: "1/9" },
	{ format: "⅒", match: "1/10" },
	{ format: "⅔", match: "2/3" },
	{ format: "⅖", match: "2/5" },
	{ format: "¾", match: "3/4" },
	{ format: "⅗", match: "3/5" },
	{ format: "⅜", match: "3/8" },
	{ format: "⅘", match: "4/5" },
	{ format: "⅚", match: "5/6" },
	{ format: "⅝", match: "5/8" },
	{ format: "⅞", match: "7/8" },
];

const superscriptSymbols: TextSubstitutionPattern[] = [
	{ format: "°", match: "^o" },
	{ format: "⁺", match: "^+" },
	{ format: "⁻", match: "^-" },
];

const subscriptSymbols: TextSubstitutionPattern[] = [
	{ format: "₊", match: "~+" },
	{ format: "₋", match: "~-" },
];

const digitPatterns = (
	prefix: string,
	digits: string,
): TextSubstitutionPattern[] =>
	Array.from(digits, (format, digit) => ({
		format,
		match: `${prefix}${digit}`,
	}));

const patterns: TextSubstitutionPattern[] = [
	...smartQuotes,
	...punctuation,
	...legal,
	...legalHtml,
	...arrows,
	...comparisons,
	...equality,
	...operators,
	...fractions,
	...superscriptSymbols,
	...subscriptSymbols,
	...digitPatterns("^", "⁰¹²³⁴⁵⁶⁷⁸⁹"),
	...digitPatterns("~", "₀₁₂₃₄₅₆₇₈₉"),
];

const typedTextBySymbol = new Map<string, string>();
for (const { format, match } of patterns) {
	if (typeof format !== "string" || typedTextBySymbol.has(format)) continue;
	typedTextBySymbol.set(format, typeof match === "string" ? match : match[0]);
}

const typedTextBeforeCursor = (editor: SlateEditor) => {
	const { selection } = editor;
	if (!selection || !editor.api.isCollapsed()) return;
	const before = editor.api.before(selection, {
		distance: 1,
		unit: "character",
	});
	if (!before) return;
	return typedTextBySymbol.get(
		editor.api.string({ anchor: before, focus: selection.anchor }),
	);
};

/** Backspace right after a symbol restores the text that produced it. */
const AutoformatShortcutsPlugin = createSlatePlugin({
	key: "autoformatShortcuts",
	editOnly: true,
	inputRules: [
		createTextSubstitutionInputRule({
			enabled: notInside(KEYS.codeBlock),
			patterns,
		}),
	],
}).overrideEditor(({ editor, tf: { deleteBackward } }) => ({
	transforms: {
		deleteBackward(unit = "character") {
			const typedText =
				unit === "character" ? typedTextBeforeCursor(editor) : undefined;
			deleteBackward(unit);
			if (typedText !== undefined) editor.tf.insertText(typedText);
		},
	},
}));

export const AutoformatKit = [AutoformatShortcutsPlugin];
