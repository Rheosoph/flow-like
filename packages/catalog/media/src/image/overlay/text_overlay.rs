use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeImage;
use flow_like_types::{
    ab_glyph::FontArc,
    async_trait,
    image::{DynamicImage, Rgba},
    imageproc::drawing::draw_text_mut,
    json::json,
};

#[crate::register_node]
#[derive(Default)]
pub struct TextOverlayNode {}

impl TextOverlayNode {
    pub fn new() -> Self {
        TextOverlayNode {}
    }
}

fn parse_hex_color(hex: &str) -> Rgba<u8> {
    let hex = hex.trim_start_matches('#');
    let (r, g, b, a) = match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(255);
            let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(255);
            let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(255);
            (r, g, b, 255u8)
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(255);
            let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(255);
            let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(255);
            let a = u8::from_str_radix(&hex[6..8], 16).unwrap_or(255);
            (r, g, b, a)
        }
        _ => (255, 255, 255, 255),
    };
    Rgba([r, g, b, a])
}

#[async_trait]
impl NodeLogic for TextOverlayNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "text_overlay",
            "Text Overlay",
            "Draw text on top of an image with configurable font size, position, and color",
            "Image/Overlay",
        );
        node.set_version(1);
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
            "The background image to draw text on",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "text",
            "Text",
            "The text string to render",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

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
            "font_size",
            "Font Size",
            "Font size in pixels",
            VariableType::Float,
        )
        .set_default_value(Some(json!(24.0)));

        node.add_input_pin(
            "color",
            "Color",
            "Text color as hex string (e.g. #FF0000 or #FF0000AA for alpha)",
            VariableType::String,
        )
        .set_default_value(Some(json!("#FFFFFF")));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "image_out",
            "Image",
            "Result image with text rendered",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut base_node_img: NodeImage = context.evaluate_pin("base_image").await?;
        let text: String = context.evaluate_pin("text").await?;
        let use_ref: bool = context.evaluate_pin("use_ref").await?;
        let x: i32 = context.evaluate_pin("x").await?;
        let y: i32 = context.evaluate_pin("y").await?;
        let font_size: f32 = context.evaluate_pin("font_size").await?;
        let color_hex: String = context.evaluate_pin("color").await?;

        if !use_ref {
            base_node_img = base_node_img.copy_image(context).await?;
        }

        let color = parse_hex_color(&color_hex);

        let font_data = include_bytes!("../annotate/assets/DejaVuSans.ttf");
        let font = FontArc::try_from_slice(font_data as &[u8])
            .map_err(|e| flow_like_types::anyhow!("Failed to load font: {}", e))?;

        let base_arc = base_node_img.get_image(context).await?;
        {
            let mut base_guard = base_arc.lock().await;
            let mut rgba = base_guard.to_rgba8();
            draw_text_mut(&mut rgba, color, x, y, font_size, &font, &text);
            *base_guard = DynamicImage::ImageRgba8(rgba);
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
    use flow_like_types::image::RgbaImage;

    #[test]
    fn parse_hex_color_rgb() {
        let c = parse_hex_color("#FF8000");
        assert_eq!(c.0, [255, 128, 0, 255]);
    }

    #[test]
    fn parse_hex_color_rgba() {
        let c = parse_hex_color("#FF800080");
        assert_eq!(c.0, [255, 128, 0, 128]);
    }

    #[test]
    fn parse_hex_color_without_hash() {
        let c = parse_hex_color("00FF00");
        assert_eq!(c.0, [0, 255, 0, 255]);
    }

    #[test]
    fn parse_hex_color_invalid_falls_back_to_white() {
        let c = parse_hex_color("xyz");
        assert_eq!(c.0, [255, 255, 255, 255]);
    }

    #[test]
    fn font_loads_successfully() {
        let font_data = include_bytes!("../annotate/assets/DejaVuSans.ttf");
        let font = FontArc::try_from_slice(font_data as &[u8]);
        assert!(font.is_ok(), "Font should load without error");
    }

    #[test]
    fn draw_text_modifies_pixels() {
        let mut rgba = RgbaImage::from_pixel(200, 50, Rgba([0, 0, 0, 255]));

        let font_data = include_bytes!("../annotate/assets/DejaVuSans.ttf");
        let font = FontArc::try_from_slice(font_data as &[u8]).unwrap();

        draw_text_mut(
            &mut rgba,
            Rgba([255, 255, 255, 255]),
            10,
            5,
            24.0,
            &font,
            "Hello",
        );

        // At least some pixels should have changed from pure black
        let has_non_black = rgba
            .pixels()
            .any(|p| p.0[0] > 0 || p.0[1] > 0 || p.0[2] > 0);
        assert!(
            has_non_black,
            "Text rendering should have produced non-black pixels"
        );
    }

    #[test]
    fn draw_text_respects_position() {
        let mut rgba = RgbaImage::from_pixel(200, 100, Rgba([0, 0, 0, 255]));

        let font_data = include_bytes!("../annotate/assets/DejaVuSans.ttf");
        let font = FontArc::try_from_slice(font_data as &[u8]).unwrap();

        // Draw text at far bottom-right corner
        draw_text_mut(
            &mut rgba,
            Rgba([255, 255, 255, 255]),
            100,
            70,
            20.0,
            &font,
            "Hi",
        );

        // Top-left quadrant should remain untouched (pure black)
        let top_left_clean = (0..50).all(|y| {
            (0..100).all(|x| {
                let p = rgba.get_pixel(x, y).0;
                p[0] == 0 && p[1] == 0 && p[2] == 0
            })
        });
        assert!(top_left_clean, "Top-left quadrant should be untouched");
    }

    #[test]
    fn draw_text_with_color() {
        let mut rgba = RgbaImage::from_pixel(200, 50, Rgba([0, 0, 0, 255]));

        let font_data = include_bytes!("../annotate/assets/DejaVuSans.ttf");
        let font = FontArc::try_from_slice(font_data as &[u8]).unwrap();

        let red = parse_hex_color("#FF0000");
        draw_text_mut(&mut rgba, red, 10, 5, 24.0, &font, "Red");

        // Find a text pixel — it should have red > 0 and green/blue == 0
        let has_red_pixel = rgba
            .pixels()
            .any(|p| p.0[0] > 0 && p.0[1] == 0 && p.0[2] == 0);
        assert!(has_red_pixel, "Should have pure red text pixels");
    }
}
