declare module "indexeddbshim/dist/indexeddbshim-noninvasive.js" {
	/**
	 * UMD build: the export shape depends on how the bundler interprets the
	 * file (ESM side effect on globalThis vs CJS module.exports). Resolved
	 * at runtime by resolveSetGlobalVars().
	 */
	const setGlobalVars: unknown;
	export default setGlobalVars;
}

declare module "websql-configurable/custom/index.js" {
	interface WebSQLDriverQuery {
		sql: string;
		args: unknown[];
	}

	interface WebSQLDriverResult {
		error?: Error;
		insertId?: number;
		rowsAffected?: number;
		rows?: Array<Record<string, unknown>>;
	}

	interface WebSQLDriver {
		exec: (
			queries: WebSQLDriverQuery[],
			readOnly: boolean,
			callback: (err?: Error | null, results?: WebSQLDriverResult[]) => void,
		) => void;
	}

	type OpenDatabase = (
		name: string,
		version: string,
		description: string,
		size: number,
		callback?: (db: unknown) => void,
	) => unknown;

	function customOpenDatabase(
		DriverClass: new (name: string) => WebSQLDriver,
	): OpenDatabase;

	export default customOpenDatabase;
}
