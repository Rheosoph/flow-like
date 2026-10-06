import type { TFunction } from "i18next";
import type {
	FieldKind,
	FieldProblem,
	FieldProblemCode,
	ViewerHabits,
	WorkbenchField,
} from "../contracts";
import { dateExample } from "../model/dates";
import { formatBytes } from "../run/format";

/*
 * The sentence under a control for each problem the model reports (SURFACE §6, spec M2, M4, M6, F, S4).
 * Literal keys only (PLAN §9): one arrow function per code and per kind, each with one `t()` call.
 */

type InterfacesT = TFunction<"interfaces">;

export interface ProblemContext {
	readonly t: InterfacesT;
	readonly field: WorkbenchField;
	readonly viewer: Pick<ViewerHabits, "dateLocale" | "decimalSign">;
	readonly problem: FieldProblem;
}

type Copy = (context: ProblemContext) => string;

/** "Invoice" → "invoice", "API key" and "URL" stay: only a capitalised word is lowered. */
export function lowerLabel(label: string): string {
	return /^[A-Z][a-z]/.test(label)
		? label.charAt(0).toLowerCase() + label.slice(1)
		: label;
}

const label = (context: ProblemContext) => context.field.label;
const fileName = (context: ProblemContext) => context.problem.fileName ?? "";

const enterThe: Copy = ({ t, field }) =>
	t("interfaces:workbench.field.problem.requiredText", "Enter the {{label}}.", {
		label: lowerLabel(field.label),
	});

/** A label that already names the file ("Invoice file", "File to scan") needs no second "file" in the sentence. */
const NAMES_A_FILE = /\bfiles?\b/i;

const addTheFile: Copy = ({ t, field }) =>
	t(
		"interfaces:workbench.field.problem.requiredFile",
		"Add the {{label}} file.",
		{ label: lowerLabel(field.label) },
	);

const addTheNamedFile: Copy = ({ t, field }) =>
	t(
		"interfaces:workbench.field.problem.requiredFileNamed",
		"Add the {{label}}.",
		{ label: lowerLabel(field.label) },
	);

const requiredFile: Copy = (context) =>
	(NAMES_A_FILE.test(context.field.label) ? addTheNamedFile : addTheFile)(
		context,
	);

const REQUIRED: Readonly<Record<FieldKind, Copy>> = {
	file: requiredFile,
	files: ({ t }) =>
		t(
			"interfaces:workbench.field.problem.requiredFiles",
			"Add at least one file.",
		),
	chips: ({ t, field }) =>
		t("interfaces:workbench.field.problem.requiredList", "Add the {{label}}.", {
			label: lowerLabel(field.label),
		}),
	date: ({ t }) =>
		t("interfaces:workbench.field.problem.requiredDate", "Choose a date."),
	choice: ({ t }) =>
		t("interfaces:workbench.field.problem.requiredChoice", "Choose one."),
	text: enterThe,
	number: enterThe,
	json: enterThe,
	group: enterThe,
	pairs: enterThe,
	bool: enterThe,
	unsupported: enterThe,
};

const withheld: Copy = ({ t, field }) =>
	t(
		"interfaces:workbench.field.problem.requiredWithheld",
		"Enter {{label}}. The app's own value is not shown here.",
		{ label: field.label },
	);

const required: Copy = (context) =>
	context.field.defaultOmitted
		? withheld(context)
		: REQUIRED[context.field.kind](context);

const asNumber = (value: number | undefined, sign: "." | ",") => {
	const text = String(value ?? "");
	return sign === "," ? text.replace(".", ",") : text;
};

const range: Copy = ({ t, problem, viewer }) =>
	t(
		"interfaces:workbench.field.problem.range",
		"Enter a number from {{min}} to {{max}}.",
		{
			min: asNumber(problem.min, viewer.decimalSign),
			max: asNumber(problem.max, viewer.decimalSign),
		},
	);

