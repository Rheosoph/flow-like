import { getCollection } from "astro:content";
import type { APIRoute, GetStaticPaths } from "astro";
import sharp from "sharp";
import { CURRENT_BOOK_EDITION } from "../../lib/book-edition";
import { normalizeBookEntryId, resolveBookSeo } from "../../lib/seo";

interface SocialCardProps {
	readonly title: string;
	readonly eyebrow: string;
	readonly topics: readonly string[];
}

const WIDTH = 1200;
const HEIGHT = 630;

function escapeXml(value: string): string {
	return value
		.replaceAll("&", "&amp;")
		.replaceAll("<", "&lt;")
		.replaceAll(">", "&gt;")
		.replaceAll('"', "&quot;")
		.replaceAll("'", "&apos;");
}

function truncate(value: string, maximumLength: number): string {
	if (value.length <= maximumLength) return value;
	return `${value.slice(0, Math.max(0, maximumLength - 1)).trimEnd()}…`;
}

function wrapTitle(value: string): readonly string[] {
	const words = value.split(/\s+/).filter(Boolean);
	const lines: string[] = [];
	let current = "";

	for (const word of words) {
		const candidate = current ? `${current} ${word}` : word;
		if (candidate.length <= 30 || !current) {
			current = candidate;
			continue;
		}

		lines.push(current);
		current = word;
		if (lines.length === 2) break;
	}

	if (current && lines.length < 3) lines.push(current);
	const consumed = lines.join(" ").split(/\s+/).length;
	if (consumed < words.length && lines.length > 0) {
		lines[lines.length - 1] = truncate(lines.at(-1) ?? "", 28);
	}

	return lines.slice(0, 3);
}

function cardEyebrow(entryId: string): string {
	const normalized = normalizeBookEntryId(entryId);
	if (!normalized) return "FLOW-LIKE DEVELOPER GUIDE";
	if (normalized === "contents") return "OPEN EDITION / CONTENTS";
	if (normalized === CURRENT_BOOK_EDITION.introduction.entryId) {
		return "OPEN EDITION / INTRODUCTION";
	}
	const editionPart = CURRENT_BOOK_EDITION.parts.find(
		(part) => part.id === normalized,
	);
	if (editionPart) return `${editionPart.label.toUpperCase()} / PART OVERVIEW`;

	for (const part of CURRENT_BOOK_EDITION.parts) {
		const chapter = part.chapters.find((item) => item.entryId === normalized);
		if (chapter) {
			return `${part.label.toUpperCase()} / CHAPTER ${String(chapter.number).padStart(2, "0")}`;
		}
	}

	return "FLOWBOOK / FLOW-LIKE DEVELOPER GUIDE";
}

