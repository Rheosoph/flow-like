#[cfg(feature = "execute")]
use flow_like_types::image::{DynamicImage, ImageBuffer, Rgba};
#[cfg(feature = "execute")]
use hayro::hayro_interpret::InterpreterSettings;
#[cfg(feature = "execute")]
use hayro::hayro_syntax::Pdf;
#[cfg(feature = "execute")]
use hayro::hayro_syntax::page::Page;
#[cfg(feature = "execute")]
use hayro::vello_cpu::Pixmap;
#[cfg(feature = "execute")]
use hayro::vello_cpu::color::AlphaColor;
#[cfg(feature = "execute")]
use hayro::vello_cpu::color::Srgb;
#[cfg(feature = "execute")]
use hayro::vello_cpu::color::palette::css::TRANSPARENT;
#[cfg(feature = "execute")]
use hayro::vello_cpu::peniko::ImageAlphaType;
#[cfg(feature = "execute")]
use hayro::{PixmapSettings, RenderCache, RenderSettings, render};

pub mod page_count;
pub mod page_to_image;
pub mod pdf_to_images;

/// What shows through the transparent parts of a rendered page.
#[cfg(feature = "execute")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PageBackground {
    #[default]
    Transparent,
    White,
    Black,
    Red,
    Green,
}

#[cfg(feature = "execute")]
impl PageBackground {
    /// The background named by a node's Background Color pin; any other name is transparent.
    pub fn from_name(name: &str) -> Self {
        match name {
            "White" => Self::White,
            "Black" => Self::Black,
            "Red" => Self::Red,
            "Green" => Self::Green,
            _ => Self::Transparent,
        }
    }

    fn color(self) -> AlphaColor<Srgb> {
        match self {
            Self::Transparent => TRANSPARENT,
            Self::White => AlphaColor::new([1.0, 1.0, 1.0, 1.0]),
            Self::Black => AlphaColor::new([0.0, 0.0, 0.0, 1.0]),
            Self::Red => AlphaColor::new([1.0, 0.0, 0.0, 1.0]),
            Self::Green => AlphaColor::new([0.0, 0.5, 0.0, 1.0]),
        }
    }
}

#[cfg(feature = "execute")]
#[derive(Clone, Copy, Debug)]
pub struct RenderOptions {
    /// Pixels per PDF point.
    pub scale: f32,
    pub background: PageBackground,
    /// Largest width × height of one rendered page; a larger page renders at the scale that
    /// fits. `None` renders every page at `scale`.
    pub max_pixels: Option<u64>,
}

#[cfg(feature = "execute")]
pub struct RenderedPage {
    /// 1-based page number.
    pub page: usize,
    pub image: DynamicImage,
}

#[cfg(feature = "execute")]
fn load_pdf(bytes: Vec<u8>) -> flow_like_types::Result<Pdf> {
    Pdf::new(bytes).map_err(|err| flow_like_types::anyhow!("Failed to load PDF: {:?}", err))
}

#[cfg(feature = "execute")]
pub fn page_count(bytes: Vec<u8>) -> flow_like_types::Result<usize> {
    Ok(load_pdf(bytes)?.pages().len())
}

/// Renders the 1-based `pages` in the order given, or every page when `None`. Synchronous:
/// hayro's render cache cannot cross an `.await`, so async callers render on a blocking
/// thread.
#[cfg(feature = "execute")]
pub fn render_pages(
    bytes: Vec<u8>,
    pages: Option<&[usize]>,
    options: &RenderOptions,
) -> flow_like_types::Result<Vec<RenderedPage>> {
    validate_scale(options.scale)?;
    if options.max_pixels == Some(0) {
        return Err(flow_like_types::anyhow!(
            "Max pixels must be greater than 0"
        ));
    }
    let pdf = load_pdf(bytes)?;
    let document = pdf.pages();
    let selected = match pages {
        Some(pages) => pages
            .iter()
            .map(|&page| resolve_page_index(page, document.len()))
            .collect::<flow_like_types::Result<Vec<_>>>()?,
        None => (0..document.len()).collect(),
    };

    let cache = RenderCache::new();
    selected
        .into_iter()
        .map(|index| {
            let page = &document[index];
            let image = render_page(
                page,
                &cache,
                fitted_scale(page, options),
                options.background,
            )?;
            Ok(RenderedPage {
                page: index + 1,
                image,
            })
        })
        .collect()
}