const date: Copy = ({ t, viewer }) => {
	const example = dateExample(viewer.dateLocale);
	return t(
		"interfaces:workbench.field.problem.date",
		"Enter a date such as {{short}} or {{full}}.",
		example,
	);
};

const UNIQUE: Readonly<Partial<Record<FieldKind, Copy>>> = {
	pairs: ({ t }) =>
		t(
			"interfaces:workbench.field.problem.uniqueNames",
			"Each name can appear once.",
		),
};

const uniqueEntries: Copy = ({ t }) =>
	t(
		"interfaces:workbench.field.problem.uniqueEntries",
		"Each entry can appear once.",
	);

const unique: Copy = (context) =>
	(UNIQUE[context.field.kind] ?? uniqueEntries)(context);

const ITEMS: Readonly<Partial<Record<FieldKind, Copy>>> = {
	pairs: ({ t }) =>
		t(
			"interfaces:workbench.field.problem.itemsRows",
			"Every row needs a name and a value that fits.",
		),
};

const itemsEntries: Copy = ({ t }) =>
	t(
		"interfaces:workbench.field.problem.itemsEntries",
		"One of the entries is not valid.",
	);

const items: Copy = (context) =>
	(ITEMS[context.field.kind] ?? itemsEntries)(context);

const pickAgain: Copy = (context) =>
	context.field.required
		? context.t(
				"interfaces:workbench.field.problem.pickAgain",
				"Pick {{name}} again.",
				{ name: fileName(context) },
			)
		: context.t(
				"interfaces:workbench.field.problem.pickAgainOptional",
				"Pick {{name}} again, or remove it.",
				{ name: fileName(context) },
			);

const COPY: Readonly<Record<FieldProblemCode, Copy>> = {
	required,
	integer: ({ t }) =>
		t(
			"interfaces:workbench.field.problem.integer",
			"Enter a whole number, digits only.",
		),
	number: ({ t }) =>
		t(
			"interfaces:workbench.field.problem.number",
			"Enter a number, digits only.",
		),
	noDecimals: ({ t }) =>
		t(
			"interfaces:workbench.field.problem.noDecimals",
			"Enter a whole number without decimals.",
		),
	range,
	date,
	json: ({ t }) =>
		t("interfaces:workbench.field.problem.json", "Enter valid JSON."),
	object: ({ t }) =>
		t("interfaces:workbench.field.problem.object", "Enter a JSON object."),
	array: ({ t }) =>
		t("interfaces:workbench.field.problem.array", "Enter a JSON list."),
	unique,
	items,
	option: ({ t }) =>
		t(
			"interfaces:workbench.field.problem.option",
			"Choose one of the options.",
		),
	fileSending: (context) =>
		context.t(
			"interfaces:workbench.field.problem.fileSending",
			"{{name}} is still sending.",
			{ name: fileName(context) },
		),
	fileFailed: (context) =>
		context.t(
			"interfaces:workbench.field.problem.fileFailed",
			"{{name}} was not sent. Try again or remove it.",
			{ name: fileName(context) },
		),
	fileTooLarge: (context) =>
		context.t(
			"interfaces:workbench.field.problem.fileTooLarge",
			"{{name}} is larger than {{limit}}, the most this page can send.",
			{
				name: fileName(context),
				limit: formatBytes(
					context.problem.limitBytes,
					context.viewer.decimalSign,
				),
			},
		),
	fileNotHere: (context) =>
		context.t(
			"interfaces:workbench.field.problem.fileNotHere",
			"{{label}} can't be sent from this page.",
			{ label: label(context) },
		),
	pickAgain,
	enterAgain: (context) =>
		context.t(
			"interfaces:workbench.field.problem.enterAgain",
			"Enter {{label}} again.",
			{ label: label(context) },
		),
	unsupported: (context) =>
		context.t(
			"interfaces:workbench.field.problem.unsupported",
			"{{label}} can't be filled in on this page.",
			{ label: label(context) },
		),
};

/** The message for one problem of one field or object property. */
export function problemText(context: ProblemContext): string {
	return COPY[context.problem.code](context);
}
