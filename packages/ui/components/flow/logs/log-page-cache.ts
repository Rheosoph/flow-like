export const PAGE_SIZE = 200;
export const MAX_CACHED_PAGES = 40;

/**
 * Fetched pages of one (run, query) pair. Least recently used pages are
 * dropped past `maxPages`, and a page already on its way is never requested
 * twice. A new query gets a new cache; results for an old one land in a cache
 * nobody reads.
 */
export class LogPageCache<T> {
	private readonly pages = new Map<number, T[]>();
	private readonly inFlight = new Map<number, Promise<T[]>>();
	private readonly failed = new Set<number>();

	constructor(
		readonly pageSize = PAGE_SIZE,
		readonly maxPages = MAX_CACHED_PAGES,
	) {}

	pageOf(index: number): number {
		return Math.floor(index / this.pageSize);
	}

	has(page: number): boolean {
		return this.pages.has(page);
	}

	isLoading(page: number): boolean {
		return this.inFlight.has(page);
	}

	hasFailed(page: number): boolean {
		return this.failed.has(page);
	}

	get size(): number {
		return this.pages.size;
	}

	get hasFailures(): boolean {
		return this.failed.size > 0;
	}

	/** Reads without refreshing recency, for renders. */
	row(index: number): T | undefined {
		return this.pages.get(this.pageOf(index))?.[index % this.pageSize];
	}

	touch(page: number): void {
		const rows = this.pages.get(page);
		if (!rows) return;
		this.pages.delete(page);
		this.pages.set(page, rows);
	}

	set(page: number, rows: T[]): void {
		this.pages.delete(page);
		this.pages.set(page, rows);
		this.failed.delete(page);
		while (this.pages.size > this.maxPages) {
			const oldest = this.pages.keys().next().value;
			if (oldest === undefined) break;
			this.pages.delete(oldest);
		}
	}

	/** Starts loading `page` unless it is cached or already loading. */
	request(
		page: number,
		fetchPage: (offset: number, limit: number) => Promise<T[]>,
	): Promise<T[]> | undefined {
		if (this.pages.has(page) || this.inFlight.has(page)) return undefined;
		const promise = fetchPage(page * this.pageSize, this.pageSize)
			.then((rows) => {
				this.set(page, rows);
				return rows;
			})
			.catch((error: unknown) => {
				this.failed.add(page);
				throw error;
			})
			.finally(() => {
				this.inFlight.delete(page);
			});
		this.inFlight.set(page, promise);
		return promise;
	}

	/** Pages covering `[first, last]`, clamped to `count` rows. */
	pagesFor(first: number, last: number, count: number): number[] {
		if (count <= 0 || last < first) return [];
		const from = this.pageOf(Math.max(0, first));
		const to = this.pageOf(Math.min(count - 1, last));
		const pages: number[] = [];
		for (let page = from; page <= to; page++) pages.push(page);
		return pages;
	}
}