#[cfg(feature = "execute")]
fn render_page<'a>(
    page: &'a Page<'a>,
    cache: &RenderCache<'a>,
    scale: f32,
    background: PageBackground,
) -> flow_like_types::Result<DynamicImage> {
    let pixmap_settings = PixmapSettings {
        x_scale: scale,
        y_scale: scale,
        bg_color: background.color(),
    };
    let pixmap = render(
        page,
        cache,
        &InterpreterSettings::default(),
        &RenderSettings::default(),
        &pixmap_settings,
    );

    pixmap_to_dynamic_image(pixmap)
}

/// Pixels hayro allocates for a page at `scale`; it sizes the pixmap with saturating `u16`
/// casts.
#[cfg(feature = "execute")]
fn rendered_pixels(width: f32, height: f32, scale: f32) -> u64 {
    u64::from((width * scale) as u16) * u64::from((height * scale) as u16)
}

#[cfg(feature = "execute")]
fn fitted_scale(page: &Page<'_>, options: &RenderOptions) -> f32 {
    let (width, height) = page.render_dimensions();
    match options.max_pixels {
        Some(max) if rendered_pixels(width, height, options.scale) > max => {
            let mut scale = (max as f64 / (f64::from(width) * f64::from(height))).sqrt() as f32;
            while scale > 0.0 && rendered_pixels(width, height, scale) > max {
                scale = scale.next_down();
            }
            scale
        }
        _ => options.scale,
    }
}

#[cfg(feature = "execute")]
fn pixmap_to_dynamic_image(pixmap: Pixmap) -> flow_like_types::Result<DynamicImage> {
    let width = pixmap.width() as u32;
    let height = pixmap.height() as u32;
    let data = pixmap.take_rgba8(ImageAlphaType::Alpha);

    let buffer: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_vec(width, height, data)
        .ok_or_else(|| {
            flow_like_types::anyhow!("Failed to build image buffer from rendered PDF page")
        })?;

    Ok(DynamicImage::ImageRgba8(buffer))
}

#[cfg(feature = "execute")]
fn validate_scale(scale: f32) -> flow_like_types::Result<()> {
    if scale <= 0.0 {
        return Err(flow_like_types::anyhow!("Scale must be greater than 0"));
    }

    Ok(())
}

