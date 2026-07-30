import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import sharp from "sharp";

const sourceDirectory = fileURLToPath(
	new URL("../src/images/parallax/", import.meta.url),
);
const outputDirectory = fileURLToPath(
	new URL("../public/parallax/optimized/", import.meta.url),
);

const images = ["workflow-core", "ui-builder", "data-studio", "governance"];
const widths = [640, 960, 1150, 1440, 1725, 1920];

await mkdir(outputDirectory, { recursive: true });

await Promise.all(
	images.flatMap((image) =>
		widths.map(async (width) => {
			const source = `${sourceDirectory}${image}.png`;
			const output = `${outputDirectory}${image}-${width}.webp`;

			await sharp(source)
				.resize({
					width,
					withoutEnlargement: true,
					kernel: sharp.kernel.lanczos3,
				})
				.webp({
					lossless: true,
					effort: 6,
				})
				.toFile(output);
		}),
	),
);

console.log(
	`Generated ${images.length * widths.length} lossless WebP parallax assets.`,
);
