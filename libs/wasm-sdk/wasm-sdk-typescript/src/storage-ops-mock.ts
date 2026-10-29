import type { FlowPath } from "./host";
import {
	type FileStat,
	type ListPage,
	type ListRequest,
	StorageError,
	type StorageOps,
	type WriteOptions,
	storageU64,
} from "./storage-ops";

const MAX_BYTES = 10 * 1024 * 1024;
const key = (path: FlowPath) => JSON.stringify([path.store_ref, path.path]);
const fail = (code: string, message: string): never => {
	throw new StorageError(code, message);
};

export class MockStorageOps implements StorageOps {
	private files = new Map<
		string,
		{ path: FlowPath; data: Uint8Array; stat: FileStat }
	>();
	private writes = new Map<
		string,
		{ path: FlowPath; total?: bigint; size: bigint; chunks: Uint8Array[] }
	>();
	private listings = new Map<
		string,
		{ binding: string; entries: FileStat[] }
	>();
	private sequence = 0;
	get pendingWrites(): number {
		return this.writes.size;
	}
	get openListings(): number {
		return this.listings.size;
	}
	private nextId(): string {
		return `mock-${++this.sequence}`;
	}
	private file(path: FlowPath) {
		return (
			this.files.get(key(path)) ?? fail("not_found", "File does not exist")
		);
	}
	private put(path: FlowPath, data: Uint8Array): FileStat {
		const tag = this.nextId();
		const stat: FileStat = {
			...path,
			size: data.length,
			e_tag: tag,
			version: tag,
			last_modified: "1970-01-01T00:00:00Z",
		};
		this.files.set(key(path), { path: { ...path }, data: data.slice(), stat });
		return { ...stat };
	}
	stat(path: FlowPath): FileStat {
		return { ...this.file(path).stat };
	}
	readRange(
		path: FlowPath,
		offset: number | bigint,
		length: number | bigint,
		ifMatch?: string,
	): Uint8Array {
		const start = storageU64(offset);
		const count = storageU64(length);
		if (start + count > 0xffffffffffffffffn)
			fail("invalid_argument", "Read range overflows u64");
		if (count > BigInt(MAX_BYTES))
			fail("resource_limit", "Range exceeds 10 MiB");
		const file = this.file(path);
		if (ifMatch !== undefined && ifMatch !== file.stat.e_tag)
			fail("conflict", "File changed");
		if (start > BigInt(file.data.length))
			fail("invalid_argument", "Read offset is beyond EOF");
		return file.data.slice(
			Number(
				start > BigInt(file.data.length) ? BigInt(file.data.length) : start,
			),
			Number(
				start + count > BigInt(file.data.length)
					? BigInt(file.data.length)
					: start + count,
			),
		);
	}
	listPage(request: ListRequest): ListPage {
		const limit = request.limit ?? 100;
		if (!Number.isInteger(limit) || limit < 1 || limit > 1000)
			fail("invalid_argument", "Page limit must be between 1 and 1000");
		const binding = JSON.stringify([
			request.path.path,
			request.path.store_ref,
			request.path.cache_store_ref,
			request.recursive ?? true,
			[...new Set(request.exclude_directories ?? [])].sort(),
		]);
		let id = request.cursor;
		let entries: FileStat[];
		if (id) {
			const session =
				this.listings.get(id) ??
				fail("invalid_cursor", "Listing is closed or expired");
			if (session.binding !== binding)
				fail("invalid_cursor", "Listing options changed");
			entries = session.entries;
		} else {
			if (this.listings.size >= 32)
				fail("resource_limit", "Too many open listings");
			id = this.nextId();
			const prefix = request.path.path.replace(/\/+$/, "");
			entries = Array.from(this.files.values())
				.filter((file) => {
					if (file.path.store_ref !== request.path.store_ref) return false;
					if (prefix && !file.path.path.startsWith(`${prefix}/`)) return false;
					const relative = prefix
						? file.path.path.slice(prefix.length + 1)
						: file.path.path;
					const directories = relative.split("/").slice(0, -1);
					return (
						(request.recursive !== false || directories.length === 0) &&
						!directories.some((dir) =>
							request.exclude_directories?.includes(dir),
						)
					);
				})
				.map((file) => ({ ...file.stat }));
		}
		const page = entries.splice(0, limit);
		if (!entries.length) {
			this.listings.delete(id);
			return { entries: page, cursor: null };
		}
		this.listings.set(id, { binding, entries });
		return { entries: page, cursor: id };
	}
	closeList(cursor: string): void {
		if (!this.listings.delete(cursor))
			fail("invalid_cursor", "Listing is closed or expired");
	}
	write(
		path: FlowPath,
		data: Uint8Array,
		options: WriteOptions = {},
	): FileStat {
		if (options.if_match === "" || options.version === "")
			fail("invalid_argument", "Write preconditions cannot be empty");
		if (data.length > MAX_BYTES)
			fail("resource_limit", "Write exceeds 10 MiB; stream chunks instead");
		if (
			options.create_only &&
			(options.if_match !== undefined || options.version !== undefined)
		)
			fail(
				"invalid_argument",
				"create_only cannot be combined with a precondition",
			);
		const current = this.files.get(key(path));
		if (
			(options.create_only && current) ||
			(options.if_match !== undefined &&
				options.if_match !== current?.stat.e_tag) ||
			(options.version !== undefined &&
				options.version !== current?.stat.version)
		)
			fail("conflict", "File changed or destination already exists");
		return this.put(path, data);
	}
	writeStart(path: FlowPath, totalSize?: number | bigint): string {
		if (this.writes.size >= 8) fail("resource_limit", "Too many open writers");
		const total = totalSize === undefined ? undefined : storageU64(totalSize);
		const id = this.nextId();
		this.writes.set(id, { path: { ...path }, total, size: 0n, chunks: [] });
		return id;
	}
	writeChunk(id: string, data: Uint8Array): void {
		const writer =
			this.writes.get(id) ?? fail("invalid_argument", "Unknown write handle");
		if (data.length > MAX_BYTES) {
			this.writes.delete(id);
			fail("resource_limit", "Chunk exceeds 10 MiB");
		}
		if (
			writer.total !== undefined &&
			writer.size + BigInt(data.length) > writer.total
		) {
			this.writes.delete(id);
			fail("incomplete_write", "Declared size exceeded");
		}
		writer.chunks.push(data.slice());
		writer.size += BigInt(data.length);
	}
	writeFinish(id: string): FileStat {
		const writer =
			this.writes.get(id) ?? fail("invalid_argument", "Unknown write handle");
		this.writes.delete(id);
		if (writer.total !== undefined && writer.size !== writer.total)
			fail("incomplete_write", "Declared size was not written");
		const data = new Uint8Array(Number(writer.size));
		let offset = 0;
		for (const chunk of writer.chunks) {
			data.set(chunk, offset);
			offset += chunk.length;
		}
		return this.put(writer.path, data);
	}
	writeAbort(id: string): void {
		this.writes.delete(id);
	}
	delete(path: FlowPath): void {
		this.files.delete(key(path));
	}
	copy(source: FlowPath, destination: FlowPath, overwrite = false): void {
		const file = this.file(source);
		if (key(source) === key(destination)) return;
		if (
			source.store_ref !== destination.store_ref &&
			!overwrite &&
			file.data.length > MAX_BYTES
		)
			fail(
				"unsupported",
				"Create-only cross-store copies are limited to 10 MiB",
			);
		if (!overwrite && this.files.has(key(destination)))
			fail("conflict", "Destination already exists");
		this.put(destination, file.data);
	}
	move(source: FlowPath, destination: FlowPath, overwrite = false): void {
		if (key(source) === key(destination)) {
			this.file(source);
			return;
		}
		this.copy(source, destination, overwrite);
		this.delete(source);
	}
	fileUrl(path: FlowPath): string | null {
		this.file(path);
		return null;
	}
}
