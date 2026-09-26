import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import sharp from "sharp";

const sourceDirectory = fileURLToPath(
	new URL("../src/images/parallax/", import.meta.url),
);
// Published versions have immutable cache headers. Bump this value, the matching
// value in platform-mock.astro and the rule in public/_headers when content changes.
const assetVersion = "v2";
const outputDirectory = fileURLToPath(
	new URL(`../public/parallax/optimized/${assetVersion}/`, import.meta.url),
);

const images = ["workflow-core", "ui-builder", "data-studio", "governance"];
const widths = [640, 960, 1150, 1440, 1725, 1920];
const maxAssetBytes = 200 * 1024;

await mkdir(outputDirectory, { recursive: true });

const generated = await Promise.all(
	images.flatMap((image) =>
		widths.map(async (width) => {
			const source = `${sourceDirectory}${image}.png`;
			const output = `${outputDirectory}${image}-${width}.webp`;

			const info = await sharp(source)
				.resize({
					width,
					withoutEnlargement: true,
					kernel: sharp.kernel.lanczos3,
				})
				.webp({
					quality: 88,
					alphaQuality: 100,
					smartSubsample: true,
					effort: 6,
				})
				.toFile(output);

			return { output, size: info.size };
		}),
	),
);

const oversized = generated.filter((asset) => asset.size > maxAssetBytes);
if (oversized.length > 0) {
	throw new Error(
		`Parallax assets exceed the ${maxAssetBytes / 1024} KiB budget:\n${oversized
			.map((asset) => `${asset.output}: ${(asset.size / 1024).toFixed(1)} KiB`)
			.join("\n")}`,
	);
}

const totalBytes = generated.reduce((total, asset) => total + asset.size, 0);
const largestBytes = Math.max(...generated.map((asset) => asset.size));
console.log(
	`Generated ${generated.length} responsive WebP parallax assets ` +
		`(${(totalBytes / 1024 / 1024).toFixed(2)} MiB total, ` +
		`${(largestBytes / 1024).toFixed(1)} KiB largest).`,
);
