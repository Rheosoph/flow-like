"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Suspense,
	lazy,
	useEffect,
	useId,
	useMemo,
	useRef,
	useState,
} from "react";
import { parseGeometryText } from "../../../lib/geometry";
import { IValueType } from "../../../lib/schema/flow/pin";
import { Label } from "../../ui/label";
import { Textarea } from "../../ui/textarea";

const GeometryPreview = lazy(() => import("../../ui/geometry-preview"));

export interface GeometryFieldProps {
	value: unknown;
	onChange: (value: unknown, valid: boolean) => void;
	schema?: string | null;
	refs?: Record<string, string>;
	valueType?: IValueType;
	disabled?: boolean;
	allowUnset?: boolean;
	preview?: boolean;
}

/** Raw GeoJSON text editing; the visible draft survives while it is invalid. */
export function GeometryJsonField({
	value,
	onChange,
	schema,
	refs,
	valueType = IValueType.Normal,
	disabled,
	allowUnset = true,
	preview = true,
}: Readonly<GeometryFieldProps>) {
	const { t } = useTranslation("flow");
	const id = useId();
	const [text, setText] = useState(() =>
		value == null ? "" : JSON.stringify(value, null, 2),
	);
	const [focused, setFocused] = useState(false);
	const previousValue = useRef(value);
	useEffect(() => {
		if (!focused && previousValue.current !== value) {
			setText(value == null ? "" : JSON.stringify(value, null, 2));
		}
		previousValue.current = value;
	}, [value, focused]);
	const parsed = useMemo(() => {
		try {
			return {
				value: parseGeometryText(text, { schema, refs, valueType, allowUnset }),
				error: null,
			};
		} catch (e) {
			return { value: null, error: e instanceof Error ? e.message : String(e) };
		}
	}, [text, schema, refs, valueType, allowUnset]);
	return (
		<div className="grid w-full gap-2">
			<Label htmlFor={id}>{t("geometryGeoJson", "GeoJSON geometry")}</Label>
			<p className="text-xs text-muted-foreground">
				{allowUnset
					? t(
							"geometryCoordinateOrder",
							"WGS 84 coordinates: longitude, latitude in degrees. Leave empty to unset.",
						)
					: t(
							"geometryRequiredCoordinateOrder",
							"WGS 84 coordinates: longitude, latitude in degrees. A geometry is required.",
						)}
			</p>
			<Textarea
				id={id}
				disabled={disabled}
				value={text}
				rows={8}
				className="font-mono text-xs"
				spellCheck={false}
				aria-invalid={Boolean(parsed.error)}
				aria-describedby={parsed.error ? `${id}-error` : undefined}
				onFocus={() => setFocused(true)}
				onBlur={() => setFocused(false)}
				onChange={(event) => {
					const next = event.target.value;
					setText(next);
					try {
						onChange(
							parseGeometryText(next, { schema, refs, valueType, allowUnset }),
							true,
						);
					} catch {
						onChange(undefined, false);
					}
				}}
			/>
			{parsed.error && (
				<p id={`${id}-error`} role="alert" className="text-xs text-destructive">
					{parsed.error}
				</p>
			)}
			{preview && !parsed.error && parsed.value != null && (
				<Suspense fallback={null}>
					<GeometryPreview value={parsed.value} valueType={valueType} />
				</Suspense>
			)}
		</div>
	);
}