function renderCardSvg(props: SocialCardProps): string {
	const lines = wrapTitle(props.title.replace(/\s+\|\s+FlowBook$/, ""));
	const fontSize = lines.length >= 3 ? 55 : 64;
	const lineHeight = lines.length >= 3 ? 66 : 76;
	const titleStart = lines.length >= 3 ? 225 : 248;
	const titleMarkup = lines
		.map(
			(line, index) =>
				`<text x="86" y="${titleStart + index * lineHeight}" class="title">${escapeXml(line)}</text>`,
		)
		.join("");
	const topicMarkup = props.topics
		.slice(0, 3)
		.map((topic, index) => {
			const x = 86 + index * 244;
			return `<g transform="translate(${x} 523)"><circle cx="5" cy="-4" r="5" fill="#ff6043"/><text x="20" y="0" class="topic">${escapeXml(truncate(topic, 25))}</text></g>`;
		})
		.join("");

	return `
		<svg width="${WIDTH}" height="${HEIGHT}" viewBox="0 0 ${WIDTH} ${HEIGHT}" xmlns="http://www.w3.org/2000/svg">
			<defs>
				<linearGradient id="accent" x1="0" y1="0" x2="1" y2="1">
					<stop stop-color="#ff9c54"/>
					<stop offset="0.48" stop-color="#ff573d"/>
					<stop offset="1" stop-color="#e92046"/>
				</linearGradient>
				<radialGradient id="glow" cx="0" cy="0" r="1" gradientTransform="translate(1020 136) rotate(132) scale(470 420)" gradientUnits="userSpaceOnUse">
					<stop stop-color="#e83243" stop-opacity="0.38"/>
					<stop offset="1" stop-color="#0c0e13" stop-opacity="0"/>
				</radialGradient>
				<pattern id="grid" width="64" height="64" patternUnits="userSpaceOnUse">
					<path d="M64 0H0V64" fill="none" stroke="#ffffff" stroke-opacity="0.045"/>
				</pattern>
				<style>
					.brand,.eyebrow,.topic,.domain{font-family:Inter,Arial,sans-serif}
					.brand{font-size:27px;font-weight:760;letter-spacing:-0.7px;fill:#f8f4f1}
					.eyebrow{font-size:15px;font-weight:720;letter-spacing:2.5px;fill:#ff8066}
					.title{font-family:Inter,Arial,sans-serif;font-size:${fontSize}px;font-weight:720;letter-spacing:-2.2px;fill:#f8f4f1}
					.topic{font-size:14px;font-weight:620;letter-spacing:.25px;fill:#c7c3c2}
					.domain{font-size:15px;font-weight:650;letter-spacing:1.3px;fill:#8f929b}
				</style>
			</defs>
			<rect width="1200" height="630" fill="#0c0e13"/>
			<rect width="1200" height="630" fill="url(#grid)"/>
			<rect width="1200" height="630" fill="url(#glow)"/>
			<circle cx="1080" cy="50" r="240" fill="none" stroke="url(#accent)" stroke-width="52" opacity=".13"/>
			<circle cx="1160" cy="-24" r="154" fill="none" stroke="url(#accent)" stroke-width="24" opacity=".2"/>
			<rect x="0" y="0" width="12" height="630" fill="url(#accent)"/>
			<g transform="translate(86 67)">
				<path d="M0 28c14-24 32-24 46-24h30c16 0 26 12 26 26S92 56 76 56H51c-16 0-29 8-38 23" fill="none" stroke="url(#accent)" stroke-width="11" stroke-linecap="round"/>
				<text x="126" y="49" class="brand">FlowBook</text>
			</g>
			<text x="86" y="153" class="eyebrow">${escapeXml(props.eyebrow)}</text>
			${titleMarkup}
			<line x1="86" y1="486" x2="1114" y2="486" stroke="#ffffff" stroke-opacity=".13"/>
			${topicMarkup}
			<text x="1114" y="527" text-anchor="end" class="domain">BOOK.FLOW-LIKE.COM</text>
			<text x="86" y="590" class="domain">${escapeXml(CURRENT_BOOK_EDITION.editionLabel.toUpperCase())}</text>
		</svg>
	`;
}

export const getStaticPaths: GetStaticPaths = async () => {
	const entries = await getCollection("docs");
	return entries.map((entry) => {
		const seo = resolveBookSeo(entry.id, entry.data);
		return {
			params: { slug: normalizeBookEntryId(entry.id) || "index" },
			props: {
				title: seo.title,
				eyebrow: cardEyebrow(entry.id),
				topics: seo.topics,
			} satisfies SocialCardProps,
		};
	});
};

export const GET: APIRoute = async ({ props }) => {
	const socialCard = props as SocialCardProps;
	const png = await sharp(Buffer.from(renderCardSvg(socialCard)))
		.png({ compressionLevel: 9, quality: 92 })
		.toBuffer();

	return new Response(new Uint8Array(png), {
		headers: {
			"Cache-Control": "public, max-age=86400, stale-while-revalidate=604800",
			"Content-Type": "image/png",
		},
	});
};
