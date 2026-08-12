import type { ContractInput, JsonSchema } from "./contract";

export interface ValidationResult {
	valid: boolean;
	errors: string[];
}

const VALID: ValidationResult = { valid: true, errors: [] };

function deepEqual(a: unknown, b: unknown): boolean {
	if (a === b) return true;
	if (typeof a !== typeof b) return false;
	if (typeof a !== "object" || a === null || b === null) return false;
	if (Array.isArray(a) !== Array.isArray(b)) return false;
	if (Array.isArray(a) && Array.isArray(b)) {
		if (a.length !== b.length) return false;
		return a.every((item, index) => deepEqual(item, b[index]));
	}
	const aRecord = a as Record<string, unknown>;
	const bRecord = b as Record<string, unknown>;
	const aKeys = Object.keys(aRecord);
	const bKeys = Object.keys(bRecord);
	if (aKeys.length !== bKeys.length) return false;
	return aKeys.every((key) => deepEqual(aRecord[key], bRecord[key]));
}

function matchesType(type: string, value: unknown): boolean {
	switch (type) {
		case "string":
			return typeof value === "string";
		case "number":
			return typeof value === "number";
		case "integer":
			return typeof value === "number" && Number.isInteger(value);
		case "boolean":
			return typeof value === "boolean";
		case "null":
			return value === null;
		case "array":
			return Array.isArray(value);
		case "object":
			return (
				typeof value === "object" && value !== null && !Array.isArray(value)
			);
		default:
			return true;
	}
}

function describe(value: unknown): string {
	if (value === null) return "null";
	if (Array.isArray(value)) return "array";
	return typeof value;
}

function checkNumericBounds(
	schema: JsonSchema,
	value: number,
	path: string,
	errors: string[],
): void {
	const { minimum, maximum, exclusiveMinimum, exclusiveMaximum } = schema;
	if (typeof minimum === "number" && value < minimum) {
		errors.push(`${path}: ${value} is less than minimum ${minimum}`);
	}
	if (typeof maximum === "number" && value > maximum) {
		errors.push(`${path}: ${value} is greater than maximum ${maximum}`);
	}
	if (typeof exclusiveMinimum === "number" && value <= exclusiveMinimum) {
		errors.push(
			`${path}: ${value} is not greater than exclusiveMinimum ${exclusiveMinimum}`,
		);
	}
	if (typeof exclusiveMaximum === "number" && value >= exclusiveMaximum) {
		errors.push(
			`${path}: ${value} is not less than exclusiveMaximum ${exclusiveMaximum}`,
		);
	}
}

function checkString(
	schema: JsonSchema,
	value: string,
	path: string,
	errors: string[],
): void {
	const { minLength, maxLength, pattern } = schema;
	if (typeof minLength === "number" && value.length < minLength) {
		errors.push(`${path}: string shorter than minLength ${minLength}`);
	}
	if (typeof maxLength === "number" && value.length > maxLength) {
		errors.push(`${path}: string longer than maxLength ${maxLength}`);
	}
	if (typeof pattern === "string") {
		let regex: RegExp | null = null;
		try {
			regex = new RegExp(pattern);
		} catch {
			// Invalid pattern in the schema — cannot enforce, treat as passing.
		}
		if (regex && !regex.test(value)) {
			errors.push(`${path}: string does not match pattern ${pattern}`);
		}
	}
}

function checkArray(
	schema: JsonSchema,
	value: unknown[],
	path: string,
	errors: string[],
): void {
	const { minItems, maxItems, items } = schema;
	if (typeof minItems === "number" && value.length < minItems) {
		errors.push(`${path}: array has fewer than minItems ${minItems}`);
	}
	if (typeof maxItems === "number" && value.length > maxItems) {
		errors.push(`${path}: array has more than maxItems ${maxItems}`);
	}
	if (Array.isArray(items)) {
		items.forEach((itemSchema, index) => {
			if (index < value.length && isSchemaObject(itemSchema)) {
				validateAt(itemSchema, value[index], `${path}[${index}]`, errors);
			}
		});
	} else if (isSchemaObject(items)) {
		value.forEach((item, index) => {
			validateAt(items, item, `${path}[${index}]`, errors);
		});
	}
}

function checkObject(
	schema: JsonSchema,
	value: Record<string, unknown>,
	path: string,
	errors: string[],
): void {
	const { properties, required, additionalProperties } = schema;
	const props = isSchemaObject(properties) ? properties : undefined;

	if (Array.isArray(required)) {
		for (const key of required) {
			if (typeof key === "string" && !(key in value)) {
				errors.push(`${path}: missing required property "${key}"`);
			}
		}
	}

	if (props) {
		for (const [key, propSchema] of Object.entries(props)) {
			if (key in value && isSchemaObject(propSchema)) {
				validateAt(propSchema, value[key], `${path}.${key}`, errors);
			}
		}
	}

	if (additionalProperties === false) {
		for (const key of Object.keys(value)) {
			if (!props || !(key in props)) {
				errors.push(`${path}: unexpected additional property "${key}"`);
			}
		}
	} else if (isSchemaObject(additionalProperties)) {
		for (const key of Object.keys(value)) {
			if (!props || !(key in props)) {
				validateAt(additionalProperties, value[key], `${path}.${key}`, errors);
			}
		}
	}
}

