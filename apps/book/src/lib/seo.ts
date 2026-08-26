import { CURRENT_BOOK_EDITION } from "./book-edition";

export const BOOK_ORIGIN = "https://book.flow-like.com";
export const FLOW_LIKE_ORIGIN = "https://flow-like.com";
export const BOOK_NAME = "FlowBook";
export const BOOK_LANGUAGE = "en";
export const BOOK_OG_LOCALE = "en_US";
export const FLOW_LIKE_ORGANIZATION_ID = `${FLOW_LIKE_ORIGIN}/#organization`;
export const BOOK_WEBSITE_ID = `${BOOK_ORIGIN}/#website`;
export const BOOK_ID = `${BOOK_ORIGIN}/#book`;

const DEFAULT_TOPICS = [
	"FlowScript",
	"visual workflow programming",
	"typed workflows",
	"Flow-Like",
] as const;

export interface BookSeoFrontmatter {
	readonly title?: string;
	readonly topics?: readonly string[];
	readonly imageAlt?: string;
}

export interface BookPageData {
	readonly title: string;
	readonly description?: string;
	readonly seo?: BookSeoFrontmatter;
}

export interface BookBreadcrumb {
	readonly name: string;
	readonly path: string;
}

export interface BookEditionLocation {
	readonly kind: "introduction" | "chapter";
	readonly number?: number;
	readonly part?: (typeof CURRENT_BOOK_EDITION.parts)[number];
}

export interface ResolvedBookSeo {
	readonly entryId: string;
	readonly path: string;
	readonly pageType:
		| "home"
		| "contents"
		| "part"
		| "reading"
		| "not-found"
		| "page";
	readonly title: string;
	readonly documentTitle: string;
	readonly description: string;
	readonly topics: readonly string[];
	readonly imagePath: string;
	readonly imageAlt: string;
	readonly breadcrumbs: readonly BookBreadcrumb[];
	readonly location?: BookEditionLocation;
	readonly part?: (typeof CURRENT_BOOK_EDITION.parts)[number];
}

export function normalizeBookEntryId(entryId: string): string {
	return entryId === "index" ? "" : entryId.replace(/^\/+|\/+$/g, "");
}

export function bookEntryPath(entryId: string): string {
	const normalized = normalizeBookEntryId(entryId);
	return normalized ? `/${normalized}/` : "/";
}

export function bookSocialImagePath(entryId: string): string {
	const normalized = normalizeBookEntryId(entryId);
	if (normalized === "404" || normalized.endsWith("/404")) return "/og.png";
	return `/social/${normalized || "index"}.png`;
}

export function getBookEditionLocation(
	entryId: string,
): BookEditionLocation | undefined {
	const normalized = normalizeBookEntryId(entryId);
	if (normalized === CURRENT_BOOK_EDITION.introduction.entryId) {
		return { kind: "introduction" };
	}

	for (const part of CURRENT_BOOK_EDITION.parts) {
		const chapter = part.chapters.find(
			(candidate) => candidate.entryId === normalized,
		);
		if (chapter) {
			return { kind: "chapter", number: chapter.number, part };
		}
	}

	return undefined;
}

export function getBookEditionPart(
	entryId: string,
): (typeof CURRENT_BOOK_EDITION.parts)[number] | undefined {
	const normalized = normalizeBookEntryId(entryId);
	return CURRENT_BOOK_EDITION.parts.find((part) => part.id === normalized);
}

function withoutChapterNumber(title: string): string {
	return title.replace(/^\d+\.\s*/, "");
}

export function getBookBreadcrumbs(
	entryId: string,
	title: string,
): readonly BookBreadcrumb[] {
	const normalized = normalizeBookEntryId(entryId);
	if (!normalized || normalized === "404" || normalized.endsWith("/404")) {
		return [];
	}

	const home = { name: BOOK_NAME, path: "/" };
	if (normalized === "contents") {
		return [home, { name: "Contents", path: "/contents/" }];
	}

	const location = getBookEditionLocation(normalized);
	const part = getBookEditionPart(normalized);
	if (location?.kind === "chapter") {
		return [
			home,
			{
				name: `${location.part?.label}: ${location.part?.title}`,
				path: bookEntryPath(location.part?.id ?? "contents"),
			},
			{
				name: `Chapter ${location.number}: ${withoutChapterNumber(title)}`,
				path: bookEntryPath(normalized),
			},
		];
	}
	if (part) {
		return [
			home,
			{
				name: `${part.label}: ${part.title}`,
				path: bookEntryPath(normalized),
			},
		];
	}

	return [
		home,
		{ name: withoutChapterNumber(title), path: bookEntryPath(normalized) },
	];
}

