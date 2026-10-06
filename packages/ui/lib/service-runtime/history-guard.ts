import type Dexie from "dexie";
import type { DBCore, DBCoreMutateResponse } from "dexie";
import { runtimeNamespaceActive, runtimeRowAppId } from "./session-scope";

/** Keep delayed UI effects from writing a retired deployment session back to disk. */
export function guardRuntimeHistoryCore(down: DBCore): DBCore {
	return {
		...down,
		table(name) {
			const table = down.table(name);
			return {
				...table,
				mutate(request) {
					if (request.type !== "add" && request.type !== "put")
						return table.mutate(request);
					const retained: number[] = [];
					request.values.forEach((value, index) => {
						const appId = runtimeRowAppId(value);
						if (!appId || runtimeNamespaceActive(appId)) retained.push(index);
					});
					if (retained.length === request.values.length)
						return table.mutate(request);
					const results = request.values.map(
						(value, index) =>
							request.keys?.[index] ??
							table.schema.primaryKey.extractKey?.(value),
					);
					const finish = (
						response?: DBCoreMutateResponse,
					): DBCoreMutateResponse => {
						const failures: DBCoreMutateResponse["failures"] = {};
						for (const [index, original] of retained.entries()) {
							if (response?.failures[index])
								failures[original] = response.failures[index];
							if (response?.results)
								results[original] = response.results[index];
						}
						return {
							numFailures: Object.keys(failures).length,
							failures,
							results,
							lastResult: results.at(-1),
						};
					};
					if (!retained.length) return Promise.resolve(finish());
					return table
						.mutate({
							...request,
							values: retained.map((index) => request.values[index]),
							keys: request.keys
								? retained.map((index) => request.keys?.[index])
								: undefined,
						})
						.then(finish);
				},
			};
		},
	};
}

export function installRuntimeHistoryGuard(db: Dexie): void {
	db.use({
		stack: "dbcore",
		name: "RuntimeSessionHistory",
		level: -10,
		create: guardRuntimeHistoryCore,
	});
}