function isSchemaObject(value: unknown): value is JsonSchema {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function validateAt(
	schema: JsonSchema,
	value: unknown,
	path: string,
	errors: string[],
): void {
	// Schemas are pre-inlined by the bundler; a leftover $ref cannot be
	// resolved here, so it is treated as passing.
	if ("$ref" in schema) return;

	const { type } = schema;
	if (typeof type === "string" && !matchesType(type, value)) {
		errors.push(`${path}: expected type ${type}, got ${describe(value)}`);
		return;
	}
	if (Array.isArray(type)) {
		const matched = type.some(
			(candidate) =>
				typeof candidate === "string" && matchesType(candidate, value),
		);
		if (!matched) {
			errors.push(
				`${path}: expected one of types [${type.join(", ")}], got ${describe(value)}`,
			);
			return;
		}
	}

	if ("enum" in schema) {
		const allowed = schema.enum;
		if (Array.isArray(allowed)) {
			if (!allowed.some((candidate) => deepEqual(candidate, value))) {
				errors.push(`${path}: value is not one of the allowed enum values`);
			}
		}
	}

	if ("const" in schema && !deepEqual(schema.const, value)) {
		errors.push(`${path}: value does not equal const`);
	}

	if (typeof value === "number")
		checkNumericBounds(schema, value, path, errors);
	if (typeof value === "string") checkString(schema, value, path, errors);
	if (Array.isArray(value)) checkArray(schema, value, path, errors);
	if (typeof value === "object" && value !== null && !Array.isArray(value)) {
		checkObject(schema, value as Record<string, unknown>, path, errors);
	}

	const { anyOf, oneOf, allOf } = schema;
	if (Array.isArray(anyOf)) {
		const passes = anyOf.some(
			(sub) => isSchemaObject(sub) && validateSchema(sub, value).valid,
		);
		if (!passes) errors.push(`${path}: value matches no schema in anyOf`);
	}
	if (Array.isArray(oneOf)) {
		const matches = oneOf.filter(
			(sub) => isSchemaObject(sub) && validateSchema(sub, value).valid,
		).length;
		if (matches !== 1) {
			errors.push(
				`${path}: value matches ${matches} schemas in oneOf, expected exactly 1`,
			);
		}
	}
	if (Array.isArray(allOf)) {
		allOf.forEach((sub, index) => {
			if (isSchemaObject(sub)) {
				const result = validateSchema(sub, value);
				if (!result.valid) {
					errors.push(`${path}: value fails allOf[${index}]`);
				}
			}
		});
	}
}

export function validateSchema(
	schema: JsonSchema | null | undefined,
	value: unknown,
): ValidationResult {
	if (schema === null || schema === undefined) return VALID;
	const errors: string[] = [];
	validateAt(schema, value, "$", errors);
	return { valid: errors.length === 0, errors };
}

export function validateInputValue(
	input: ContractInput,
	value: unknown,
): ValidationResult {
	if (value === undefined) {
		if (input.optional) return VALID;
		return { valid: false, errors: ["$: value is required"] };
	}

	const errors: string[] = [];
	switch (input.type) {
		case "string":
			if (typeof value !== "string") {
				errors.push(`$: expected string, got ${describe(value)}`);
			}
			break;
		case "number":
		case "integer": {
			const wantsInteger = input.type === "integer";
			if (
				typeof value !== "number" ||
				(wantsInteger && !Number.isInteger(value))
			) {
				errors.push(`$: expected ${input.type}, got ${describe(value)}`);
				break;
			}
			if (input.min !== undefined && value < input.min) {
				errors.push(`$: ${value} is less than min ${input.min}`);
			}
			if (input.max !== undefined && value > input.max) {
				errors.push(`$: ${value} is greater than max ${input.max}`);
			}
			break;
		}
		case "boolean":
			if (typeof value !== "boolean") {
				errors.push(`$: expected boolean, got ${describe(value)}`);
			}
			break;
		case "enum":
			if (typeof value !== "string" || !input.choices?.includes(value)) {
				errors.push(
					`$: value is not one of [${(input.choices ?? []).join(", ")}]`,
				);
			}
			break;
		case "json": {
			const result = validateSchema(input.schema, value);
			errors.push(...result.errors);
			break;
		}
	}
	return { valid: errors.length === 0, errors };
}
