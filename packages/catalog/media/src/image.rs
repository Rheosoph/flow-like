use flow_like::flow::execution::context::ExecutionContext;
use flow_like_types::image::DynamicImage;
use flow_like_types::sync::Mutex;
use flow_like_types::{Cacheable, Result, create_id};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub mod annotate;
pub mod content;
pub mod generation;
pub mod metadata;
pub mod overlay;
pub mod pdf;
pub mod transform;

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct NodeImage {
    pub image_ref: String,
}

#[derive(Default, Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct BoundingBox {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub score: f32,
    pub class_idx: i32,
    pub class_name: Option<String>,
}

impl BoundingBox {
    pub fn xywh(&self) -> (u32, u32, u32, u32) {
        let w = self.x2 - self.x1;
        let h = self.y2 - self.y1;
        let x = (self.x2 + self.x1) / 2.0;
        let y = (self.y2 + self.y1) / 2.0;
        (x as u32, y as u32, w as u32, h as u32)
    }

    pub fn x1y1wh(&self) -> (u32, u32, u32, u32) {
        let w = self.x2 - self.x1;
        let h = self.y2 - self.y1;
        (self.x1 as u32, self.y1 as u32, w as u32, h as u32)
    }

    pub fn area(&self) -> f32 {
        let w = self.x2 - self.x1;
        let h = self.y2 - self.y1;
        if w > 0.0 && h > 0.0 { w * h } else { 0.0 }
    }

    pub fn iou(&self, other: &BoundingBox) -> f32 {
        let x1_inter = self.x1.max(other.x1);
        let y1_inter = self.y1.max(other.y1);
        let x2_inter = self.x2.min(other.x2);
        let y2_inter = self.y2.min(other.y2);

        let w_inter = x2_inter - x1_inter;
        let h_inter = y2_inter - y1_inter;

        let intersection = if w_inter > 0.0 && h_inter > 0.0 {
            w_inter * h_inter
        } else {
            0.0
        };

        let union = self.area() + other.area() - intersection;
        if union > 0.0 {
            intersection / union
        } else {
            0.0
        }
    }

    pub fn scale(&mut self, scale_w: f32, scale_h: f32) {
        self.x1 *= scale_w;
        self.y1 *= scale_h;
        self.x2 *= scale_w;
        self.y2 *= scale_h;
    }
}

pub struct NodeImageWrapper {
    pub image: Arc<Mutex<DynamicImage>>,
}

impl Cacheable for NodeImageWrapper {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl NodeImage {
    pub async fn new(ctx: &mut ExecutionContext, image: DynamicImage) -> Self {
        let id = create_id();
        let image_ref = Arc::new(Mutex::new(image));
        let wrapper = NodeImageWrapper {
            image: image_ref.clone(),
        };
        ctx.cache
            .write()
            .await
            .insert(id.clone(), Arc::new(wrapper));
        NodeImage { image_ref: id }
    }

    pub async fn copy_image(&self, ctx: &mut ExecutionContext) -> Result<Self> {
        let image = ctx
            .cache
            .read()
            .await
            .get(&self.image_ref)
            .cloned()
            .ok_or_else(|| flow_like_types::anyhow!("Image not found in cache"))?;
        let image_wrapper = image
            .as_any()
            .downcast_ref::<NodeImageWrapper>()
            .ok_or_else(|| flow_like_types::anyhow!("Could not downcast to NodeImageWrapper"))?;
        let image = image_wrapper.image.lock().await.clone();
        let new_id = create_id();
        let new_image_ref = Arc::new(Mutex::new(image.clone()));
        let new_wrapper = NodeImageWrapper {
            image: new_image_ref.clone(),
        };
        ctx.cache
            .write()
            .await
            .insert(new_id.clone(), Arc::new(new_wrapper));
        let new_image = NodeImage { image_ref: new_id };
        Ok(new_image)
    }

    pub async fn get_image(&self, ctx: &mut ExecutionContext) -> Result<Arc<Mutex<DynamicImage>>> {
        let image = ctx
            .cache
            .read()
            .await
            .get(&self.image_ref)
            .cloned()
            .ok_or_else(|| flow_like_types::anyhow!("Image not found in cache"))?;
        let image_wrapper = image
            .as_any()
            .downcast_ref::<NodeImageWrapper>()
            .ok_or_else(|| flow_like_types::anyhow!("Could not downcast to NodeImageWrapper"))?;
        let image = image_wrapper.image.clone();
        Ok(image)
    }
}