export function resolveBookSeo(
	entryId: string,
	data: BookPageData,
): ResolvedBookSeo {
	const normalized = normalizeBookEntryId(entryId);
	const location = getBookEditionLocation(normalized);
	const part = getBookEditionPart(normalized);
	const pageType =
		normalized === "404" || normalized.endsWith("/404")
			? "not-found"
			: !normalized
				? "home"
				: normalized === "contents"
					? "contents"
					: part
						? "part"
						: location
							? "reading"
							: "page";
	const title =
		pageType === "not-found"
			? "Page not found"
			: (data.seo?.title ?? data.title);
	const documentTitle = data.seo?.title
		? data.seo.title
		: title.includes(BOOK_NAME)
			? title
			: `${title} | ${BOOK_NAME}`;
	const description =
		pageType === "not-found"
			? "The requested FlowBook page could not be found. Return to the book or start reading the open edition."
			: (data.description ?? CURRENT_BOOK_EDITION.description);
	const topics =
		data.seo?.topics && data.seo.topics.length > 0
			? data.seo.topics
			: DEFAULT_TOPICS;
	const imageAlt =
		data.seo?.imageAlt ??
		(location?.kind === "chapter"
			? `FlowBook chapter ${location.number}: ${withoutChapterNumber(data.title)}`
			: `${title} | ${CURRENT_BOOK_EDITION.subtitle}`);

	return {
		entryId: normalized,
		path: bookEntryPath(normalized),
		pageType,
		title,
		documentTitle,
		description,
		topics,
		imagePath: bookSocialImagePath(normalized),
		imageAlt,
		breadcrumbs: getBookBreadcrumbs(normalized, data.title),
		location,
		part,
	};
}

function absoluteUrl(path: string): string {
	return new URL(path, BOOK_ORIGIN).toString();
}

function editionEntryIds(
	part?: (typeof CURRENT_BOOK_EDITION.parts)[number],
): readonly string[] {
	if (part) return part.chapters.map((chapter) => chapter.entryId);
	return [
		CURRENT_BOOK_EDITION.introduction.entryId,
		...CURRENT_BOOK_EDITION.parts.flatMap((part) =>
			part.chapters.map((chapter) => chapter.entryId),
		),
	];
}

function chapterStructuredData(
	seo: ResolvedBookSeo,
	data: BookPageData,
	canonical: string,
	imageId: string,
): Record<string, unknown> | undefined {
	if (!seo.location) return undefined;

	return {
		"@type": "Chapter",
		"@id": `${canonical}#chapter`,
		url: canonical,
		name: data.title,
		alternativeHeadline: seo.title,
		description: seo.description,
		position:
			seo.location.kind === "chapter" ? seo.location.number : "Introduction",
		isPartOf: { "@id": BOOK_ID },
		mainEntityOfPage: { "@id": `${canonical}#webpage` },
		inLanguage: BOOK_LANGUAGE,
		isAccessibleForFree: true,
		keywords: seo.topics.join(", "),
		image: { "@id": imageId },
		publisher: { "@id": FLOW_LIKE_ORGANIZATION_ID },
	};
}

function contentsItemList(seo: ResolvedBookSeo): Record<string, unknown> {
	const entries = editionEntryIds(seo.part);
	const id = seo.part
		? `${BOOK_ORIGIN}/${seo.part.id}/#chapters`
		: `${BOOK_ORIGIN}/contents/#chapters`;
	return {
		"@type": "ItemList",
		"@id": id,
		name: seo.part
			? `${seo.part.label}: ${seo.part.title}`
			: `${BOOK_NAME} chapters`,
		numberOfItems: entries.length,
		itemListOrder: "https://schema.org/ItemListOrderAscending",
		itemListElement: entries.map((entryId, index) => ({
			"@type": "ListItem",
			position: index + 1,
			url: absoluteUrl(bookEntryPath(entryId)),
			item: { "@id": `${absoluteUrl(bookEntryPath(entryId))}#chapter` },
		})),
	};
}

