import type { IBackendState } from "../../../../state/backend-state";
import type { GraphOverlay } from "../../../../state/backend-state/graph-state";
import type { SavedQuery } from "../../../../state/backend-state/query-state";
import {
	HOME_DATA_SQL_MAX_SOURCES,
	extractHomeQueryParameters,
	homeDataColumns,
	homeDataSqlConfigError,
	homeOntologyColumns,
	normalizeHomeDataConfig,
} from "../../../home/home-data-query";
import type { IHomeLayout, IHomeWidget } from "../../../home/types";
import { validateHomeEmbedReference } from "./embed-validation";
import { issue, objectRecord, stringArg } from "./shared";
import type { HomeReferenceValidationOptions, HomeToolIssue } from "./types";
import { HOME_WIDGET_CONFIG_CONTRACTS } from "./widget-contracts/definitions";
import type {
	HomeWidgetConfigCondition,
	HomeWidgetConfigFieldContract,
} from "./widget-contracts/types";

function homeContractConditionMatches(
	condition: HomeWidgetConfigCondition | undefined,
	config: Record<string, unknown>,
	fields: Readonly<Record<string, HomeWidgetConfigFieldContract>>,
): boolean {
	if (!condition) return true;
	if (
		condition.all &&
		!condition.all.every((entry) =>
			homeContractConditionMatches(entry, config, fields),
		)
	)
		return false;
	if (!condition.field) return true;
	const actual = config[condition.field] ?? fields[condition.field]?.default;
	if (condition.equals !== undefined) return actual === condition.equals;
	if (condition.in) return condition.in.includes(actual as never);
	if (condition.not_equals !== undefined)
		return actual !== condition.not_equals;
	if (condition.not_in) return !condition.not_in.includes(actual as never);
	return true;
}

function referencedAppPaths(layout: IHomeLayout) {
	const references = new Map<string, string[]>();
	const add = (appId: string, path: string) => {
		if (!appId) return;
		const paths = references.get(appId) ?? [];
		paths.push(path);
		references.set(appId, paths);
	};
	layout.widgets.forEach((widget, index) => {
		const path = `$.widgets[${index}].config`;
		if (!Object.hasOwn(HOME_WIDGET_CONFIG_CONTRACTS, widget.type)) return;
		const contract =
			HOME_WIDGET_CONFIG_CONTRACTS[
				widget.type as keyof typeof HOME_WIDGET_CONFIG_CONTRACTS
			];
		if (widget.type === "data" && widget.config.sourceKind === "sql") {
			const sources = Array.isArray(widget.config.sources)
				? widget.config.sources.slice(0, HOME_DATA_SQL_MAX_SOURCES)
				: [];
			sources.forEach((source, sourceIndex) => {
				if (objectRecord(source) && typeof source.appId === "string")
					add(source.appId.trim(), `${path}.sources[${sourceIndex}].appId`);
			});
		}
		for (const [fieldName, field] of Object.entries(contract.fields)) {
			const reference = field.reference;
			if (
				!reference ||
				!homeContractConditionMatches(
					reference.when,
					widget.config,
					contract.fields,
				)
			)
				continue;
			if (reference.kind === "profile_app_id") {
				add(stringArg(widget.config, fieldName), `${path}.${fieldName}`);
			} else if (
				reference.kind === "profile_app_ids" &&
				Array.isArray(widget.config[fieldName])
			) {
				widget.config[fieldName].forEach((appId, appIndex) => {
					if (typeof appId === "string")
						add(appId.trim(), `${path}.${fieldName}[${appIndex}]`);
				});
			}
		}
	});
	return references;
}

function dataFieldReferences(widget: IHomeWidget) {
	const config = normalizeHomeDataConfig(widget.config);
	const fields: Array<[string, string]> = [];
	const add = (field: string, suffix: string) => {
		if (field) fields.push([field, suffix]);
	};
	if (config.dateRange !== "all") add(config.dateField, "dateField");
	if (config.mode === "records" && config.visualization !== "histogram") {
		// An explicit projection selects these fields even if the renderer does not use them.
		if (config.fields.length || config.visualization === "kanban")
			add(config.groupBy, "groupBy");
		const xy = ["graph", "scatter"].includes(config.visualization);
		if (
			config.fields.length ||
			xy ||
			["timeline", "recordcalendar"].includes(config.visualization)
		)
			add(config.xField, "xField");
		if (config.fields.length || xy) add(config.yField, "yField");
		if (config.sortBy !== "value" && config.sortBy !== "group")
			add(config.sortBy, "sortBy");
		config.fields.forEach((field, index) => add(field, `fields[${index}]`));
	} else {
		add(config.groupBy, "groupBy");
		add(config.seriesBy, "seriesBy");
		if (config.visualization === "boxplot") add(config.yField, "yField");
		else {
			config.measures.forEach((measure, index) => {
				if (measure.aggregation !== "count")
					add(measure.field, `measures[${index}].field`);
			});
		}
	}
	config.filters.forEach((filter, index) =>
		add(filter.field, `filters[${index}].field`),
	);
	return fields;
}

