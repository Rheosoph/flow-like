"use client";

import { Textarea, WidgetSchemaListEditor } from "@flow-like/flow-like-ui";
import type { JsonSchema } from "@flow-like/widget-sdk";
import { useMemo } from "react";
import type { WidgetPropDraftValue } from "../../../lib/widget-props-form";
import {
	homogeneousArrayItemSchema,
	parseWidgetListDraft,
	serializeWidgetList,
} from "../../../lib/widget-schema-form";

interface SchemaListFieldProps {
	fieldName: string;
	id: string;
	labelledBy: string;
	schema: JsonSchema;
	value: WidgetPropDraftValue;
	disabled: boolean;
	describedBy?: string;
	onChange: (value: WidgetPropDraftValue) => void;
}

/** Adapts Test Widget's editable JSON-string draft to the shared value editor. */
export function SchemaListField({
	fieldName,
	id,
	labelledBy,
	schema,
	value,
	disabled,
	describedBy,
	onChange,
}: SchemaListFieldProps) {
	const itemSchema = homogeneousArrayItemSchema(schema);
	const parsed = useMemo(() => parseWidgetListDraft(value), [value]);

	if (!itemSchema || parsed.items === null) {
		return (
			<div className="space-y-2">
				<Textarea
					id={id}
					value={typeof value === "string" ? value : ""}
					onChange={(event) => onChange(event.target.value)}
					disabled={disabled}
					rows={5}
					spellCheck={false}
					aria-labelledby={labelledBy}
					aria-describedby={describedBy}
					className="font-mono text-xs"
				/>
				{parsed.error && (
					<p className="text-xs text-destructive">{parsed.error}</p>
				)}
			</div>
		);
	}

	return (
		<WidgetSchemaListEditor
			fieldName={fieldName}
			id={id}
			labelledBy={labelledBy}
			schema={schema}
			value={parsed.items}
			disabled={disabled}
			describedBy={describedBy}
			onChange={(items) => onChange(serializeWidgetList(items))}
		/>
	);
}
