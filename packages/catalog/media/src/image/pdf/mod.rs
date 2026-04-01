#[cfg(feature = "execute")]
use flow_like::flow::execution::context::ExecutionContext;
#[cfg(feature = "execute")]
use flow_like_catalog_core::FlowPath;
#[cfg(feature = "execute")]
use flow_like_types::image::{DynamicImage, ImageBuffer, Rgba};
#[cfg(feature = "execute")]
use hayro::hayro_syntax::Pdf;
#[cfg(feature = "execute")]
use hayro::vello_cpu::Pixmap;
#[cfg(feature = "execute")]
use hayro::vello_cpu::color::AlphaColor;
#[cfg(feature = "execute")]
use hayro::vello_cpu::color::Srgb;
#[cfg(feature = "execute")]
use hayro::vello_cpu::color::palette::css::TRANSPARENT;
#[cfg(feature = "execute")]
use std::sync::Arc;

pub mod page_count;
pub mod page_to_image;
pub mod pdf_to_images;

#[cfg(feature = "execute")]
pub(super) async fn load_pdf_from_flowpath(
    context: &mut ExecutionContext,
    flow_path: &FlowPath,
) -> flow_like_types::Result<Pdf> {
    let bytes = flow_path.get(context, false).await?;
    let data: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(bytes);

    Pdf::new(data).map_err(|err| flow_like_types::anyhow!("Failed to load PDF: {:?}", err))
}

#[cfg(feature = "execute")]
pub(super) fn pixmap_to_dynamic_image(pixmap: Pixmap) -> flow_like_types::Result<DynamicImage> {
    let width = pixmap.width() as u32;
    let height = pixmap.height() as u32;
    let data: Vec<u8> = pixmap
        .take_unpremultiplied()
        .into_iter()
        .flat_map(|p| [p.r, p.g, p.b, p.a])
        .collect();

    let buffer: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_vec(width, height, data)
        .ok_or_else(|| {
            flow_like_types::anyhow!("Failed to build image buffer from rendered PDF page")
        })?;

    Ok(DynamicImage::ImageRgba8(buffer))
}

#[cfg(feature = "execute")]
pub(super) fn validate_scale(scale: f32) -> flow_like_types::Result<()> {
    if scale <= 0.0 {
        return Err(flow_like_types::anyhow!("Scale must be greater than 0"));
    }

    Ok(())
}

#[cfg(feature = "execute")]
pub(super) fn resolve_page_index(
    page_number: i64,
    total_pages: usize,
) -> flow_like_types::Result<usize> {
    if page_number < 1 {
        return Err(flow_like_types::anyhow!("Page number must be at least 1"));
    }

    let index = (page_number - 1) as usize;

    if index >= total_pages {
        return Err(flow_like_types::anyhow!(
            "Requested page {} but document has {} pages",
            page_number,
            total_pages
        ));
    }

    Ok(index)
}

#[cfg(feature = "execute")]
pub(super) fn resolve_bg_color(name: &str) -> AlphaColor<Srgb> {
    match name {
        "Black" => AlphaColor::new([0.0, 0.0, 0.0, 1.0]),
        "White" => AlphaColor::new([1.0, 1.0, 1.0, 1.0]),
        "Red" => AlphaColor::new([1.0, 0.0, 0.0, 1.0]),
        "Green" => AlphaColor::new([0.0, 0.5, 0.0, 1.0]),
        _ => TRANSPARENT,
    }
}