export function buildBookStructuredData(
	seo: ResolvedBookSeo,
	data: BookPageData,
	canonical: string,
	imageUrl: string,
): Record<string, unknown> | undefined {
	if (seo.pageType === "not-found") return undefined;

	const webpageId = `${canonical}#webpage`;
	const imageId = `${canonical}#primaryimage`;
	const breadcrumbId = `${canonical}#breadcrumb`;
	const chapter = chapterStructuredData(seo, data, canonical, imageId);
	const itemList =
		seo.pageType === "contents" || seo.pageType === "part"
			? contentsItemList(seo)
			: undefined;
	const mainEntityId = chapter
		? `${canonical}#chapter`
		: itemList
			? String(itemList["@id"])
			: BOOK_ID;

	const graph: Record<string, unknown>[] = [
		{
			"@type": "Organization",
			"@id": FLOW_LIKE_ORGANIZATION_ID,
			name: CURRENT_BOOK_EDITION.publisher,
			url: FLOW_LIKE_ORIGIN,
			logo: {
				"@type": "ImageObject",
				url: absoluteUrl("/favicon.svg"),
				width: 1024,
				height: 1024,
			},
		},
		{
			"@type": "WebSite",
			"@id": BOOK_WEBSITE_ID,
			url: `${BOOK_ORIGIN}/`,
			name: BOOK_NAME,
			alternateName: "FlowBook: A Developer's Guide to Flow-Like",
			description: CURRENT_BOOK_EDITION.description,
			inLanguage: BOOK_LANGUAGE,
			publisher: { "@id": FLOW_LIKE_ORGANIZATION_ID },
		},
		{
			"@type": "Book",
			"@id": BOOK_ID,
			url: `${BOOK_ORIGIN}/`,
			name: CURRENT_BOOK_EDITION.title,
			alternateName: CURRENT_BOOK_EDITION.subtitle,
			description: CURRENT_BOOK_EDITION.description,
			bookEdition: CURRENT_BOOK_EDITION.editionLabel,
			bookFormat: "https://schema.org/EBook",
			inLanguage: CURRENT_BOOK_EDITION.language,
			copyrightYear: CURRENT_BOOK_EDITION.year,
			isAccessibleForFree: true,
			image: absoluteUrl("/social/index.png"),
			genre: ["Software development", "Workflow automation"],
			about: [
				{ "@type": "Thing", name: "Flow-Like FlowScript" },
				{ "@type": "Thing", name: "Visual workflow programming" },
			],
			publisher: { "@id": FLOW_LIKE_ORGANIZATION_ID },
			hasPart: editionEntryIds().map((entryId) => ({
				"@id": `${absoluteUrl(bookEntryPath(entryId))}#chapter`,
			})),
			encoding: {
				"@type": "MediaObject",
				name: `${CURRENT_BOOK_EDITION.title} PDF edition`,
				contentUrl: absoluteUrl("/flowbook.pdf"),
				encodingFormat: "application/pdf",
			},
		},
		{
			"@type":
				seo.pageType === "contents" || seo.pageType === "part"
					? "CollectionPage"
					: "WebPage",
			"@id": webpageId,
			url: canonical,
			name: seo.title,
			description: seo.description,
			inLanguage: BOOK_LANGUAGE,
			isPartOf: { "@id": BOOK_WEBSITE_ID },
			about: { "@id": BOOK_ID },
			mainEntity: { "@id": mainEntityId },
			primaryImageOfPage: { "@id": imageId },
			...(seo.breadcrumbs.length > 0
				? { breadcrumb: { "@id": breadcrumbId } }
				: {}),
		},
		{
			"@type": "ImageObject",
			"@id": imageId,
			url: imageUrl,
			contentUrl: imageUrl,
			width: 1200,
			height: 630,
			caption: seo.imageAlt,
			inLanguage: BOOK_LANGUAGE,
		},
	];

	if (chapter) graph.push(chapter);
	if (itemList) graph.push(itemList);
	if (seo.breadcrumbs.length > 0) {
		graph.push({
			"@type": "BreadcrumbList",
			"@id": breadcrumbId,
			itemListElement: seo.breadcrumbs.map((breadcrumb, index) => ({
				"@type": "ListItem",
				position: index + 1,
				name: breadcrumb.name,
				item: absoluteUrl(breadcrumb.path),
			})),
		});
	}

	return { "@context": "https://schema.org", "@graph": graph };
}

export function serializeStructuredData(
	value: Record<string, unknown>,
): string {
	return JSON.stringify(value).replace(/</g, "\\u003c");
}
