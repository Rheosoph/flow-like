import { EyeIcon, EyeOffIcon } from "lucide-react";
import { useState } from "react";
import { IValueType } from "../../../lib/schema/flow/pin";
import {
	type IVariable,
	IVariableType,
} from "../../../lib/schema/flow/variable";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../lib/uint8";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { GenericVariable } from "./generic-variable";
import { VariablesMenuEdit } from "./variables-menu-edit";

/**
 * Resolve the initial value of a runtime variable from a saved value or its
 * board default. A plain Boolean with no value is seeded to `false` so the
 * rendered switch (which always shows a definite on/off state) matches the
 * stored value instead of being counted as "not configured".
 */
export function seedRuntimeVariable(
	variable: IVariable,
	existingBytes?: number[] | null,
): IVariable {
	let bytes = existingBytes ?? variable.default_value ?? null;
	if (
		(!bytes || bytes.length === 0) &&
		variable.value_type === IValueType.Normal &&
		variable.data_type === IVariableType.Boolean
	) {
		bytes = convertJsonToUint8Array(false) ?? null;
	}
	return { ...variable, default_value: bytes };
}

/**
 * Whether {@link VariablesMenuEdit} renders a dedicated editor for this
 * data-type/value-type combination. Byte and non-Normal Struct variables have
 * no typed editor and would otherwise render nothing.
 */
function hasTypedEditor(variable: IVariable): boolean {
	if (variable.value_type === IValueType.HashMap) return true;
	switch (variable.data_type) {
		case IVariableType.String:
		case IVariableType.Boolean:
		case IVariableType.Date:
		case IVariableType.Float:
		case IVariableType.Integer:
		case IVariableType.PathBuf:
		case IVariableType.Generic:
			return true;
		case IVariableType.Struct:
			return variable.value_type === IValueType.Normal;
		default:
			return false;
	}
}

/**
 * Whether the type-specific editor already masks its value when `secret` is
 * set (String/Integer/Float render a password field). Other editors show the
 * value in clear text, so secrets of those types need the masked fallback.
 */
function selfMasksSecret(variable: IVariable): boolean {
	if (variable.value_type !== IValueType.Normal) return false;
	return (
		variable.data_type === IVariableType.String ||
		variable.data_type === IVariableType.Integer ||
		variable.data_type === IVariableType.Float
	);
}

function decodeSecret(bytes: number[] | null | undefined): string {
	const decoded = parseUint8ArrayToJson(bytes);
	if (decoded === undefined || decoded === null) return "";
	return typeof decoded === "string" ? decoded : JSON.stringify(decoded);
}

function MaskedSecretInput({
	disabled,
	variable,
	updateVariable,
}: Readonly<{
	disabled?: boolean;
	variable: IVariable;
	updateVariable: (variable: IVariable) => Promise<void>;
}>) {
	const [reveal, setReveal] = useState(false);
	const value = decodeSecret(variable.default_value);

	return (
		<div className="relative w-full">
			<Input
				disabled={disabled}
				type={reveal ? "text" : "password"}
				autoComplete="off"
				spellCheck="false"
				autoCorrect="off"
				autoCapitalize="off"
				value={value}
				onChange={(e) => {
					void updateVariable({
						...variable,
						default_value: convertJsonToUint8Array(e.target.value) ?? null,
					});
				}}
				placeholder="Enter secret value..."
				className="pr-10 font-mono"
			/>
			<Button
				type="button"
				variant="ghost"
				size="icon"
				disabled={disabled}
				className="absolute right-1 top-1/2 -translate-y-1/2 h-7 w-7"
				onClick={() => setReveal((prev) => !prev)}
			>
				{reveal ? (
					<EyeOffIcon className="w-4 h-4" />
				) : (
					<EyeIcon className="w-4 h-4" />
				)}
			</Button>
		</div>
	);
}

/**
 * Editor for a single runtime-configured variable. Renders the type-correct
 * editor (path picker, date picker, number field, …) and falls back to a raw
 * JSON editor for uncommon types so the variable always stays configurable.
 * Secret variables whose editor does not mask its value use a masked input so
 * the value is never shown in clear text.
 */
export function RuntimeVariableEditor({
	disabled,
	variable,
	updateVariable,
	refs,
}: Readonly<{
	disabled?: boolean;
	variable: IVariable;
	updateVariable: (variable: IVariable) => Promise<void>;
	refs?: Record<string, string>;
}>) {
	if (variable.secret && !selfMasksSecret(variable)) {
		return (
			<MaskedSecretInput
				disabled={disabled}
				variable={variable}
				updateVariable={updateVariable}
			/>
		);
	}

	if (hasTypedEditor(variable)) {
		return (
			<VariablesMenuEdit
				disabled={disabled}
				variable={variable}
				updateVariable={updateVariable}
				refs={refs}
			/>
		);
	}

	return (
		<GenericVariable
			disabled={disabled}
			variable={variable}
			onChange={(next) => {
				void updateVariable(next);
			}}
		/>
	);
}
