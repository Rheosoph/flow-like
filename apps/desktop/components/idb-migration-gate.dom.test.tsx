// @vitest-environment happy-dom

import { StrictMode, act, useEffect } from "react";
import { type Root, createRoot, hydrateRoot } from "react-dom/client";
import { renderToString } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const migration = vi.hoisted(() => ({
	needed: vi.fn<() => boolean>(),
	run: vi.fn<() => Promise<void>>(),
	translation: "Upgrading local storage…",
}));

vi.mock("../lib/init-idb-sqlite", () => ({
	needsSqliteIdbMigration: migration.needed,
	runSqliteIdbMigration: migration.run,
}));
vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({ t: () => migration.translation }),
}));

import { IdbMigrationGate } from "./idb-migration-gate";

function deferred() {
	let resolve!: () => void;
	let reject!: (error: Error) => void;
	const promise = new Promise<void>((done, fail) => {
		resolve = done;
		reject = fail;
	});
	return { promise, resolve, reject };
}

const readData = vi.fn();
const renderData = vi.fn();

function DataConsumer() {
	renderData();
	useEffect(() => {
		readData();
	}, []);
	return <p>Data consumers mounted</p>;
}

const tree = (
	<IdbMigrationGate>
		<DataConsumer />
	</IdbMigrationGate>
);

let host: HTMLDivElement;
let root: Root | undefined;
let pending: ReturnType<typeof deferred>;

beforeEach(() => {
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	migration.needed.mockReset().mockReturnValue(true);
	pending = deferred();
	migration.run.mockReset().mockReturnValue(pending.promise);
	migration.translation = "Upgrading local storage…";
	readData.mockClear();
	renderData.mockClear();
	host = document.createElement("div");
	document.body.append(host);
});

afterEach(async () => {
	if (root) await act(async () => root?.unmount());
	root = undefined;
	host.remove();
	vi.unstubAllGlobals();
});

function serverMarkup() {
	const browserWindow = window;
	vi.stubGlobal("window", undefined);
	try {
		return renderToString(tree);
	} finally {
		vi.stubGlobal("window", browserWindow);
	}
}

describe("IndexedDB migration gate", () => {
	it("hydrates first-launch storage without mounting data consumers before migration", async () => {
		host.innerHTML = serverMarkup();
		expect(migration.needed).not.toHaveBeenCalled();
		expect(renderData).not.toHaveBeenCalled();
		const onRecoverableError = vi.fn();
		migration.translation = "Lokaler Speicher wird aktualisiert…";
		await act(async () => {
			root = hydrateRoot(host, tree, { onRecoverableError });
		});
		expect(onRecoverableError).not.toHaveBeenCalled();
		expect(host.textContent).toContain(migration.translation);
		expect(renderData).not.toHaveBeenCalled();
		expect(readData).not.toHaveBeenCalled();
		await act(async () => pending.resolve());
		expect(host.textContent).toBe("Data consumers mounted");
		expect(readData).toHaveBeenCalledOnce();
	});

	it("hydrates an already migrated profile and opens it without starting migration", async () => {
		migration.needed.mockReturnValue(false);
		host.innerHTML = serverMarkup();
		expect(renderData).not.toHaveBeenCalled();
		const onRecoverableError = vi.fn();
		await act(async () => {
			root = hydrateRoot(host, tree, { onRecoverableError });
		});
		expect(onRecoverableError).not.toHaveBeenCalled();
		expect(migration.run).not.toHaveBeenCalled();
		expect(readData).toHaveBeenCalledOnce();
	});

	it("ignores an obsolete effect completion during StrictMode replay", async () => {
		const current = deferred();
		migration.run
			.mockReturnValueOnce(pending.promise)
			.mockReturnValueOnce(current.promise);
		root = createRoot(host);
		await act(async () => root?.render(<StrictMode>{tree}</StrictMode>));
		expect(migration.run).toHaveBeenCalledTimes(2);
		await act(async () => pending.resolve());
		expect(renderData).not.toHaveBeenCalled();
		await act(async () => current.resolve());
		expect(host.textContent).toBe("Data consumers mounted");
		expect(readData).toHaveBeenCalled();
	});

	it("releases the gate when the migration service rejects", async () => {
		root = createRoot(host);
		await act(async () => root?.render(tree));
		expect(readData).not.toHaveBeenCalled();
		await act(async () => pending.reject(new Error("Migration failed")));
		expect(host.textContent).toBe("Data consumers mounted");
		expect(readData).toHaveBeenCalledOnce();
	});

	it("does not mount data consumers after the gate has unmounted", async () => {
		root = createRoot(host);
		await act(async () => root?.render(tree));
		await act(async () => root?.unmount());
		root = undefined;
		await act(async () => pending.resolve());
		expect(renderData).not.toHaveBeenCalled();
		expect(readData).not.toHaveBeenCalled();
	});
});