function validateHomeSqlWidget(widget: IHomeWidget, path: string) {
	const issues: HomeToolIssue[] = [];
	const config = normalizeHomeDataConfig(widget.config);
	const problem = config.sources.length ? homeDataSqlConfigError(config) : null;
	if (problem)
		issues.push(issue("error", "home_sql_config_invalid", path, problem));
	for (const parameter of extractHomeQueryParameters(config.sql)) {
		if (parameter.startsWith("__home_")) {
			issues.push(
				issue(
					"error",
					"home_query_parameter_reserved",
					`${path}.sql`,
					`SQL uses reserved parameter '${parameter}'.`,
				),
			);
		} else if (!Object.hasOwn(config.queryParams, parameter)) {
			issues.push(
				issue(
					"error",
					"home_query_parameter_missing",
					`${path}.queryParams.${parameter}`,
					`SQL requires parameter '${parameter}'.`,
				),
			);
		}
	}
	if (dataFieldReferences(widget).length > 0) {
		issues.push(
			issue(
				"warning",
				"home_query_result_fields_unverified",
				path,
				"Cross-app SQL result fields cannot be verified before the query runs.",
			),
		);
	}
	return issues;
}

/** Validate app, event, table, ontology, and saved-query identities used by a canonical layout. */
export async function validateHomeLayoutReferences(
	backend: Pick<
		IBackendState,
		"dbState" | "eventState" | "graphState" | "queryState"
	>,
	layout: IHomeLayout,
	options: HomeReferenceValidationOptions,
): Promise<HomeToolIssue[]> {
	const issues: HomeToolIssue[] = [];
	const appPaths = referencedAppPaths(layout);
	for (const [appId, paths] of appPaths) {
		if (options.profileAppIds.has(appId)) continue;
		for (const path of paths) {
			issues.push(
				issue(
					"error",
					"home_app_not_in_profile",
					path,
					`App '${appId}' is not available in the current profile.`,
				),
			);
		}
	}

	const eventsByApp = new Map<
		string,
		Promise<Awaited<ReturnType<IBackendState["eventState"]["getEvents"]>>>
	>();
	const tablesByScope = new Map<string, Promise<string[]>>();
	const ontologiesByScope = new Map<string, Promise<GraphOverlay[]>>();
	const queriesByScope = new Map<string, Promise<SavedQuery[]>>();
	const getEvents = (appId: string) => {
		let pending = eventsByApp.get(appId);
		if (!pending) {
			pending = backend.eventState.getEvents(appId);
			eventsByApp.set(appId, pending);
		}
		return pending;
	};

	for (const [index, widget] of layout.widgets.entries()) {
		const path = `$.widgets[${index}].config`;
		const appId = stringArg(widget.config, "appId");
		if (
			widget.type === "app-embed" &&
			appId &&
			options.profileAppIds.has(appId)
		) {
			try {
				const events = await getEvents(appId);
				issues.push(...validateHomeEmbedReference(widget.config, events, path));
			} catch {
				issues.push(
					issue(
						"warning",
						"home_app_interfaces_unreadable",
						path,
						`App '${appId}' interface metadata could not be checked.`,
					),
				);
			}
		}

		if (widget.type === "data" && widget.config.sourceKind === "sql") {
			issues.push(...validateHomeSqlWidget(widget, path));
			continue;
		}
		if (widget.type !== "data" || !appId || !options.profileAppIds.has(appId)) {
			continue;
		}
		const config = normalizeHomeDataConfig(widget.config);
		const personal = config.scope === "personal";
		const cacheKey = `${appId}:${config.scope}`;
		try {
			let knownColumns: Set<string> | undefined;
			if (config.sourceKind === "table") {
				let pending = tablesByScope.get(cacheKey);
				if (!pending) {
					pending = personal
						? backend.dbState.listTablesUser(appId)
						: backend.dbState.listTables(appId);
					tablesByScope.set(cacheKey, pending);
				}
				const tables = await pending;
				if (!tables.includes(config.table)) {
					issues.push(
						issue(
							"error",
							"home_table_missing",
							`${path}.table`,
							`Table '${config.table}' is not available in ${config.scope} data.`,
						),
					);
					continue;
				}
				const schema = await backend.dbState.getSchema(
					appId,
					config.table,
					personal,
				);
				knownColumns = new Set(
					homeDataColumns(schema).map((column) => column.name),
				);
			} else if (config.sourceKind === "ontology") {
				let pending = ontologiesByScope.get(cacheKey);
				if (!pending) {
					pending = backend.graphState.listOverlays(appId, personal);
					ontologiesByScope.set(cacheKey, pending);
				}
				const overlays = await pending;
				const overlay = overlays.find(
					(entry) => entry.id === config.ontologyId,
				);
				if (!overlay) {
					issues.push(
						issue(
							"error",
							"home_ontology_missing",
							`${path}.ontologyId`,
							`Ontology '${config.ontologyId}' is not available in ${config.scope} data.`,
						),
					);
					continue;
				}
				const objectType = overlay.nodes.find(
					(node) => node.label === config.objectType,
				);
				if (!objectType) {
					issues.push(
						issue(
							"error",
							"home_ontology_object_missing",
							`${path}.objectType`,
							`Object type '${config.objectType}' is not part of ontology '${config.ontologyId}'.`,
						),
					);
					continue;
				}
				knownColumns = new Set(
					homeOntologyColumns(overlay, config.objectType).map(
						(column) => column.name,
					),
				);
			} else {
				let pending = queriesByScope.get(cacheKey);
				if (!pending) {
					pending = backend.queryState.listSavedQueries(appId, personal);
					queriesByScope.set(cacheKey, pending);
				}
				const queries = await pending;
				const query = queries.find((entry) => entry.id === config.queryId);
				if (!query) {
					issues.push(
						issue(
							"error",
							"home_query_missing",
							`${path}.queryId`,
							`Saved query '${config.queryId}' is not available in ${config.scope} data.`,
						),
					);
					continue;
				}
				const schemaProperties = objectRecord(query.param_schema?.properties)
					? query.param_schema.properties
					: undefined;
				for (const parameter of extractHomeQueryParameters(query.sql)) {
					if (parameter.startsWith("__home_")) {
						issues.push(
							issue(
								"error",
								"home_query_parameter_reserved",
								`${path}.queryId`,
								`Saved query '${query.name}' uses reserved parameter '${parameter}'.`,
							),
						);
						continue;
					}
					if (!Object.hasOwn(config.queryParams, parameter)) {
						issues.push(
							issue(
								"error",
								"home_query_parameter_missing",
								`${path}.queryParams.${parameter}`,
								`Saved query '${query.name}' requires parameter '${parameter}'.`,
							),
						);
						continue;
					}
					const value = config.queryParams[parameter];
					if (value === "$viewer.id") continue;
					const parameterSchema = objectRecord(schemaProperties?.[parameter])
						? schemaProperties[parameter]
						: undefined;
					const expectedType = parameterSchema?.type;
					const matchesType =
						expectedType === "string"
							? typeof value === "string"
							: expectedType === "number"
								? typeof value === "number" && Number.isFinite(value)
								: expectedType === "integer"
									? typeof value === "number" &&
										Number.isFinite(value) &&
										Number.isInteger(value)
									: expectedType === "boolean"
										? typeof value === "boolean"
										: undefined;
					if (matchesType === false) {
						issues.push(
							issue(
								"error",
								"home_query_parameter_type_invalid",
								`${path}.queryParams.${parameter}`,
								`Parameter '${parameter}' must be a ${expectedType}.`,
							),
						);
					} else if (matchesType === undefined) {
						issues.push(
							issue(
								"warning",
								"home_query_parameter_schema_unavailable",
								`${path}.queryParams.${parameter}`,
								`Parameter '${parameter}' has no supported type metadata, so its fixed value could not be type-checked.`,
							),
						);
					}
				}
				if (dataFieldReferences(widget).length > 0) {
					issues.push(
						issue(
							"warning",
							"home_query_result_fields_unverified",
							path,
							"Saved-query result fields are not exposed by its metadata and could not be verified.",
						),
					);
				}
			}
			if (knownColumns?.size) {
				for (const [field, suffix] of dataFieldReferences(widget)) {
					if (knownColumns.has(field)) continue;
					issues.push(
						issue(
							"error",
							"home_data_field_missing",
							`${path}.${suffix}`,
							`Field '${field}' is not available in the selected source.`,
						),
					);
				}
			}
		} catch {
			issues.push(
				issue(
					"warning",
					"home_data_source_unreadable",
					path,
					"This data source could not be checked. Its identifiers were preserved.",
				),
			);
		}
	}
	return issues;
}
