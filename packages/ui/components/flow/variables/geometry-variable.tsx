"use client";

import { useTranslation } from "@flow-like/locales";
import { useId, useState } from "react";
import {
	GEOMETRY_KINDS,
	type GeometryKind,
	geometryKindFromSchema,
	geometryMarker,
} from "../../../lib/geometry";
import { IValueType } from "../../../lib/schema/flow/pin";
import type { IVariable } from "../../../lib/schema/flow/variable";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../lib/uint8";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { GeometryEditor } from "./geometry-editor";
import {
	type GeometryFieldProps,
	GeometryJsonField,
} from "./geometry-json-field";

export function GeometrySubtypeSelect({
	schema,
	refs,
	onChange,
	disabled,
}: Readonly<{
	schema?: string | null;
	refs?: Record<string, string>;
	onChange: (schema: string | null) => void;
	disabled?: boolean;
}>) {
	const { t } = useTranslation("flow");
	const id = useId();
	let kind = "any";
	let error: string | null = null;
	try {
		kind = geometryKindFromSchema(schema, refs) ?? "any";
	} catch (e) {
		error = String(e);
	}
	return (
		<div className="grid gap-2">
			<Label htmlFor={id}>{t("geometrySubtype", "Geometry subtype")}</Label>
			<Select
				value={kind}
				disabled={disabled}
				onValueChange={(kind) =>
					onChange(
						geometryMarker(kind === "any" ? null : (kind as GeometryKind)),
					)
				}
			>
				<SelectTrigger id={id}>
					<SelectValue />
				</SelectTrigger>
				<SelectContent>
					<SelectItem value="any">
						{t("anyGeometry", "Any geometry")}
					</SelectItem>
					{GEOMETRY_KINDS.map((kind) => (
						<SelectItem key={kind} value={kind}>
							{kind}
						</SelectItem>
					))}
				</SelectContent>
			</Select>
			{error && (
				<p role="alert" className="text-xs text-destructive">
					{error}
				</p>
			)}
		</div>
	);
}

/**
 * Single geometries get the map and coordinate editor; containers (Array,
 * HashSet, HashMap) and secrets that are still hidden fall back to GeoJSON text.
 */
export function GeometryValueInput({
	secret = false,
	...props
}: Readonly<GeometryFieldProps & { secret?: boolean }>) {
	const { t } = useTranslation("flow");
	const id = useId();
	const [revealed, setRevealed] = useState(false);
	const { value, valueType = IValueType.Normal, disabled } = props;
	const hidden = secret && !revealed;
	return (
		<div className="grid w-full gap-2">
			{secret && (
				<Button
					type="button"
					variant="outline"
					size="sm"
					className="w-fit"
					onClick={() => setRevealed(!revealed)}
				>
					{revealed
						? t("hideGeometryValue", "Hide geometry value")
						: t("showGeometryValue", "Show geometry value")}
				</Button>
			)}
			{hidden ? (
				<Input
					id={id}
					type="password"
					value={value == null ? "" : JSON.stringify(value)}
					readOnly
					disabled={disabled}
					aria-label={t("geometryGeoJson", "GeoJSON geometry")}
				/>
			) : valueType === IValueType.Normal ? (
				<GeometryEditor {...props} />
			) : (
				<GeometryJsonField {...props} valueType={valueType} />
			)}
		</div>
	);
}

export function GeometryVariable({
	variable,
	onChange,
	disabled,
	refs,
}: Readonly<{
	variable: IVariable;
	onChange: (variable: IVariable) => void;
	disabled?: boolean;
	refs?: Record<string, string>;
}>) {
	return (
		<GeometryValueInput
			disabled={disabled}
			schema={variable.schema}
			refs={refs}
			secret={variable.secret}
			valueType={variable.value_type}
			value={parseUint8ArrayToJson(variable.default_value)}
			onChange={(value, valid) => {
				if (valid)
					onChange({
						...variable,
						// JSON null explicitly clears secrets; absent bytes retain their saved value.
						default_value: convertJsonToUint8Array(value ?? null),
					});
			}}
		/>
	);
}
