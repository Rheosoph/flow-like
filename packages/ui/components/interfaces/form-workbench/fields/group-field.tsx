"use client";

import { cx } from "../../../settings/devices/primitives/tone";
import type { ControlProps } from "./bind";
import { DIFFERS_EDGE } from "./control-style";
import { FieldControl } from "./field-control";
import { fitsHalf } from "./fit";

/**
 * An object with known properties (SURFACE §4): a bordered group titled by the field's label, one control per
 * property, each with its own markers, recent values and date anchor. Properties that are short share a row.
 */
export function GroupField(props: ControlProps) {
	const { bind, ...rest } = props;
	const { field, layout } = rest;
	return (
		<fieldset
			aria-labelledby={bind.ids.label}
			className={cx(
				"m-0 grid min-w-0 gap-3 rounded-lg border border-border p-3",
				layout.touch ? "grid-cols-1" : "grid-cols-2",
				bind.markers.differs && DIFFERS_EDGE,
			)}
		>
			{field.props.map((prop) => {
				const half =
					!layout.touch && fitsHalf(prop, rest.markersFor(prop.key).optional);
				return (
					<div
						key={prop.key}
						className={cx("min-w-0", half ? "" : "col-span-full")}
					>
						<FieldControl {...rest} field={prop} blocked={false} />
					</div>
				);
			})}
		</fieldset>
	);
}
