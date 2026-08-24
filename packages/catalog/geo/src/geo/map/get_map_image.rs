use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeImage;
use flow_like_types::{async_trait, json::json};

use crate::geo::GeoCoordinate;

#[crate::register_node]
#[derive(Default)]
pub struct GetMapImageNode {}

impl GetMapImageNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetMapImageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "geo_get_map_image",
            "Get Map Image",
            "Fetches a static map image for the given coordinates using OpenStreetMap tiles. Returns a satellite/standard map image centered on the location.",
            "Web/Geo/Map",
        );
        node.set_flowscript_name("geo", "getMapImage");
        node.add_icon("/flow/icons/map.svg");

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Initiate the map image request",
            VariableType::Execution,
        );
        node.add_input_pin(
            "coordinate",
            "Coordinate",
            "The geographic coordinate (latitude, longitude) to center the map on",
            VariableType::Struct,
        )
        .set_schema::<GeoCoordinate>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "zoom",
            "Zoom",
            "Map zoom level (1-19). Higher values show more detail. Default: 15",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(15)));

        node.add_input_pin(
            "width",
            "Width",
            "Image width in pixels. Default: 512",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(512)));

        node.add_input_pin(
            "height",
            "Height",
            "Image height in pixels. Default: 512",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(512)));

        node.add_input_pin("style", "Style", "Map style to use", VariableType::String)
            .set_default_value(Some(json!("Standard")))
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec![
                        "Standard".to_string(),
                        "Satellite".to_string(),
                        "Terrain".to_string(),
                        "Dark".to_string(),
                    ])
                    .build(),
            );

        node.add_output_pin(
            "exec_success",
            "Success",
            "Triggered when the map image is successfully fetched",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_error",
            "Error",
            "Triggered when the request fails",
            VariableType::Execution,
        );
        node.add_output_pin(
            "image",
            "Image",
            "The fetched map image",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>();

        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(9)
                .set_performance(7)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_types::{
            image::{DynamicImage, GenericImageView, ImageReader, RgbaImage},
            reqwest,
        };
        use std::io::Cursor;

        context.deactivate_exec_pin("exec_success").await?;
        context.activate_exec_pin("exec_error").await?;

        let coordinate: GeoCoordinate = context.evaluate_pin("coordinate").await?;
        let zoom: i64 = context.evaluate_pin("zoom").await?;
        let width: i64 = context.evaluate_pin("width").await?;
        let height: i64 = context.evaluate_pin("height").await?;
        let style: String = context.evaluate_pin("style").await?;

        let zoom = zoom.clamp(1, 19) as u32;
        let width = width.clamp(64, 2048) as u32;
        let height = height.clamp(64, 2048) as u32;

        let tile_base_url = match style.as_str() {
            "Standard" => "https://tile.openstreetmap.org",
            "Satellite" => {
                "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile"
            }
            "Terrain" => "https://tile.opentopomap.org",
            "Dark" => "https://tiles.stadiamaps.com/tiles/alidade_smooth_dark",
            _ => "https://tile.openstreetmap.org",
        };

        let (center_tile_x, center_tile_y, pixel_offset_x, pixel_offset_y) =
            lat_lon_to_tile(coordinate.latitude, coordinate.longitude, zoom);

        let tile_size = 256u32;
        let half_width = width as i32 / 2;
        let half_height = height as i32 / 2;

        let start_pixel_x = pixel_offset_x as i32 - half_width;
        let start_pixel_y = pixel_offset_y as i32 - half_height;

        let start_tile_x =
            center_tile_x as i32 + (start_pixel_x as f64 / tile_size as f64).floor() as i32;
        let start_tile_y =
            center_tile_y as i32 + (start_pixel_y as f64 / tile_size as f64).floor() as i32;
        let end_tile_x = center_tile_x as i32
            + ((start_pixel_x + width as i32) as f64 / tile_size as f64).ceil() as i32;
        let end_tile_y = center_tile_y as i32
            + ((start_pixel_y + height as i32) as f64 / tile_size as f64).ceil() as i32;

        let tiles_x = (end_tile_x - start_tile_x) as u32;
        let tiles_y = (end_tile_y - start_tile_y) as u32;

        let mut composite = RgbaImage::new(tiles_x * tile_size, tiles_y * tile_size);

        let client = reqwest::Client::builder()
            .user_agent("FlowLike/1.0")
            .build()?;

        let max_tile = 2u32.pow(zoom);

        for ty in 0..tiles_y {
            for tx in 0..tiles_x {
                let tile_x = (start_tile_x + tx as i32).rem_euclid(max_tile as i32) as u32;
                let tile_y = (start_tile_y + ty as i32).rem_euclid(max_tile as i32) as u32;

                let tile_url = if style == "Satellite" {
                    format!("{}/{}/{}/{}", tile_base_url, zoom, tile_y, tile_x)
                } else {
                    format!("{}/{}/{}/{}.png", tile_base_url, zoom, tile_x, tile_y)
                };

                let response = client.get(&tile_url).send().await?;
                if response.status().is_success() {
                    let bytes = response.bytes().await?;
                    if let Ok(tile_img) = ImageReader::new(Cursor::new(bytes))
                        .with_guessed_format()?
                        .decode()
                    {
                        for (px, py, pixel) in tile_img.pixels() {
                            let dest_x = tx * tile_size + px;
                            let dest_y = ty * tile_size + py;
                            if dest_x < composite.width() && dest_y < composite.height() {
                                composite.put_pixel(dest_x, dest_y, pixel);
                            }
                        }
                    }
                }
            }
        }

        let offset_in_composite_x =
            (pixel_offset_x as i32 - start_pixel_x.rem_euclid(tile_size as i32)) as u32;
        let offset_in_composite_y =
            (pixel_offset_y as i32 - start_pixel_y.rem_euclid(tile_size as i32)) as u32;

        let crop_x = offset_in_composite_x.saturating_sub(half_width as u32);
        let crop_y = offset_in_composite_y.saturating_sub(half_height as u32);

        let composite_dynamic = DynamicImage::ImageRgba8(composite);
        let cropped = composite_dynamic.crop_imm(crop_x, crop_y, width, height);

        let node_img = NodeImage::new(context, cropped).await;
        context.set_pin_value("image", json!(node_img)).await?;

        context.deactivate_exec_pin("exec_error").await?;
        context.activate_exec_pin("exec_success").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

fn lat_lon_to_tile(lat: f64, lon: f64, zoom: u32) -> (u32, u32, u32, u32) {
    let n = 2.0_f64.powi(zoom as i32);
    let x = (lon + 180.0) / 360.0 * n;
    let lat_rad = lat.to_radians();
    let y = (1.0 - lat_rad.tan().asinh() / std::f64::consts::PI) / 2.0 * n;

    let tile_x = x.floor() as u32;
    let tile_y = y.floor() as u32;
    let pixel_x = ((x - x.floor()) * 256.0) as u32;
    let pixel_y = ((y - y.floor()) * 256.0) as u32;

    (tile_x, tile_y, pixel_x, pixel_y)
}
