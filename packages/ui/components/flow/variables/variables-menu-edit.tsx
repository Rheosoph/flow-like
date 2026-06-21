import { useEffect, useState } from "react";
import { IValueType } from "../../../lib/schema/flow/pin";
import {
	type IVariable,
	IVariableType,
} from "../../../lib/schema/flow/variable";
import { BoolArrayVariable } from "./bool-array-variable";
import { BoolSetVariable } from "./bool-set-variable";
import { BoolVariable } from "./bool-variable";
import { DateArrayVariable } from "./date-array-variable";
import { DateSetVariable } from "./date-set-variable";
import { DateVariable } from "./date-variable";
import { FloatArrayVariable } from "./float-array-variable";
import { FloatSetVariable } from "./float-set-variable";
import { FloatVariable } from "./float-variable";
import { GenericVariable } from "./generic-variable";
import { IntegerArrayVariable } from "./integer-array-variable";
import { IntegerSetVariable } from "./integer-set-variable";
import { IntegerVariable } from "./integer-variable";
import { PathbufArrayVariable } from "./pathbuf-array-variable";
import { PathbufSetVariable } from "./pathbuf-set-variable";
import { PathbufVariable } from "./pathbuf-variable";
import { StringArrayVariable } from "./string-array-variable";
import { StringSetVariable } from "./string-set-variable";
import { StringVariable } from "./string-variable";
import { StructVariable } from "./struct-variable";

export function VariablesMenuEdit({
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
	const [intermediateValue, setIntermediateValue] = useState(variable);

	useEffect(() => {
		if (intermediateValue === variable) return;
		updateVariable(intermediateValue);
	}, [intermediateValue]);

	if (
		variable.data_type === IVariableType.String &&
		variable.value_type === IValueType.Normal
	) {
		return (
			<StringVariable
				disabled={disabled}
				variable={intermediateValue}
				onChange={setIntermediateValue}
			/>
		);
	}

	if (
		variable.data_type === IVariableType.String &&
		variable.value_type === IValueType.Array
	) {
		return (
			<StringArrayVariable
				disabled={disabled}
				variable={intermediateValue}
				onChange={setIntermediateValue}
			/>
		);
	}

	if (
		variable.data_type === IVariableType.String &&
		variable.value_type === IValueType.HashSet
	) {
		return (
			<StringSetVariable
				disabled={disabled}
				variable={intermediateValue}
				onChange={setIntermediateValue}
			/>
		);
	}

	if (
		variable.data_type === IVariableType.Boolean &&
		variable.value_type === IValueType.Normal
	) {
		return (
			<BoolVariable
				disabled={disabled}
				variable={intermediateValue}
				onChange={setIntermediateValue}
			/>
		);
	}

	if (
		variable.data_type === IVariableType.Boolean &&
		variable.value_type === IValueType.Array
	) {
		return (
			<BoolArrayVariable
				disabled={disabled}
				variable={intermediateValue}
				onChange={setIntermediateValue}
			/>
		);
	}

	if (
		variable.data_type === IVariableType.Boolean &&
		variable.value_type === IValueType.HashSet
	) {
		return (
			<BoolSetVariable
				disabled={disabled}
				variable={intermediateValue}
				onChange={setIntermediateValue}
			/>
		);
	}

	if (
		variable.data_type === IVariableType.Date &&
		variable.value_type === IValueType.Normal
	) {
		return (
			<DateVariable
				disabled={disabled}
				variable={intermediateValue}
				onChange={setIntermediateValue}
			/>
		);
	}

	if (
		variable.data_type === IVariableType.Date &&
		variable.value_type === IValueType.Array
	) {
		return (
			<DateArrayVariable
				disabled={disabled}
				variable={intermediateValue}
				onChange={setIntermediateValue}
			/>
		);
	}

	if (
		variable.data_type === IVariableType.Date &&
		variable.value_type === IValueType.HashSet
	) {
		return (
			<DateSetVariable
				disabled={disabled}
				variable={intermediateValue}
				onChange={setIntermediateValue}
			/>
		);
	}

	if (
		variable.data_type === IVariableType.Float &&
		variable.value_type === IValueType.Normal
	) {
		return (
			<FloatVariable
				disabled={disabled}
				variable={intermediateValue}
				onChange={setIntermediateValue}
			/>
		);
	}

	if (
		variable.data_type === IVariableType.Float &&
		variable.value_type === IValueType.Array
	) {
		return (
			<FloatArrayVariable
				disabled={disabled}
				variable={intermediateValue}
				onChange={setIntermediateValue}
			/>
		);
	}

	if (
		variable.data_type === IVariableType.Float &&
		variable.value_type === IValueType.HashSet
	) {
		return (
			<FloatSetVariable
				disabled={disabled}
				variable={intermediateValue}
				onChange={setIntermediateValue}
			/>
		);
	}

	if (
		variable.data_type === IVariableType.Integer &&
		variable.value_type === IValueType.Normal
	) {
		return (
			<IntegerVariable
				disabled={disabled}
				variable={intermediateValue}
				onChange={setIntermediateValue}
			/>
		);
	}

	if (
		variable.data_type === IVariableType.Integer &&
		variable.value_type === IValueType.Array
	) {
		return (
			<IntegerArrayVariable
				disabled={disabled}
				variable={intermediateValue}
				onChange={setIntermediateValue}
			/>
		);
	}

	if (
		variable.data_type === IVariableType.Integer &&
		variable.value_type === IValueType.HashSet
	) {
		return (
			<IntegerSetVariable
				disabled={disabled}
				variable={intermediateValue}
				onChange={setIntermediateValue}
			/>
		);
	}

	if (variable.data_type === IVariableType.Generic) {
		return (
			<GenericVariable
				disabled={disabled}
				variable={intermediateValue}
				onChange={setIntermediateValue}
			/>
		);
	}

	if (
		variable.data_type === IVariableType.PathBuf &&
		variable.value_type === IValueType.Normal
	) {
		return (
			<PathbufVariable
				disabled={disabled}
				variable={intermediateValue}
				onChange={setIntermediateValue}
			/>
		);
	}

	if (
		variable.data_type === IVariableType.PathBuf &&
		variable.value_type === IValueType.Array
	) {
		return (
			<PathbufArrayVariable
				disabled={disabled}
				variable={intermediateValue}
				onChange={setIntermediateValue}
			/>
		);
	}

	if (
		variable.data_type === IVariableType.PathBuf &&
		variable.value_type === IValueType.HashSet
	) {
		return (
			<PathbufSetVariable
				disabled={disabled}
				variable={intermediateValue}
				onChange={setIntermediateValue}
			/>
		);
	}

	if (
		variable.data_type === IVariableType.Struct &&
		variable.value_type === IValueType.Normal
	) {
		return (
			<StructVariable
				disabled={disabled}
				variable={intermediateValue}
				onChange={setIntermediateValue}
				refs={refs}
			/>
		);
	}

	return null;
}
