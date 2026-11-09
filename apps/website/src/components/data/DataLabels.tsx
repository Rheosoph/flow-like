import { useState } from "react";
import { LuFingerprint, LuTable2 } from "react-icons/lu";
import {
	DATA_CLASSIFICATIONS,
	type DataClassification,
	LEVEL_COPY,
	PII_COPY,
	SensitivityBadges,
	combineDataSensitivity,
} from "./sensitivity";

interface Column {
	name: string;
	type: string;
	level: DataClassification;
	pii: boolean;
}

const INITIAL: readonly Column[] = [
	{ name: "id", type: "int64", level: "internal", pii: false },
	{ name: "name", type: "utf8", level: "confidential", pii: true },
	{ name: "email", type: "utf8", level: "confidential", pii: true },
	{ name: "country", type: "utf8", level: "public", pii: false },
	{ name: "segment", type: "utf8", level: "internal", pii: false },
	{ name: "credit_limit", type: "float64", level: "restricted", pii: false },
];

export default function DataLabels() {
	const [columns, setColumns] = useState<readonly Column[]>(INITIAL);
	const table = combineDataSensitivity(
		...columns.map((column) => ({
			classification: column.level,
			pii: column.pii,
		})),
	);
	const tableLevel = table.classification ?? "public";

	const update = (name: string, change: Partial<Column>) =>
		setColumns(
			columns.map((column) =>
				column.name === name ? { ...column, ...change } : column,
			),
		);

	return (
		<div className="dt-labels-demo">
			<div className="dt-labels-table">
				<header>
					<LuTable2 aria-hidden="true" />
					<b>customers</b>
					<small>Data Studio · schema</small>
				</header>
				<div className="dt-labels-head" aria-hidden="true">
					<span>Column</span>
					<span>Type</span>
					<span>Sensitivity</span>
					<span>Personal data</span>
				</div>
				<ul>
					{columns.map((column) => (
						<li key={column.name}>
							<code>{column.name}</code>
							<span className="dt-schema-type">{column.type}</span>
							<label className="dt-level-select" data-level={column.level}>
								<span className="sr-only">Sensitivity of {column.name}</span>
								<i />
								<select
									value={column.level}
									onChange={(event) =>
										update(column.name, {
											level: event.target.value as DataClassification,
										})
									}
								>
									{DATA_CLASSIFICATIONS.map((level) => (
										<option key={level} value={level}>
											{LEVEL_COPY[level].label}
										</option>
									))}
								</select>
							</label>
							<label className="dt-pii-switch">
								<input
									type="checkbox"
									checked={column.pii}
									onChange={(event) =>
										update(column.name, { pii: event.target.checked })
									}
								/>
								<span aria-hidden="true">
									<LuFingerprint />
								</span>
								<span className="sr-only">
									{PII_COPY.label}: {column.name}
								</span>
							</label>
						</li>
					))}
				</ul>
			</div>
			<aside className="dt-labels-side" aria-live="polite">
				<span className="dt-labels-kicker">Table label</span>
				<div className="dt-labels-result">
					<SensitivityBadges value={table} />
				</div>
				<p className="dt-labels-hint">{LEVEL_COPY[tableLevel].hint}</p>
				<p className="dt-labels-rule">
					A table carries the strictest label of its columns, and personal data
					if any column holds it.
				</p>
				<div className="dt-labels-preview">
					<span className="dt-labels-kicker">On the source card</span>
					<div className="dt-labels-card">
						<LuTable2 aria-hidden="true" />
						<div>
							<b>customers</b>
							<small>{columns.length} columns · 12,480 rows</small>
						</div>
						<SensitivityBadges value={table} />
					</div>
				</div>
				<p className="dt-labels-meta">
					Stored with the schema as column metadata:{" "}
					<code>flow-like:classification</code>, <code>flow-like:pii</code>.
				</p>
			</aside>
		</div>
	);
}
