use crate::image::NodeImage;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{
    async_trait,
    image::{DynamicImage, GenericImageView, imageops::FilterType},
    json::json,
};

#[crate::register_node]
#[derive(Default)]
pub struct ImageOverlayNode {}

impl ImageOverlayNode {
    pub fn new() -> Self {
        ImageOverlayNode {}
    }
}

fn apply_opacity(overlay: &DynamicImage, opacity: f32) -> DynamicImage {
    let mut rgba = overlay.to_rgba8();
    let clamped = opacity.clamp(0.0, 1.0);
    for pixel in rgba.pixels_mut() {
        pixel.0[3] = (pixel.0[3] as f32 * clamped) as u8;
    }
    DynamicImage::ImageRgba8(rgba)
}

fn fit_overlay(overlay: &DynamicImage, max_w: u32, max_h: u32, mode: &str) -> DynamicImage {
    match mode {
        "fill" => overlay.resize_exact(max_w, max_h, FilterType::Lanczos3),
        "cover" => overlay.resize_to_fill(max_w, max_h, FilterType::Lanczos3),
        "contain" => overlay.resize(max_w, max_h, FilterType::Lanczos3),
        _ => overlay.resize(max_w, max_h, FilterType::Lanczos3),
    }
}

#[async_trait]
impl NodeLogic for ImageOverlayNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "image_overlay",
            "Image Overlay",
            "Overlay one image on top of another with configurable position, size, opacity and fit mode",
            "Image/Overlay",
        );
        node.add_icon("/flow/icons/image.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "base_image",
            "Base Image",
            "The background image",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "overlay_image",
            "Overlay Image",
            "The image to overlay on top",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "use_ref",
            "Use Reference",
            "Use reference of the base image, transforming the original instead of a copy",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "x",
            "X",
            "Horizontal offset in pixels from the left edge",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "y",
            "Y",
            "Vertical offset in pixels from the top edge",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "max_w",
            "Max Width",
            "Maximum width of the overlay (0 = original width)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "max_h",
            "Max Height",
            "Maximum height of the overlay (0 = original height)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "opacity",
            "Opacity",
            "Overlay opacity from 0.0 (transparent) to 1.0 (opaque)",
            VariableType::Float,
        )
        .set_default_value(Some(json!(1.0)));

        node.add_input_pin(
            "fit_mode",
            "Fit Mode",
            "How to fit the overlay into max width/height",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "contain".to_string(),
                    "cover".to_string(),
                    "fill".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("contain")));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "image_out",
            "Image",
            "Result image with overlay applied",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut base_node_img: NodeImage = context.evaluate_pin("base_image").await?;
        let overlay_node_img: NodeImage = context.evaluate_pin("overlay_image").await?;
        let use_ref: bool = context.evaluate_pin("use_ref").await?;
        let x: i64 = context.evaluate_pin("x").await?;
        let y: i64 = context.evaluate_pin("y").await?;
        let max_w: u32 = context.evaluate_pin("max_w").await?;
        let max_h: u32 = context.evaluate_pin("max_h").await?;
        let opacity: f32 = context.evaluate_pin("opacity").await?;
        let fit_mode: String = context.evaluate_pin("fit_mode").await?;

        if !use_ref {
            base_node_img = base_node_img.copy_image(context).await?;
        }

        let base_arc = base_node_img.get_image(context).await?;
        let overlay_arc = overlay_node_img.get_image(context).await?;

        {
            let mut base_guard = base_arc.lock().await;
            let overlay_guard = overlay_arc.lock().await;

            let mut overlay_img = overlay_guard.clone();

            let (ow, oh) = overlay_img.dimensions();
            let target_w = if max_w > 0 { max_w } else { ow };
            let target_h = if max_h > 0 { max_h } else { oh };

            if target_w != ow || target_h != oh {
                overlay_img = fit_overlay(&overlay_img, target_w, target_h, &fit_mode);
            }

            if opacity < 1.0 {
                overlay_img = apply_opacity(&overlay_img, opacity);
            }

            let mut base_rgba = base_guard.to_rgba8();
            let overlay_rgba = overlay_img.to_rgba8();
            flow_like_types::image::imageops::overlay(&mut base_rgba, &overlay_rgba, x, y);
            *base_guard = DynamicImage::ImageRgba8(base_rgba);
        }

        context
            .set_pin_value("image_out", json!(base_node_img))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::image::{DynamicImage, GenericImageView, Rgba, RgbaImage};

    fn solid_rgba(w: u32, h: u32, r: u8, g: u8, b: u8, a: u8) -> DynamicImage {
        let buf = RgbaImage::from_pixel(w, h, Rgba([r, g, b, a]));
        DynamicImage::ImageRgba8(buf)
    }

    #[test]
    fn apply_opacity_full() {
        let img = solid_rgba(2, 2, 255, 0, 0, 200);
        let result = apply_opacity(&img, 1.0);
        let px = result.to_rgba8().get_pixel(0, 0).0;
        assert_eq!(px[3], 200);
    }

    #[test]
    fn apply_opacity_half() {
        let img = solid_rgba(2, 2, 255, 0, 0, 200);
        let result = apply_opacity(&img, 0.5);
        let px = result.to_rgba8().get_pixel(0, 0).0;
        assert_eq!(px[3], 100);
    }

    #[test]
    fn apply_opacity_zero() {
        let img = solid_rgba(2, 2, 255, 0, 0, 255);
        let result = apply_opacity(&img, 0.0);
        let px = result.to_rgba8().get_pixel(0, 0).0;
        assert_eq!(px[3], 0);
    }

    #[test]
    fn apply_opacity_clamps_above_one() {
        let img = solid_rgba(2, 2, 255, 0, 0, 200);
        let result = apply_opacity(&img, 2.0);
        let px = result.to_rgba8().get_pixel(0, 0).0;
        assert_eq!(px[3], 200);
    }

    #[test]
    fn fit_overlay_contain_preserves_aspect() {
        let img = solid_rgba(200, 100, 0, 0, 0, 255);
        let result = fit_overlay(&img, 100, 100, "contain");
        let (w, h) = result.dimensions();
        assert_eq!(w, 100);
        assert_eq!(h, 50);
    }

    #[test]
    fn fit_overlay_fill_exact_dimensions() {
        let img = solid_rgba(200, 100, 0, 0, 0, 255);
        let result = fit_overlay(&img, 50, 80, "fill");
        let (w, h) = result.dimensions();
        assert_eq!(w, 50);
        assert_eq!(h, 80);
    }

    #[test]
    fn fit_overlay_cover_fills_bounds() {
        let img = solid_rgba(200, 100, 0, 0, 0, 255);
        let result = fit_overlay(&img, 50, 50, "cover");
        let (w, h) = result.dimensions();
        assert_eq!(w, 50);
        assert_eq!(h, 50);
    }

    #[test]
    fn overlay_composites_onto_base() {
        let base = solid_rgba(100, 100, 0, 0, 0, 255);
        let overlay = solid_rgba(10, 10, 255, 0, 0, 255);

        let mut base_rgba = base.to_rgba8();
        let overlay_rgba = overlay.to_rgba8();
        flow_like_types::image::imageops::overlay(&mut base_rgba, &overlay_rgba, 5, 5);

        let px_overlay = base_rgba.get_pixel(5, 5).0;
        assert_eq!(px_overlay[0], 255);
        assert_eq!(px_overlay[1], 0);

        let px_base = base_rgba.get_pixel(0, 0).0;
        assert_eq!(px_base[0], 0);
    }

    #[test]
    fn overlay_with_half_opacity_blends() {
        let base = solid_rgba(10, 10, 0, 0, 0, 255);
        let overlay = solid_rgba(10, 10, 255, 0, 0, 255);
        let overlay = apply_opacity(&overlay, 0.5);

        let mut base_rgba = base.to_rgba8();
        let overlay_rgba = overlay.to_rgba8();
        flow_like_types::image::imageops::overlay(&mut base_rgba, &overlay_rgba, 0, 0);

        let px = base_rgba.get_pixel(0, 0).0;
        // Red channel should be partially blended (roughly half of 255)
        assert!(
            px[0] > 100 && px[0] < 200,
            "expected blended red, got {}",
            px[0]
        );
    }
}
