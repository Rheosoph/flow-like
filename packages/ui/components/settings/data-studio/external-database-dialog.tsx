"use client";

import { useTranslation } from "@flow-like/locales";
import type { TFunction } from "i18next";
import { KeyRound, Loader2, Server } from "lucide-react";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useState,
} from "react";
import { toast } from "sonner";
import { apiErrorMessage } from "../../../lib/api-error";
import { getErrorMessage } from "../../../lib/error-message";
import { useBackend } from "../../../state/backend-state";
import type {
	ExternalDataSource,
	ExternalDataSourceDriver,
	UpsertExternalDataSourcePayload,
} from "../../../state/backend-state/data-source-state";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import {
	Dialog,
	DialogBody,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { Textarea } from "../../ui/textarea";

export const EXTERNAL_DATABASE_DRIVERS: readonly ExternalDataSourceDriver[] = [
	"postgres",
	"mysql",
	"clickhouse",
	"bigquery",
	"athena",
];

/** Drivers the query service reads today; the others are stored until it does. */
export const QUERYABLE_DRIVERS: ReadonlySet<ExternalDataSourceDriver> = new Set(
	["postgres", "mysql", "clickhouse"],
);

const ALIAS_PATTERN = /^[a-z_][a-z0-9_]{0,62}$/;
const RESERVED_ALIASES = new Set(["public", "information_schema"]);

type SecretMode = "keep" | "replace" | "clear";

interface FieldSpec {
	readonly key: string;
	readonly label: string;
	readonly kind: "text" | "port" | "select" | "password" | "multiline";
	readonly required: boolean;
	readonly placeholder?: string;
	readonly defaultValue?: string;
	readonly options?: readonly string[];
}

export interface DriverSpec {
	readonly label: string;
	readonly config: readonly FieldSpec[];
	readonly secret: readonly FieldSpec[];
}

export interface ExternalDatabaseDialogProps {
	readonly open: boolean;
	readonly onOpenChange: (open: boolean) => void;
	readonly appId: string;
	/** The source to edit; without it the dialog connects a new one. */
	readonly source?: ExternalDataSource | null;
	readonly onSaved: (source: ExternalDataSource) => void;
}

export function isValidAlias(alias: string): boolean {
	return ALIAS_PATTERN.test(alias) && !RESERVED_ALIASES.has(alias);
}

/** A schema name from a display name: `Sales EU` → `sales_eu`. */
export function suggestAlias(name: string): string {
	const slug = name
		.normalize("NFKD")
		.replace(/\p{M}/gu, "")
		.toLowerCase()
		.replace(/[^a-z0-9]+/g, "_")
		.replace(/^_+|_+$/g, "");
	if (!slug) return "";
	const alias = (/^[0-9]/.test(slug) ? `_${slug}` : slug).slice(0, 63);
	return RESERVED_ALIASES.has(alias) ? `${alias}_db` : alias;
}

function defaultsOf(fields: readonly FieldSpec[]): Record<string, string> {
	return Object.fromEntries(
		fields.map((field) => [field.key, field.defaultValue ?? ""]),
	);
}

function configValuesOf(source: ExternalDataSource): Record<string, string> {
	return Object.fromEntries(
		Object.entries(source.config ?? {}).map(([key, value]) => [
			key,
			value === null || value === undefined ? "" : String(value),
		]),
	);
}

export function ExternalDatabaseDialog({
	open,
	onOpenChange,
	appId,
	source,
	onSaved,
}: ExternalDatabaseDialogProps) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const specs = useMemo(() => driverSpecs(t), [t]);
	const editing = !!source;

	const [name, setName] = useState("");
	const [alias, setAlias] = useState("");
	const [aliasTouched, setAliasTouched] = useState(false);
	const [driver, setDriver] = useState<ExternalDataSourceDriver>("postgres");
	const [config, setConfig] = useState<Record<string, string>>({});
	const [secret, setSecret] = useState<Record<string, string>>({});
	const [secretMode, setSecretMode] = useState<SecretMode>("replace");
	const [showErrors, setShowErrors] = useState(false);
	const [submitting, setSubmitting] = useState(false);
	const [serverError, setServerError] = useState<string | null>(null);

	useEffect(() => {
		if (!open) return;
		const initialDriver = source?.driver ?? "postgres";
		setName(source?.name ?? "");
		setAlias(source?.alias ?? "");
		setAliasTouched(!!source);
		setDriver(initialDriver);
		setConfig({
			...defaultsOf(specs[initialDriver].config),
			...(source ? configValuesOf(source) : {}),
		});
		setSecret({});
		setSecretMode(
			source && source.secret_fields.length > 0 ? "keep" : "replace",
		);
		setShowErrors(false);
		setSubmitting(false);
		setServerError(null);
	}, [open, source, specs]);

	const spec = specs[driver];
	const storedFields = source?.secret_fields ?? [];
	const storedFit = storedFields.every((field) =>
		spec.secret.some((secretField) => secretField.key === field),
	);
	const keepable = editing && storedFit && storedFields.length > 0;
	const mode: SecretMode = keepable ? secretMode : "replace";

	const changeDriver = useCallback(
		(next: ExternalDataSourceDriver) => {
			setConfig((previous) => ({
				...defaultsOf(specs[next].config),
				...Object.fromEntries(
					specs[next].config
						.filter(
							(field) =>
								(field.kind === "text" || field.kind === "password") &&
								previous[field.key]?.trim(),
						)
						.map((field) => [field.key, previous[field.key]]),
				),
			}));
			setSecret({});
			setDriver(next);
		},
		[specs],
	);

	const changeName = useCallback(
		(value: string) => {
			setName(value);
			if (!aliasTouched) setAlias(suggestAlias(value));
		},
		[aliasTouched],
	);

	const credentials = useMemo(
		() =>
			Object.fromEntries(
				spec.secret
					.map((field) => [field.key, secret[field.key] ?? ""] as const)
					.filter(([, value]) => value !== ""),
			),
		[secret, spec],
	);
	const hasCredentials = Object.keys(credentials).length > 0;

	const errors = useMemo(() => {
		const found: Record<string, string> = {};
		if (!name.trim()) {
			found.name = t("externalDbNameRequired", "Give the database a name.");
		}
		if (!isValidAlias(alias)) {
			found.alias = t(
				"externalDbAliasInvalid",
				"Use up to 63 lowercase letters, digits and _, starting with a letter or _. 'public' and 'information_schema' are reserved.",
			);
		}
		for (const field of spec.config) {
			const value = config[field.key]?.trim() ?? "";
			if (field.required && !value) {
				found[field.key] = t("externalDbFieldRequired", "Required");
			} else if (field.kind === "port" && value) {
				const port = Number(value);
				if (!Number.isInteger(port) || port < 1 || port > 65535) {
					found[field.key] = t(
						"externalDbPortInvalid",
						"Use a port between 1 and 65535.",
					);
				}
			}
		}
		if (keepable && mode === "replace" && !hasCredentials) {
			found.secret = t(
				"externalDbSecretReplaceEmpty",
				"Enter the new credentials, or keep the stored ones.",
			);
		}
		return found;
	}, [alias, config, hasCredentials, keepable, mode, name, spec, t]);

	const visibleError = (key: string, value: string) =>
		showErrors || value ? errors[key] : undefined;

	const payload = useCallback((): UpsertExternalDataSourcePayload => {
		const settings: Record<string, unknown> = {};
		for (const field of spec.config) {
			const value = config[field.key]?.trim() ?? "";
			if (value) {
				settings[field.key] = field.kind === "port" ? Number(value) : value;
			}
		}
		const base = { name: name.trim(), alias, driver, config: settings };
		if (mode === "clear") return { ...base, secret: {} };
		if (mode === "replace" && hasCredentials) {
			return { ...base, secret: credentials };
		}
		if (editing && !storedFit) return { ...base, secret: {} };
		return base;
	}, [
		alias,
		config,
		credentials,
		driver,
		editing,
		hasCredentials,
		mode,
		name,
		spec,
		storedFit,
	]);

	const submit = useCallback(async () => {
		if (submitting) return;
		if (Object.keys(errors).length > 0) {
			setShowErrors(true);
			return;
		}
		setSubmitting(true);
		setServerError(null);
		try {
			const saved = source
				? await backend.dataSourceState.updateDataSource(
						appId,
						source.id,
						payload(),
					)
				: await backend.dataSourceState.createDataSource(appId, payload());
			setSecret({});
			toast.success(
				source
					? t("externalDbSaved", "Saved {{name}}", { name: saved.name })
					: t("externalDbConnected", "Connected {{name}}", {
							name: saved.name,
						}),
			);
			onSaved(saved);
			onOpenChange(false);
		} catch (error) {
			setServerError(apiErrorMessage(error, getErrorMessage(error)));
		} finally {
			setSubmitting(false);
		}
	}, [
		appId,
		backend.dataSourceState,
		errors,
		onOpenChange,
		onSaved,
		payload,
		source,
		submitting,
		t,
	]);

	const handleOpenChange = useCallback(
		(next: boolean) => {
			if (submitting) return;
			if (!next) setSecret({});
			onOpenChange(next);
		},
		[onOpenChange, submitting],
	);

	const optional = (label: string) =>
		t("externalDbOptionalLabel", "{{label}} (optional)", { label });

	return (
		<Dialog open={open} onOpenChange={handleOpenChange}>
			<DialogContent className="flex max-h-[85vh] max-w-2xl flex-col overflow-hidden">
				<DialogHeader>
					<DialogTitle className="flex items-center gap-2">
						<Server className="h-5 w-5 text-primary" />
						{editing
							? t("externalDbEditTitle", "Edit external database")
							: t("externalDbCreateTitle", "Connect an external database")}
					</DialogTitle>
					<DialogDescription>
						{t(
							"externalDbDialogDescription",
							"Queries in this app read its tables as schema.table. Credentials are stored encrypted and are never shown again.",
						)}
					</DialogDescription>
				</DialogHeader>

				<DialogBody className="-mx-1 space-y-5 px-1">
					<div className="grid gap-4 sm:grid-cols-2">
						<Field
							id="external-db-name"
							label={t("externalDbName", "Name")}
							error={visibleError("name", name)}
						>
							<Input
								id="external-db-name"
								value={name}
								onChange={(event) => changeName(event.target.value)}
								placeholder={t("externalDbNamePlaceholder", "Sales warehouse")}
								autoComplete="off"
							/>
						</Field>
						<Field
							id="external-db-alias"
							label={t("externalDbAlias", "Schema name in SQL")}
							error={visibleError("alias", alias)}
							hint={
								alias
									? t("externalDbAliasHint", "Queries read {{alias}}.<table>", {
											alias,
										})
									: undefined
							}
						>
							<Input
								id="external-db-alias"
								value={alias}
								onChange={(event) => {
									setAliasTouched(true);
									setAlias(event.target.value.trim().toLowerCase());
								}}
								placeholder="warehouse"
								autoComplete="off"
								spellCheck={false}
								className="font-mono"
							/>
						</Field>
					</div>

					<Field
						id="external-db-driver"
						label={t("externalDbDriver", "Database")}
					>
						<Select
							value={driver}
							onValueChange={(value) =>
								changeDriver(value as ExternalDataSourceDriver)
							}
						>
							<SelectTrigger id="external-db-driver">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								{EXTERNAL_DATABASE_DRIVERS.map((option) => (
									<SelectItem key={option} value={option}>
										<span className="flex items-center gap-2">
											{specs[option].label}
											{!QUERYABLE_DRIVERS.has(option) && (
												<Badge variant="outline" className="text-[10px]">
													{t(
														"externalDbQuerySupportComing",
														"Query support coming",
													)}
												</Badge>
											)}
										</span>
									</SelectItem>
								))}
							</SelectContent>
						</Select>
					</Field>

					{!QUERYABLE_DRIVERS.has(driver) && (
						<p className="rounded-md border border-dashed px-3 py-2 text-xs text-muted-foreground">
							{t(
								"externalDbQuerySupportComingDescription",
								"The connection is stored now; queries can read these tables once the query service supports this database.",
							)}
						</p>
					)}

					<div className="grid gap-4 sm:grid-cols-2">
						{spec.config.map((field) => (
							<ConfigField
								key={`${driver}-${field.key}`}
								field={field}
								label={field.required ? field.label : optional(field.label)}
								value={config[field.key] ?? ""}
								error={visibleError(field.key, config[field.key] ?? "")}
								onChange={(value) =>
									setConfig((previous) => ({ ...previous, [field.key]: value }))
								}
							/>
						))}
					</div>

					<CredentialsFields
						fields={spec.secret}
						values={secret}
						onChange={(key, value) =>
							setSecret((previous) => ({ ...previous, [key]: value }))
						}
						mode={mode}
						onModeChange={setSecretMode}
						storedFields={keepable ? storedFields : []}
						optional={optional}
						error={showErrors ? errors.secret : undefined}
					/>

					{serverError && (
						<p className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
							{serverError}
						</p>
					)}
				</DialogBody>

				<DialogFooter>
					<Button
						variant="outline"
						onClick={() => handleOpenChange(false)}
						disabled={submitting}
					>
						{t("cancel", "Cancel")}
					</Button>
					<Button onClick={submit} disabled={submitting}>
						{submitting && <Loader2 className="h-4 w-4 animate-spin" />}
						{editing
							? t("externalDbSave", "Save")
							: t("externalDbConnect", "Connect")}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

function Field({
	id,
	label,
	error,
	hint,
	children,
}: Readonly<{
	id: string;
	label: string;
	error?: string;
	hint?: string;
	children: ReactNode;
}>) {
	return (
		<div className="space-y-1.5">
			<Label htmlFor={id}>{label}</Label>
			{children}
			{error ? (
				<p className="text-xs text-destructive">{error}</p>
			) : hint ? (
				<p className="text-xs text-muted-foreground">{hint}</p>
			) : null}
		</div>
	);
}

function ConfigField({
	field,
	label,
	value,
	error,
	onChange,
}: Readonly<{
	field: FieldSpec;
	label: string;
	value: string;
	error?: string;
	onChange: (value: string) => void;
}>) {
	const id = `external-db-config-${field.key}`;
	return (
		<Field id={id} label={label} error={error}>
			{field.kind === "select" ? (
				<Select value={value} onValueChange={onChange}>
					<SelectTrigger id={id}>
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						{field.options?.map((option) => (
							<SelectItem key={option} value={option}>
								{option}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			) : (
				<Input
					id={id}
					value={value}
					onChange={(event) => onChange(event.target.value)}
					placeholder={field.placeholder}
					autoComplete="off"
					spellCheck={false}
					inputMode={field.kind === "port" ? "numeric" : undefined}
				/>
			)}
		</Field>
	);
}

function CredentialsFields({
	fields,
	values,
	onChange,
	mode,
	onModeChange,
	storedFields,
	optional,
	error,
}: Readonly<{
	fields: readonly FieldSpec[];
	values: Record<string, string>;
	onChange: (key: string, value: string) => void;
	mode: SecretMode;
	onModeChange: (mode: SecretMode) => void;
	/** Stored fields that can be kept; empty when there is nothing to keep. */
	storedFields: readonly string[];
	optional: (label: string) => string;
	error?: string;
}>) {
	const { t } = useTranslation("settings");
	const stored = storedFields.length > 0;
	return (
		<div className="space-y-3 rounded-lg border p-4">
			<div className="flex flex-wrap items-center justify-between gap-2">
				<div className="flex items-center gap-2 text-sm font-medium">
					<KeyRound className="h-4 w-4 text-muted-foreground" />
					{t("externalDbCredentials", "Credentials")}
				</div>
				{stored && mode === "keep" && (
					<div className="flex gap-2">
						<Button
							variant="outline"
							size="sm"
							onClick={() => onModeChange("replace")}
						>
							{t("externalDbReplaceCredentials", "Replace")}
						</Button>
						<Button
							variant="ghost"
							size="sm"
							onClick={() => onModeChange("clear")}
						>
							{t("externalDbRemoveCredentials", "Remove")}
						</Button>
					</div>
				)}
				{stored && mode !== "keep" && (
					<Button
						variant="ghost"
						size="sm"
						onClick={() => onModeChange("keep")}
					>
						{t("externalDbKeepCredentials", "Keep stored credentials")}
					</Button>
				)}
			</div>
			{mode === "keep" && (
				<p className="text-sm text-muted-foreground">
					{t("externalDbCredentialsStored", "Stored encrypted: {{fields}}", {
						fields: storedFields.join(", "),
					})}
				</p>
			)}
			{mode === "clear" && (
				<p className="text-sm text-muted-foreground">
					{t(
						"externalDbCredentialsWillBeRemoved",
						"The stored credentials are removed when you save.",
					)}
				</p>
			)}
			{mode === "replace" && (
				<div className="grid gap-4 sm:grid-cols-2">
					{fields.map((field) => {
						const id = `external-db-secret-${field.key}`;
						const label = field.required ? field.label : optional(field.label);
						const value = values[field.key] ?? "";
						return field.kind === "multiline" ? (
							<div key={field.key} className="space-y-1.5 sm:col-span-2">
								<Label htmlFor={id}>{label}</Label>
								<Textarea
									id={id}
									value={value}
									onChange={(event) => onChange(field.key, event.target.value)}
									placeholder={field.placeholder}
									autoComplete="off"
									spellCheck={false}
									className="min-h-28 font-mono text-xs"
								/>
							</div>
						) : (
							<div key={field.key} className="space-y-1.5">
								<Label htmlFor={id}>{label}</Label>
								<Input
									id={id}
									type={field.kind === "password" ? "password" : "text"}
									value={value}
									onChange={(event) => onChange(field.key, event.target.value)}
									placeholder={field.placeholder}
									autoComplete={
										field.kind === "password" ? "new-password" : "off"
									}
									spellCheck={false}
								/>
							</div>
						);
					})}
				</div>
			)}
			{error && <p className="text-xs text-destructive">{error}</p>}
		</div>
	);
}

export function driverSpecs(
	t: TFunction<"settings">,
): Record<ExternalDataSourceDriver, DriverSpec> {
	const host: FieldSpec = {
		key: "host",
		label: t("externalDbHost", "Host"),
		kind: "text",
		required: true,
		placeholder: "db.example.com",
	};
	const database: FieldSpec = {
		key: "database",
		label: t("externalDbDatabase", "Database"),
		kind: "text",
		required: true,
	};
	const port = (defaultValue: string): FieldSpec => ({
		key: "port",
		label: t("externalDbPort", "Port"),
		kind: "port",
		required: true,
		defaultValue,
	});
	const tls = (
		defaultValue: string,
		options: readonly string[],
	): FieldSpec => ({
		key: "ssl_mode",
		label: t("externalDbSslMode", "TLS"),
		kind: "select",
		required: true,
		defaultValue,
		options,
	});
	const login: readonly FieldSpec[] = [
		{
			key: "username",
			label: t("externalDbUsername", "Username"),
			kind: "text",
			required: false,
		},
		{
			key: "password",
			label: t("externalDbPassword", "Password"),
			kind: "password",
			required: false,
		},
	];
	return {
		postgres: {
			label: t("externalDbDriverPostgres", "PostgreSQL / CockroachDB"),
			config: [
				host,
				port("5432"),
				database,
				{
					key: "schema",
					label: t("externalDbSchema", "Schema"),
					kind: "text",
					required: true,
					defaultValue: "public",
				},
				tls("require", [
					"require",
					"verify-ca",
					"verify-full",
					"prefer",
					"disable",
				]),
			],
			secret: login,
		},
		mysql: {
			label: t("externalDbDriverMysql", "MySQL / MariaDB"),
			config: [
				host,
				port("3306"),
				database,
				tls("required", ["required", "preferred", "disabled"]),
			],
			secret: login,
		},
		clickhouse: {
			label: t("externalDbDriverClickhouse", "ClickHouse"),
			config: [host, port("8123"), database],
			secret: login,
		},
		bigquery: {
			label: t("externalDbDriverBigquery", "BigQuery"),
			config: [
				{
					key: "project_id",
					label: t("externalDbProjectId", "Project ID"),
					kind: "text",
					required: true,
					placeholder: "acme-analytics",
				},
				{
					key: "dataset",
					label: t("externalDbDataset", "Dataset"),
					kind: "text",
					required: true,
				},
				{
					key: "location",
					label: t("externalDbLocation", "Location"),
					kind: "text",
					required: false,
					placeholder: "EU",
				},
			],
			secret: [
				{
					key: "service_account_json",
					label: t("externalDbServiceAccountKey", "Service account key (JSON)"),
					kind: "multiline",
					required: false,
					placeholder: '{ "type": "service_account", … }',
				},
			],
		},
		athena: {
			label: t("externalDbDriverAthena", "Amazon Athena"),
			config: [
				{
					key: "region",
					label: t("externalDbRegion", "Region"),
					kind: "text",
					required: true,
					placeholder: "eu-west-1",
				},
				database,
				{
					key: "catalog",
					label: t("externalDbCatalog", "Catalog"),
					kind: "text",
					required: false,
					placeholder: "AwsDataCatalog",
				},
				{
					key: "workgroup",
					label: t("externalDbWorkgroup", "Workgroup"),
					kind: "text",
					required: false,
					placeholder: "primary",
				},
				{
					key: "output_location",
					label: t("externalDbOutputLocation", "Query result location"),
					kind: "text",
					required: false,
					placeholder: "s3://bucket/athena-results/",
				},
			],
			secret: [
				{
					key: "access_key_id",
					label: t("externalDbAccessKeyId", "Access key ID"),
					kind: "text",
					required: false,
				},
				{
					key: "secret_access_key",
					label: t("externalDbSecretAccessKey", "Secret access key"),
					kind: "password",
					required: false,
				},
				{
					key: "session_token",
					label: t("externalDbSessionToken", "Session token"),
					kind: "password",
					required: false,
				},
			],
		},
	};
}