#[cfg(feature = "execute")]
fn resolve_page_index(page_number: usize, total_pages: usize) -> flow_like_types::Result<usize> {
    if page_number < 1 {
        return Err(flow_like_types::anyhow!("Page number must be at least 1"));
    }

    let index = page_number - 1;

    if index >= total_pages {
        return Err(flow_like_types::anyhow!(
            "Requested page {} but document has {} pages",
            page_number,
            total_pages
        ));
    }

    Ok(index)
}

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;
    use flow_like_types::image::GenericImageView;

    const DIGITAL: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../apps/desktop/lib/university/courses/specialist/document-processing/fixtures/digital-agreement.pdf"
    ));
    const MIXED: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../apps/desktop/lib/university/courses/specialist/document-processing/fixtures/mixed-agreement.pdf"
    ));
    const SCANNED: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../apps/desktop/lib/university/courses/specialist/document-processing/fixtures/scanned-agreement.pdf"
    ));

    /// Every fixture is A4 portrait.
    const A4: (f32, f32) = (595.0, 842.0);

    fn options(scale: f32) -> RenderOptions {
        RenderOptions {
            scale,
            background: PageBackground::White,
            max_pixels: None,
        }
    }

    fn error(result: flow_like_types::Result<Vec<RenderedPage>>) -> String {
        match result {
            Ok(_) => panic!("expected the render to fail"),
            Err(error) => error.to_string(),
        }
    }

    #[test]
    fn counts_the_pages_of_every_fixture() {
        assert_eq!(page_count(DIGITAL.to_vec()).unwrap(), 1);
        assert_eq!(page_count(SCANNED.to_vec()).unwrap(), 1);
        assert_eq!(page_count(MIXED.to_vec()).unwrap(), 2);
    }

    #[test]
    fn rejects_bytes_that_are_not_a_pdf() {
        let refused = page_count(b"definitely not a pdf".to_vec()).unwrap_err();
        assert!(
            refused.to_string().starts_with("Failed to load PDF"),
            "{refused}"
        );
        assert!(
            error(render_pages(b"%PDF-".to_vec(), None, &options(1.0)))
                .starts_with("Failed to load PDF")
        );
    }

    #[test]
    fn renders_every_page_at_page_size_times_scale() {
        for (bytes, pages) in [(DIGITAL, 1), (SCANNED, 1), (MIXED, 2)] {
            let rendered = render_pages(bytes.to_vec(), None, &options(1.5)).unwrap();
            assert_eq!(rendered.len(), pages);
            for (index, page) in rendered.iter().enumerate() {
                assert_eq!(page.page, index + 1);
                assert_eq!(
                    page.image.dimensions(),
                    ((A4.0 * 1.5) as u32, (A4.1 * 1.5) as u32)
                );
            }
        }
    }

    #[test]
    fn renders_the_selected_pages_in_the_order_given() {
        let rendered = render_pages(MIXED.to_vec(), Some(&[2, 1, 2]), &options(0.5)).unwrap();
        let order: Vec<_> = rendered.iter().map(|page| page.page).collect();
        assert_eq!(order, vec![2, 1, 2]);
        assert_eq!(rendered[0].image.dimensions(), (297, 421));
    }

    #[test]
    fn refuses_pages_outside_the_document_and_scales_at_or_below_zero() {
        assert_eq!(
            error(render_pages(MIXED.to_vec(), Some(&[3]), &options(1.0))),
            "Requested page 3 but document has 2 pages"
        );
        assert_eq!(
            error(render_pages(MIXED.to_vec(), Some(&[0]), &options(1.0))),
            "Page number must be at least 1"
        );
        for scale in [0.0, -1.0] {
            assert_eq!(
                error(render_pages(DIGITAL.to_vec(), None, &options(scale))),
                "Scale must be greater than 0"
            );
        }
        assert_eq!(
            error(render_pages(
                DIGITAL.to_vec(),
                None,
                &RenderOptions {
                    max_pixels: Some(0),
                    ..options(1.0)
                }
            )),
            "Max pixels must be greater than 0"
        );
    }

    #[test]
    fn max_pixels_shrinks_only_pages_beyond_it() {
        let max = 200_000;
        let capped = RenderOptions {
            max_pixels: Some(max),
            ..options(4.0)
        };
        let [page] =
            <[RenderedPage; 1]>::try_from(render_pages(DIGITAL.to_vec(), None, &capped).unwrap())
                .unwrap_or_else(|_| panic!("one page"));
        let (width, height) = page.image.dimensions();
        assert!(u64::from(width) * u64::from(height) <= max);
        assert!(u64::from(width) * u64::from(height) > max * 9 / 10);
        let aspect = f64::from(width) / f64::from(height);
        assert!((aspect - f64::from(A4.0 / A4.1)).abs() < 0.01, "{aspect}");

        let roomy = RenderOptions {
            max_pixels: Some(u64::MAX),
            ..options(1.0)
        };
        let unchanged = render_pages(DIGITAL.to_vec(), None, &roomy).unwrap();
        assert_eq!(unchanged[0].image.dimensions(), (595, 842));
    }

    #[test]
    fn backgrounds_fill_the_page_margin() {
        let corner = |background| {
            let rendered = render_pages(
                DIGITAL.to_vec(),
                None,
                &RenderOptions {
                    background,
                    ..options(0.25)
                },
            )
            .unwrap();
            rendered[0].image.get_pixel(0, 0).0
        };
        assert_eq!(corner(PageBackground::White), [255, 255, 255, 255]);
        assert_eq!(corner(PageBackground::Transparent)[3], 0);
        assert_eq!(PageBackground::from_name("White"), PageBackground::White);
        assert_eq!(PageBackground::from_name("Green"), PageBackground::Green);
        assert_eq!(
            PageBackground::from_name("white"),
            PageBackground::Transparent
        );
        assert_eq!(
            PageBackground::from_name("Transparent"),
            PageBackground::Transparent
        );
    }
}
