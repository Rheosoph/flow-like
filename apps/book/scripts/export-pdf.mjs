#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { createReadStream } from "node:fs";
import {
	access,
	mkdir,
	mkdtemp,
	readFile,
	rename,
	rm,
	stat,
	writeFile,
} from "node:fs/promises";
import { createServer } from "node:http";
import { dirname, extname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const bookDirectory = resolve(scriptDirectory, "..");
const repositoryDirectory = resolve(bookDirectory, "../..");

const DEFAULT_DIST_DIRECTORY = resolve(bookDirectory, "dist");
const DEFAULT_OUTPUT_PATH = resolve(
	repositoryDirectory,
	"output/pdf/flowbook.pdf",
);
const DEFAULT_PRINT_ROUTE = "/print/";
const INTERNAL_LINK_PREFIX =
	"https://book.flow-like.com/__flowbook_pdf_target__/";

const MIME_TYPES = new Map([
	[".avif", "image/avif"],
	[".css", "text/css; charset=utf-8"],
	[".gif", "image/gif"],
	[".html", "text/html; charset=utf-8"],
	[".ico", "image/x-icon"],
	[".jpeg", "image/jpeg"],
	[".jpg", "image/jpeg"],
	[".js", "text/javascript; charset=utf-8"],
	[".json", "application/json; charset=utf-8"],
	[".mjs", "text/javascript; charset=utf-8"],
	[".png", "image/png"],
	[".svg", "image/svg+xml"],
	[".webp", "image/webp"],
	[".woff", "font/woff"],
	[".woff2", "font/woff2"],
	[".xml", "application/xml; charset=utf-8"],
]);

function usage() {
	return `FlowBook PDF exporter

Usage:
  node apps/book/scripts/export-pdf.mjs [options]
  bun apps/book/scripts/export-pdf.mjs [options]

Options:
  --dist <directory>       Built Astro output (default: apps/book/dist)
  --output <file>          Final PDF (default: output/pdf/flowbook.pdf)
  --route <path>           Print route in the built site (default: /print/)
  --keep-temp              Keep intermediate PDFs and extracted text
  --help                   Show this help

The Astro site must be built before this command runs. The print page must expose
[data-pdf-front], [data-pdf-body], and one [data-pdf-chapter] per chapter.`;
}

function parseArguments(argv) {
	const options = {
		distDirectory: DEFAULT_DIST_DIRECTORY,
		outputPath: DEFAULT_OUTPUT_PATH,
		printRoute: DEFAULT_PRINT_ROUTE,
		keepTemp: false,
	};

	for (let index = 0; index < argv.length; index += 1) {
		const argument = argv[index];
		if (argument === "--help") {
			console.log(usage());
			process.exit(0);
		}
		if (argument === "--keep-temp") {
			options.keepTemp = true;
			continue;
		}

		const value = argv[index + 1];
		if (argument === "--dist" || argument === "--output" || argument === "--route") {
			if (!value) throw new Error(`${argument} requires a value.`);
			index += 1;
			if (argument === "--dist") {
				options.distDirectory = resolveFromWorkingDirectory(value);
			} else if (argument === "--output") {
				options.outputPath = resolveFromWorkingDirectory(value);
			} else {
				options.printRoute = normalizeRoute(value);
			}
			continue;
		}

		throw new Error(`Unknown argument: ${argument}\n\n${usage()}`);
	}

	return options;
}

function resolveFromWorkingDirectory(value) {
	return isAbsolute(value) ? value : resolve(process.cwd(), value);
}

function normalizeRoute(value) {
	const withLeadingSlash = value.startsWith("/") ? value : `/${value}`;
	return withLeadingSlash.endsWith("/")
		? withLeadingSlash
		: `${withLeadingSlash}/`;
}

function formatRelativePath(path) {
	const localPath = relative(repositoryDirectory, path);
	return localPath && !localPath.startsWith("..") ? localPath : path;
}

async function ensureBuiltPrintRoute(distDirectory, printRoute) {
	const routePath = printRoute.replace(/^\/+|\/+$/g, "");
	const printHtmlPath = resolve(distDirectory, routePath, "index.html");
	try {
		await access(printHtmlPath);
	} catch {
		throw new Error(
			`Built print route is missing at ${formatRelativePath(printHtmlPath)}. Build apps/book after adding src/pages/print.astro, then rerun the exporter.`,
		);
	}
	return printHtmlPath;
}

function isPathInside(parent, candidate) {
	const pathFromParent = relative(parent, candidate);
	return (
		pathFromParent === "" ||
		(!pathFromParent.startsWith(`..${sep}`) && pathFromParent !== ".." && !isAbsolute(pathFromParent))
	);
}

async function resolveStaticFile(distDirectory, pathname) {
	let decodedPath;
	try {
		decodedPath = decodeURIComponent(pathname);
	} catch {
		return { status: 400 };
	}
	if (decodedPath.includes("\0")) return { status: 400 };

	let candidate = resolve(distDirectory, `.${decodedPath}`);
	if (!isPathInside(distDirectory, candidate)) return { status: 403 };

	try {
		const candidateStat = await stat(candidate);
		if (candidateStat.isDirectory()) candidate = resolve(candidate, "index.html");
	} catch {
		if (!extname(candidate)) candidate = resolve(candidate, "index.html");
	}

	if (!isPathInside(distDirectory, candidate)) return { status: 403 };
	try {
		const fileStat = await stat(candidate);
		if (!fileStat.isFile()) return { status: 404 };
		return { status: 200, path: candidate, size: fileStat.size };
	} catch {
		return { status: 404 };
	}
}

async function startStaticServer(distDirectory) {
	const server = createServer(async (request, response) => {
		try {
			const requestUrl = new URL(request.url ?? "/", "http://127.0.0.1");
			const file = await resolveStaticFile(distDirectory, requestUrl.pathname);
			if (file.status !== 200 || !file.path) {
				response.writeHead(file.status, { "Content-Type": "text/plain; charset=utf-8" });
				response.end(file.status === 404 ? "Not found" : "Invalid request");
				return;
			}

			response.writeHead(200, {
				"Cache-Control": "no-store",
				"Content-Length": file.size,
				"Content-Type": MIME_TYPES.get(extname(file.path).toLowerCase()) ?? "application/octet-stream",
			});
			if (request.method === "HEAD") {
				response.end();
				return;
			}
			createReadStream(file.path).pipe(response);
		} catch (error) {
			response.writeHead(500, { "Content-Type": "text/plain; charset=utf-8" });
			response.end(error instanceof Error ? error.message : String(error));
		}
	});

	await new Promise((resolveListen, rejectListen) => {
		server.once("error", rejectListen);
		server.listen(0, "127.0.0.1", () => {
			server.off("error", rejectListen);
			resolveListen();
		});
	});

	const address = server.address();
	if (!address || typeof address === "string") {
		server.close();
		throw new Error("Could not resolve the local static server address.");
	}

	return {
		server,
		origin: `http://127.0.0.1:${address.port}`,
	};
}

async function closeServer(server) {
	if (!server) return;
	await new Promise((resolveClose) => server.close(() => resolveClose()));
}

async function loadDependencies() {
	try {
		const [puppeteerModule, pdfLib] = await Promise.all([
			import("puppeteer"),
			import("pdf-lib"),
		]);
		return {
			puppeteer: puppeteerModule.default,
			pdfLib,
		};
	} catch (error) {
		throw new Error(
			`The FlowBook PDF export requires the \`puppeteer\` and \`pdf-lib\` packages. Install the workspace dependencies before exporting. (${error instanceof Error ? error.message : String(error)})`,
		);
	}
}

async function waitForDocumentAssets(page) {
	await page.evaluate(async () => {
		if (document.fonts?.ready) await document.fonts.ready;
		const images = Array.from(document.images);
		for (const image of images) {
			image.loading = "eager";
			image.decoding = "sync";
			image.fetchPriority = "high";
		}
		await Promise.all(
			images.map(async (image) => {
				if (!image.complete) {
					await new Promise((resolveImage) => {
						const timeout = window.setTimeout(resolveImage, 30_000);
						const finish = () => {
							window.clearTimeout(timeout);
							resolveImage();
						};
						image.addEventListener("load", finish, { once: true });
						image.addEventListener("error", finish, { once: true });
					});
				}
				if (image.naturalWidth > 0) {
					try {
						await image.decode();
					} catch {
						// The explicit broken-image check below reports a useful source URL.
					}
				}
			}),
		);
		const brokenImages = images
			.filter((image) => !image.complete || image.naturalWidth === 0)
			.map((image) => image.currentSrc || image.src || "<unknown image>");
		if (brokenImages.length > 0) {
			throw new Error(`Print page has broken images: ${brokenImages.join(", ")}`);
		}
	});
}

async function normalizePrintDocument(page) {
	return page.evaluate((internalLinkPrefix) => {
		const FRONT_SELECTORS = [
			"[data-pdf-front]",
			".pdf-front",
			".print-front",
		].join(",");
		const BODY_SELECTORS = [
			"[data-pdf-body]",
			".pdf-body",
			".print-body",
		].join(",");
		const CHAPTER_SELECTORS = [
			"[data-pdf-chapter]",
			".pdf-chapter",
			".print-chapter",
			"article[data-chapter-slug]",
		].join(",");

		const front = document.querySelector(FRONT_SELECTORS);
		const body = document.querySelector(BODY_SELECTORS);
		if (!(front instanceof HTMLElement) || !(body instanceof HTMLElement)) {
			throw new Error(
				"The print route must contain [data-pdf-front] and [data-pdf-body] roots.",
			);
		}

		const initialChapterCandidates = Array.from(
			body.querySelectorAll(CHAPTER_SELECTORS),
		).filter((element) => element instanceof HTMLElement);
		const chapterElements =
			initialChapterCandidates.length > 0
				? initialChapterCandidates
				: Array.from(body.querySelectorAll(":scope > article, :scope > section")).filter(
						(element) => element instanceof HTMLElement && element.querySelector("h1"),
					);
		if (chapterElements.length === 0) {
			throw new Error(
				"The print route has no chapters. Add data-pdf-chapter to each rendered chapter wrapper.",
			);
		}

		const slugify = (value) =>
			value
				.normalize("NFKD")
				.toLowerCase()
				.replace(/[^a-z0-9]+/g, "-")
				.replace(/^-+|-+$/g, "") || "chapter";
		const normalizeText = (value) =>
			value.normalize("NFKD").toLowerCase().replace(/[^a-z0-9]+/g, " ").trim();
		const normalizePath = (value) => {
			try {
				const url = new URL(value, location.origin);
				const path = decodeURIComponent(url.pathname)
					.replace(/\/index\.html$/, "/")
					.replace(/\/+$/, "");
				return path || "/";
			} catch {
				return null;
			}
		};
		const canonicalLink = document.querySelector('link[rel="canonical"]');
		let canonicalOrigin = "https://book.flow-like.com";
		if (canonicalLink instanceof HTMLLinkElement) {
			try {
				canonicalOrigin = new URL(canonicalLink.href).origin;
			} catch {
				// Keep the stable public fallback.
			}
		}

		document.documentElement.dataset.theme = "light";
		document.documentElement.dataset.pdfExport = "true";
		document.documentElement.style.colorScheme = "light";

		let overrideStyle = document.querySelector("#flowbook-pdf-export-overrides");
		if (!(overrideStyle instanceof HTMLStyleElement)) {
			overrideStyle = document.createElement("style");
			overrideStyle.id = "flowbook-pdf-export-overrides";
			document.head.append(overrideStyle);
		}
		overrideStyle.textContent = `
			@media print {
				html { color-scheme: light !important; }
				html[data-pdf-pass="body"] :is(${FRONT_SELECTORS}) { display: none !important; }
				html[data-pdf-pass="front"] :is(${BODY_SELECTORS}) { display: none !important; }
				.pdf-chapter-marked { position: relative !important; }
				.pdf-page-marker {
					position: absolute !important;
					inset: 0 auto auto 0 !important;
					display: block !important;
					margin: 0 !important;
					padding: 0 !important;
					font: 1px/1 Arial, sans-serif !important;
					letter-spacing: 0 !important;
					white-space: nowrap !important;
					color: #fff !important;
					opacity: 1 !important;
				}
				pre.pdf-code-wide,
				.pdf-code-wide pre {
					font-size: 6.35pt !important;
					line-height: 1.48 !important;
				}
				pre.pdf-code-very-wide,
				.pdf-code-very-wide pre {
					font-size: 6.6pt !important;
					line-height: 1.42 !important;
					white-space: pre-wrap !important;
					overflow-wrap: anywhere !important;
					word-break: break-word !important;
				}
				.pdf-table-cards {
					display: grid !important;
					gap: 3.2mm !important;
					margin-block: 5mm !important;
				}
				.pdf-table-card {
					break-inside: avoid-page !important;
					border: 0.35mm solid #d7d2ce !important;
					border-top: 0.8mm solid #e45234 !important;
					border-radius: 1.5mm !important;
					background: #fff !important;
					padding: 3.4mm 4mm !important;
				}
				.pdf-table-card__title {
					margin: 0 0 2.5mm !important;
					font: 700 9.5pt/1.25 Arial, sans-serif !important;
					color: #191719 !important;
				}
				.pdf-table-card__fields {
					display: grid !important;
					grid-template-columns: minmax(28mm, 0.32fr) minmax(0, 1fr) !important;
					margin: 0 !important;
				}
				.pdf-table-card__label,
				.pdf-table-card__value {
					margin: 0 !important;
					padding-block: 1.6mm !important;
					border-top: 0.25mm solid #ebe7e3 !important;
					font-size: 8.5pt !important;
					line-height: 1.4 !important;
				}
				.pdf-table-card__label {
					padding-inline-end: 3mm !important;
					font-family: Arial, sans-serif !important;
					font-weight: 700 !important;
					color: #5c5652 !important;
				}
				.pdf-table-card__value { color: #191719 !important; }
				[data-toc-page], .toc-page {
					margin-inline-start: auto !important;
					padding-inline-start: 4mm !important;
					font-variant-numeric: tabular-nums !important;
					white-space: nowrap !important;
				}
			}
		`;

		const usedIds = new Set();
		const pathToChapter = new Map();
		const titleToChapter = new Map();
		const chapters = chapterElements.map((chapter, chapterIndex) => {
			const heading = chapter.querySelector("h1");
			const title =
				chapter.dataset.chapterTitle?.trim() || heading?.textContent?.trim() || `Chapter ${chapterIndex + 1}`;
			const idSource =
				chapter.id ||
				chapter.dataset.chapterSlug ||
				chapter.dataset.slug ||
				chapter.dataset.entryId ||
				title;
			let chapterId = slugify(idSource);
			let suffix = 2;
			while (usedIds.has(chapterId)) {
				chapterId = `${slugify(idSource)}-${suffix}`;
				suffix += 1;
			}
			usedIds.add(chapterId);
			chapter.id = chapterId;
			chapter.dataset.pdfChapter = "true";
			chapter.dataset.pdfChapterId = chapterId;
			chapter.classList.add("pdf-chapter-marked");

			const paths = new Set();
			for (const attribute of [
				"data-path",
				"data-source-path",
				"data-chapter-path",
				"data-pdf-path",
				"data-url",
			]) {
				const value = chapter.getAttribute(attribute);
				const path = value ? normalizePath(value) : null;
				if (path) paths.add(path);
			}
			for (const slug of [
				chapter.dataset.chapterSlug,
				chapter.dataset.slug,
				chapter.dataset.entryId,
			]) {
				if (!slug) continue;
				const path = normalizePath(`/${slug}/`);
				if (path) paths.add(path);
			}

			const localIdMap = new Map();
			for (const element of chapter.querySelectorAll("[id]")) {
				if (!(element instanceof HTMLElement || element instanceof SVGElement)) continue;
				const oldId = element.id;
				if (!oldId) continue;
				let nextId = `${chapterId}--${slugify(oldId)}`;
				let idSuffix = 2;
				while (usedIds.has(nextId)) {
					nextId = `${chapterId}--${slugify(oldId)}-${idSuffix}`;
					idSuffix += 1;
				}
				usedIds.add(nextId);
				localIdMap.set(oldId, nextId);
				element.id = nextId;
			}
			for (const element of [chapter, ...chapter.querySelectorAll("*")]) {
				for (const attribute of [
					"aria-controls",
					"aria-describedby",
					"aria-labelledby",
					"headers",
				]) {
					const references = element.getAttribute(attribute);
					if (!references) continue;
					element.setAttribute(
						attribute,
						references
							.split(/\s+/)
							.map((reference) => localIdMap.get(reference) ?? reference)
							.join(" "),
					);
				}
				const labelTarget = element.getAttribute("for");
				if (labelTarget && localIdMap.has(labelTarget)) {
					element.setAttribute("for", localIdMap.get(labelTarget));
				}
			}

			const marker = `FLOWBOOK_PAGE_MARKER_${String(chapterIndex + 1).padStart(4, "0")}`;
			const markerElement = document.createElement("span");
			markerElement.className = "pdf-page-marker";
			markerElement.dataset.pdfPageMarker = marker;
			markerElement.textContent = marker;
			chapter.prepend(markerElement);

			const entry = {
				id: chapterId,
				kind: "chapter",
				title,
				marker,
				paths: Array.from(paths),
				localIdMap,
				element: chapter,
			};
			titleToChapter.set(normalizeText(title), entry);
			for (const path of paths) pathToChapter.set(path, entry);
			return entry;
		});
		const partTargets = Array.from(body.querySelectorAll("[data-pdf-part]"))
			.filter((element) => element instanceof HTMLElement)
			.map((part, partIndex) => {
				const heading = part.querySelector("h1");
				const label = part.querySelector(".part-kicker, .part-divider__label")?.textContent?.trim();
				const headingText = heading?.textContent?.trim() || `Part ${partIndex + 1}`;
				const title = label ? `${label} — ${headingText}` : headingText;
				const idSource = part.id || part.dataset.pdfPart || title;
				let partId = slugify(idSource);
				let suffix = 2;
				while (usedIds.has(partId)) {
					partId = `${slugify(idSource)}-${suffix}`;
					suffix += 1;
				}
				usedIds.add(partId);
				part.id = partId;
				part.classList.add("pdf-chapter-marked");
				const marker = `FLOWBOOK_PAGE_MARKER_PART_${String(partIndex + 1).padStart(4, "0")}`;
				const markerElement = document.createElement("span");
				markerElement.className = "pdf-page-marker";
				markerElement.dataset.pdfPageMarker = marker;
				markerElement.textContent = marker;
				part.prepend(markerElement);
				return {
					id: partId,
					kind: "part",
					title,
					marker,
					paths: [],
					element: part,
				};
			});
		const pageTargets = [...chapters, ...partTargets].sort((left, right) => {
			if (left.element === right.element) return 0;
			return left.element.compareDocumentPosition(right.element) & Node.DOCUMENT_POSITION_FOLLOWING
				? -1
				: 1;
		});
		const pageTargetById = new Map(pageTargets.map((target) => [target.id, target]));

		const findChapterForPath = (pathname) => {
			const normalizedPath = normalizePath(pathname);
			if (!normalizedPath) return null;
			const directMatch = pathToChapter.get(normalizedPath);
			if (directMatch) return directMatch;
			const finalSegment = normalizedPath.split("/").filter(Boolean).at(-1);
			if (!finalSegment) return null;
			return (
				chapters.find((chapter) =>
					[
						chapter.id,
						chapter.element.dataset.chapterSlug,
						chapter.element.dataset.slug,
						chapter.element.dataset.entryId,
					]
						.filter(Boolean)
						.some((value) => slugify(value).endsWith(slugify(finalSegment))),
				) ?? null
			);
		};

		let rewrittenContentLinks = 0;
		for (const chapter of chapters) {
			for (const link of chapter.element.querySelectorAll("a[href]")) {
				if (!(link instanceof HTMLAnchorElement)) continue;
				const rawHref = link.getAttribute("href") ?? "";
				if (!rawHref || /^(mailto:|tel:|javascript:)/i.test(rawHref)) continue;

				if (rawHref.startsWith("#")) {
					const oldTarget = decodeURIComponent(rawHref.slice(1));
					const localTarget = chapter.localIdMap.get(oldTarget);
					if (localTarget) {
						link.href = `#${localTarget}`;
						rewrittenContentLinks += 1;
						continue;
					}
					const targetChapter = pageTargetById.get(oldTarget);
					if (targetChapter) {
						link.href = `${internalLinkPrefix}${encodeURIComponent(targetChapter.id)}`;
						link.dataset.pdfTarget = targetChapter.id;
						rewrittenContentLinks += 1;
					}
					continue;
				}

				let url;
				try {
					url = new URL(rawHref, location.href);
				} catch {
					continue;
				}
				const targetChapter = findChapterForPath(url.pathname);
				if (targetChapter) {
					link.href = `${internalLinkPrefix}${encodeURIComponent(targetChapter.id)}`;
					link.dataset.pdfTarget = targetChapter.id;
					rewrittenContentLinks += 1;
				} else if (url.origin === location.origin) {
					link.href = `${canonicalOrigin}${url.pathname}${url.search}${url.hash}`;
					rewrittenContentLinks += 1;
				}
			}
		}

		let tocEntries = 0;
		for (const link of front.querySelectorAll("a[href], [data-toc-target]")) {
			if (!(link instanceof HTMLAnchorElement)) continue;
			const rawTarget = link.dataset.tocTarget?.trim();
			const rawHref = link.getAttribute("href") ?? "";
			let targetChapter = null;
			if (rawTarget) {
				const targetId = rawTarget.replace(/^#/, "");
				targetChapter = pageTargetById.get(targetId) ?? findChapterForPath(rawTarget);
			}
			if (!targetChapter && rawHref.startsWith("#")) {
				const targetId = decodeURIComponent(rawHref.slice(1));
				targetChapter = pageTargetById.get(targetId) ?? null;
			}
			if (!targetChapter && rawHref) {
				try {
					targetChapter = findChapterForPath(new URL(rawHref, location.href).pathname);
				} catch {
					// Text matching below is the final fallback.
				}
			}
			if (!targetChapter) {
				targetChapter = titleToChapter.get(normalizeText(link.textContent ?? "")) ?? null;
			}
			if (!targetChapter) continue;

			link.dataset.pdfTarget = targetChapter.id;
			link.href = `${internalLinkPrefix}${encodeURIComponent(targetChapter.id)}`;
			const entryContainer =
				link.closest("[data-toc-entry], li, .toc-entry, tr") ?? link.parentElement;
			if (entryContainer) {
				let pageLabel = entryContainer.querySelector("[data-toc-page], .toc-page");
				if (!(pageLabel instanceof HTMLElement)) {
					pageLabel = document.createElement("span");
					pageLabel.className = "toc-page";
					pageLabel.dataset.tocPage = "";
					entryContainer.append(pageLabel);
				} else {
					pageLabel.dataset.tocPage = "";
				}
				pageLabel.dataset.pdfTarget = targetChapter.id;
			}
			tocEntries += 1;
		}

		let wideCodeBlocks = 0;
		let veryWideCodeBlocks = 0;
		for (const pre of body.querySelectorAll("pre")) {
			const renderedLines = Array.from(pre.querySelectorAll(".ec-line .code"));
			const lines = (
				renderedLines.length > 0
					? renderedLines.map((line) => line.textContent ?? "")
					: (pre.textContent ?? "").replace(/\r/g, "").split("\n")
			).map((line) => line.replace(/\t/g, "    "));
			const widths = lines.map((line) => Array.from(line).length);
			const maximumWidth = Math.max(0, ...widths);
			const oversizedLines = widths.filter((width) => width > 92).length;
			const printableBlock = pre.closest(".expressive-code") ?? pre;
			pre.dataset.pdfMaxColumns = String(maximumWidth);
			if (maximumWidth > 124 || oversizedLines > Math.max(3, lines.length * 0.2)) {
				printableBlock.classList.add("pdf-code-very-wide");
				veryWideCodeBlocks += 1;
			} else if (maximumWidth > 84) {
				printableBlock.classList.add("pdf-code-wide");
				wideCodeBlocks += 1;
			}
		}

		let convertedTables = 0;
		for (const table of Array.from(body.querySelectorAll("table"))) {
			if (
				!(table instanceof HTMLTableElement) ||
				table.matches("[data-pdf-keep-table], .pdf-keep-table") ||
				table.querySelector("table")
			) {
				continue;
			}
			const allRows = Array.from(table.rows);
			if (allRows.length < 2) continue;
			const headerRow = table.tHead?.rows[0] ?? allRows.find((row) => row.querySelector("th"));
			const columnCount = Math.max(0, ...allRows.map((row) => row.cells.length));
			const dataRows = allRows.filter((row) => row !== headerRow);
			const totalCharacters = dataRows.reduce(
				(sum, row) => sum + (row.textContent?.trim().length ?? 0),
				0,
			);
			const averageCharacters = dataRows.length > 0 ? totalCharacters / dataRows.length : 0;
			const isDense =
				columnCount >= 5 ||
				(columnCount >= 4 && dataRows.length >= 3) ||
				(columnCount >= 3 && dataRows.length >= 6 && averageCharacters >= 90);
			if (!isDense || dataRows.length === 0) continue;

			const headers = Array.from(headerRow?.cells ?? []).map(
				(cell, columnIndex) => cell.textContent?.trim() || `Column ${columnIndex + 1}`,
			);
			while (headers.length < columnCount) headers.push(`Column ${headers.length + 1}`);

			const cards = document.createElement("section");
			cards.className = "pdf-table-cards";
			cards.dataset.sourceColumns = String(columnCount);
			cards.dataset.sourceRows = String(dataRows.length);
			cards.setAttribute("role", "group");
			const caption = table.caption?.textContent?.trim();
			cards.setAttribute("aria-label", caption || "Table presented as readable cards");

			for (const row of dataRows) {
				const cells = Array.from(row.cells);
				if (cells.length === 0) continue;
				const card = document.createElement("article");
				card.className = "pdf-table-card";

				const firstValue = cells[0]?.textContent?.trim();
				if (firstValue) {
					const cardTitle = document.createElement("div");
					cardTitle.className = "pdf-table-card__title";
					cardTitle.append(...Array.from(cells[0].childNodes).map((node) => node.cloneNode(true)));
					card.append(cardTitle);
				}

				const fields = document.createElement("dl");
				fields.className = "pdf-table-card__fields";
				cells.forEach((cell, columnIndex) => {
					if (columnIndex === 0 && firstValue) return;
					const label = document.createElement("dt");
					label.className = "pdf-table-card__label";
					label.textContent = headers[columnIndex] ?? `Column ${columnIndex + 1}`;
					const value = document.createElement("dd");
					value.className = "pdf-table-card__value";
					value.append(...Array.from(cell.childNodes).map((node) => node.cloneNode(true)));
					fields.append(label, value);
				});
				card.append(fields);
				cards.append(card);
			}

			table.replaceWith(cards);
			convertedTables += 1;
		}

		return {
			chapters: chapters.map(({ id, title, marker, paths }) => ({ id, title, marker, paths })),
			targets: pageTargets.map(({ id, kind, title, marker, paths }) => ({
				id,
				kind,
				title,
				marker,
				paths,
			})),
			stats: {
				convertedTables,
				rewrittenContentLinks,
				tocEntries,
				veryWideCodeBlocks,
				wideCodeBlocks,
			},
		};
	}, INTERNAL_LINK_PREFIX);
}

async function setPrintPass(page, pass) {
	await page.evaluate((nextPass) => {
		document.documentElement.dataset.pdfPass = nextPass;
		const front = document.querySelector("[data-pdf-front], .pdf-front, .print-front");
		const body = document.querySelector("[data-pdf-body], .pdf-body, .print-body");
		if (front instanceof HTMLElement) {
			front.toggleAttribute("aria-hidden", nextPass === "body");
		}
		if (body instanceof HTMLElement) {
			body.toggleAttribute("aria-hidden", nextPass === "front");
		}
		window.scrollTo(0, 0);
	}, pass);
	await page.evaluate(() => new Promise((resolveFrame) => requestAnimationFrame(() => requestAnimationFrame(resolveFrame))));
}

async function removeDiscoveryMarkers(page) {
	await page.evaluate(() => {
		for (const marker of document.querySelectorAll("[data-pdf-page-marker]")) marker.remove();
	});
}

async function injectTocPageNumbers(page, bodyPageByChapterId, frontPageCount) {
	return page.evaluate(
		({ pageNumbers, offset }) => {
			let updated = 0;
			for (const label of document.querySelectorAll("[data-toc-page], .toc-page")) {
				if (!(label instanceof HTMLElement)) continue;
				const target = label.dataset.pdfTarget;
				const bodyPage = target ? pageNumbers[target] : undefined;
				if (!Number.isInteger(bodyPage)) continue;
				label.textContent = String(offset + bodyPage);
				label.setAttribute("aria-label", `Page ${offset + bodyPage}`);
				updated += 1;
			}
			return updated;
		},
		{ pageNumbers: bodyPageByChapterId, offset: frontPageCount },
	);
}

async function renderPdf(page, path, options) {
	await waitForDocumentAssets(page);
	await page.pdf({
		path,
		format: "A4",
		printBackground: true,
		preferCSSPageSize: true,
		tagged: true,
		outline: true,
		waitForFonts: true,
		timeout: 120_000,
		...options,
	});
}

function normalizeExtractedText(value) {
	return value
		.normalize("NFKD")
		.toLowerCase()
		.replace(/[^a-z0-9]+/g, " ")
		.trim();
}

async function discoverChapterPages(bodyPdfPath, extractedTextPath, chapters) {
	const extraction = spawnSync(
		"pdftotext",
		["-layout", "-enc", "UTF-8", bodyPdfPath, extractedTextPath],
		{ encoding: "utf8" },
	);
	if (extraction.error?.code === "ENOENT") {
		throw new Error(
			"`pdftotext` is required to discover chapter pages. Install Poppler and rerun the export.",
		);
	}
	if (extraction.status !== 0) {
		throw new Error(
			`pdftotext failed (${extraction.status ?? "unknown status"}): ${extraction.stderr?.trim() || "no diagnostic output"}`,
		);
	}

	const extractedText = (await readFile(extractedTextPath, "utf8")).replace(/\r/g, "");
	const pages = extractedText.split("\f");
	const pageByChapterId = {};
	const missingChapters = [];

	for (const chapter of chapters) {
		let pageIndex = pages.findIndex((pageText) => pageText.includes(chapter.marker));
		if (pageIndex < 0) {
			const normalizedTitle = normalizeExtractedText(chapter.title);
			pageIndex = pages.findIndex((pageText) =>
				normalizeExtractedText(pageText).includes(normalizedTitle),
			);
		}
		if (pageIndex < 0) {
			missingChapters.push(chapter.title);
			continue;
		}
		pageByChapterId[chapter.id] = pageIndex + 1;
	}

	if (missingChapters.length > 0) {
		throw new Error(
			`Could not discover PDF page numbers for: ${missingChapters.join(", ")}. Ensure chapter wrappers begin on printable pages and do not hide their content.`,
		);
	}

	return { pageByChapterId, extractedPageCount: pages.filter((page) => page.length > 0).length };
}

async function readPdfPageCount(PDFDocument, path) {
	const document = await PDFDocument.load(await readFile(path), { ignoreEncryption: true });
	return document.getPageCount();
}

function findInternalLinkTarget(uri) {
	if (!uri.startsWith(INTERNAL_LINK_PREFIX)) return null;
	const encodedTarget = uri.slice(INTERNAL_LINK_PREFIX.length).split(/[?#]/, 1)[0];
	try {
		return decodeURIComponent(encodedTarget);
	} catch {
		return encodedTarget;
	}
}

function rewriteInternalPdfLinks(pdfDocument, pdfLib, targetPageIndexById) {
	const { PDFArray, PDFDict, PDFHexString, PDFName, PDFString } = pdfLib;
	const annotationName = PDFName.of("Annots");
	const actionName = PDFName.of("A");
	const uriName = PDFName.of("URI");
	const destinationName = PDFName.of("Dest");
	let rewritten = 0;

	for (const page of pdfDocument.getPages()) {
		const annotations = page.node.lookupMaybe(annotationName, PDFArray);
		if (!annotations) continue;
		for (let index = 0; index < annotations.size(); index += 1) {
			const annotation = annotations.lookupMaybe(index, PDFDict);
			const action = annotation?.lookupMaybe(actionName, PDFDict);
			const uriObject = action?.lookupMaybe(uriName, PDFString, PDFHexString);
			if (!annotation || !uriObject) continue;
			const targetId = findInternalLinkTarget(uriObject.decodeText());
			if (!targetId) continue;
			const targetPageIndex = targetPageIndexById.get(targetId);
			if (!Number.isInteger(targetPageIndex)) continue;
			const targetPage = pdfDocument.getPage(targetPageIndex);
			annotation.delete(actionName);
			annotation.set(
				destinationName,
				pdfDocument.context.obj([targetPage.ref, "FitH", null]),
			);
			rewritten += 1;
		}
	}

	return rewritten;
}

function addChapterBookmarks(pdfDocument, pdfLib, chapters, targetPageIndexById) {
	const { PDFHexString, PDFName } = pdfLib;
	const context = pdfDocument.context;
	const outlineRoot = context.obj({ Type: "Outlines", Count: 0 });
	const outlineRootReference = context.register(outlineRoot);
	const bookmarkEntries = chapters
		.map((chapter) => ({
			chapter,
			pageIndex: targetPageIndexById.get(chapter.id),
		}))
		.filter(({ pageIndex }) => Number.isInteger(pageIndex));
	if (bookmarkEntries.length === 0) return 0;

	const itemReferences = bookmarkEntries.map(({ chapter, pageIndex }) => {
		const targetPage = pdfDocument.getPage(pageIndex);
		return context.register(
			context.obj({
				Title: PDFHexString.fromText(chapter.title),
				Parent: outlineRootReference,
				Dest: context.obj([targetPage.ref, "FitH", null]),
			}),
		);
	});

	for (let index = 0; index < itemReferences.length; index += 1) {
		const item = context.lookup(itemReferences[index]);
		if (index > 0) item.set(PDFName.of("Prev"), itemReferences[index - 1]);
		if (index < itemReferences.length - 1) {
			item.set(PDFName.of("Next"), itemReferences[index + 1]);
		}
	}
	outlineRoot.set(PDFName.of("First"), itemReferences[0]);
	outlineRoot.set(PDFName.of("Last"), itemReferences.at(-1));
	outlineRoot.set(PDFName.of("Count"), context.obj(itemReferences.length));
	pdfDocument.catalog.set(PDFName.of("Outlines"), outlineRootReference);
	pdfDocument.catalog.set(PDFName.of("PageMode"), PDFName.of("UseOutlines"));
	return itemReferences.length;
}

async function mergePdfSections({
	pdfLib,
	frontPdfPath,
	bodyPdfPath,
	mergedPdfPath,
	targets,
	bodyPageByTargetId,
}) {
	const { PDFDocument } = pdfLib;
	const [frontDocument, bodyDocument] = await Promise.all([
		PDFDocument.load(await readFile(frontPdfPath), { ignoreEncryption: true }),
		PDFDocument.load(await readFile(bodyPdfPath), { ignoreEncryption: true }),
	]);
	const mergedDocument = await PDFDocument.create();
	const frontPageCount = frontDocument.getPageCount();
	const bodyPageCount = bodyDocument.getPageCount();
	const copiedFrontPages = await mergedDocument.copyPages(
		frontDocument,
		frontDocument.getPageIndices(),
	);
	for (const page of copiedFrontPages) mergedDocument.addPage(page);
	const copiedBodyPages = await mergedDocument.copyPages(
		bodyDocument,
		bodyDocument.getPageIndices(),
	);
	for (const page of copiedBodyPages) mergedDocument.addPage(page);

	const targetPageIndexById = new Map(
		targets.map((target) => [
			target.id,
			frontPageCount + bodyPageByTargetId[target.id] - 1,
		]),
	);
	const rewrittenLinks = rewriteInternalPdfLinks(
		mergedDocument,
		pdfLib,
		targetPageIndexById,
	);
	const bookmarks = addChapterBookmarks(
		mergedDocument,
		pdfLib,
		targets,
		targetPageIndexById,
	);

	const sourceDate = process.env.SOURCE_DATE_EPOCH
		? new Date(Number(process.env.SOURCE_DATE_EPOCH) * 1000)
		: new Date();
	mergedDocument.setTitle("FlowBook: The FlowScript Book", {
		showInWindowTitleBar: true,
	});
	mergedDocument.setAuthor("Flow-Like");
	mergedDocument.setSubject(
		"Build reliable software in code and as a visible workflow.",
	);
	mergedDocument.setKeywords([
		"FlowBook",
		"FlowScript",
		"Flow-Like",
		"workflow",
		"software engineering",
	]);
	mergedDocument.setLanguage("en");
	mergedDocument.setCreator("FlowBook PDF exporter");
	mergedDocument.setProducer("Flow-Like");
	mergedDocument.setCreationDate(sourceDate);
	mergedDocument.setModificationDate(sourceDate);

	await writeFile(
		mergedPdfPath,
		await mergedDocument.save({ addDefaultPage: false, objectsPerTick: 50 }),
	);
	return {
		bookmarks,
		bodyPageCount,
		frontPageCount,
		rewrittenLinks,
		totalPageCount: mergedDocument.getPageCount(),
	};
}

async function main() {
	const options = parseArguments(process.argv.slice(2));
	await ensureBuiltPrintRoute(options.distDirectory, options.printRoute);
	const { puppeteer, pdfLib } = await loadDependencies();

	const tempRoot = resolve(repositoryDirectory, "tmp/pdfs");
	await mkdir(tempRoot, { recursive: true });
	const tempDirectory = await mkdtemp(join(tempRoot, "flowbook-export-"));
	const discoveryPdfPath = resolve(tempDirectory, "body-discovery.pdf");
	const discoveryTextPath = resolve(tempDirectory, "body-discovery.txt");
	const bodyPdfPath = resolve(tempDirectory, "body.pdf");
	const frontPdfPath = resolve(tempDirectory, "front.pdf");
	const mergedPdfPath = resolve(tempDirectory, "flowbook.pdf");

	let browser;
	let staticServer;
	try {
		console.log(`Serving ${formatRelativePath(options.distDirectory)} for PDF export...`);
		staticServer = await startStaticServer(options.distDirectory);
		browser = await puppeteer.launch({
			headless: true,
			protocolTimeout: 180_000,
			args: [
				"--force-color-profile=srgb",
				"--disable-background-timer-throttling",
				"--disable-renderer-backgrounding",
				"--disable-dev-shm-usage",
				"--no-sandbox",
				"--disable-setuid-sandbox",
			],
		});
		const page = await browser.newPage();
		await page.setViewport({ width: 1440, height: 1000, deviceScaleFactor: 1 });
		await page.emulateMediaType("print");
		await page.emulateMediaFeatures([
			{ name: "prefers-color-scheme", value: "light" },
			{ name: "prefers-reduced-motion", value: "reduce" },
		]);

		const pageErrors = [];
		page.on("pageerror", (error) => pageErrors.push(error.message));
		const printUrl = `${staticServer.origin}${options.printRoute}`;
		const response = await page.goto(printUrl, {
			waitUntil: ["domcontentloaded", "networkidle0"],
			timeout: 120_000,
		});
		if (!response?.ok()) {
			throw new Error(
				`Print route returned HTTP ${response?.status() ?? "unknown"}: ${printUrl}`,
			);
		}
		await waitForDocumentAssets(page);
		if (pageErrors.length > 0) {
			throw new Error(`Print page runtime errors:\n- ${pageErrors.join("\n- ")}`);
		}

		const printDocument = await normalizePrintDocument(page);
		console.log(
			`Prepared ${printDocument.targets.length} contents targets across ${printDocument.chapters.length} chapters; ` +
				`${printDocument.stats.convertedTables} dense tables converted, ` +
				`${printDocument.stats.wideCodeBlocks} wide and ${printDocument.stats.veryWideCodeBlocks} very-wide code blocks classified.`,
		);

		await setPrintPass(page, "body");
		console.log("Rendering body discovery pass...");
		await renderPdf(page, discoveryPdfPath, {
			displayHeaderFooter: false,
		});
		const { pageByChapterId: pageByTargetId } = await discoverChapterPages(
			discoveryPdfPath,
			discoveryTextPath,
			printDocument.targets,
		);

		await removeDiscoveryMarkers(page);
		console.log("Rendering clean body...");
		await renderPdf(page, bodyPdfPath, {
			displayHeaderFooter: false,
		});
		const PDFDocument = pdfLib.PDFDocument;
		const discoveryPageCount = await readPdfPageCount(PDFDocument, discoveryPdfPath);
		const bodyPageCount = await readPdfPageCount(PDFDocument, bodyPdfPath);
		if (discoveryPageCount !== bodyPageCount) {
			throw new Error(
				`Removing page markers changed body pagination (${discoveryPageCount} to ${bodyPageCount} pages).`,
			);
		}

		await setPrintPass(page, "front");
		const updatedEntries = await injectTocPageNumbers(page, pageByTargetId, 0);
		if (updatedEntries < printDocument.targets.length) {
			throw new Error(
				`Only ${updatedEntries} of ${printDocument.targets.length} front-matter TOC entries matched printable sections. Add data-toc-target or href links for every part and chapter.`,
			);
		}
		console.log("Rendering front matter with body-local page numbers...");
		await renderPdf(page, frontPdfPath, {
			displayHeaderFooter: false,
		});
		const finalFrontPageCount = await readPdfPageCount(PDFDocument, frontPdfPath);

		console.log(
			`Merging ${finalFrontPageCount} front pages with ${bodyPageCount} body pages...`,
		);
		const mergeResult = await mergePdfSections({
			pdfLib,
			frontPdfPath,
			bodyPdfPath,
			mergedPdfPath,
			targets: printDocument.targets,
			bodyPageByTargetId: pageByTargetId,
		});

		await mkdir(dirname(options.outputPath), { recursive: true });
		const stagingOutput = `${options.outputPath}.tmp-${process.pid}`;
		await writeFile(stagingOutput, await readFile(mergedPdfPath));
		await rename(stagingOutput, options.outputPath);
		console.log(
			`Created ${formatRelativePath(options.outputPath)}: ` +
				`${mergeResult.totalPageCount} pages, ${mergeResult.bookmarks} bookmarks, ` +
				`${mergeResult.rewrittenLinks} internal links repaired.`,
		);
	} finally {
		if (browser) await browser.close().catch(() => {});
		if (staticServer?.server) await closeServer(staticServer.server).catch(() => {});
		if (options.keepTemp) {
			console.log(`Kept intermediates at ${formatRelativePath(tempDirectory)}.`);
		} else if (isPathInside(tempRoot, tempDirectory)) {
			await rm(tempDirectory, { recursive: true, force: true });
		}
	}
}

main().catch((error) => {
	console.error(error instanceof Error ? error.stack ?? error.message : String(error));
	process.exitCode = 1;
});
