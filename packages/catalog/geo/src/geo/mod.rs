use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub mod geometry;
pub mod h3;
pub mod location;
pub mod location_event;
pub mod map;
pub(crate) mod pins;
pub mod routing;
pub mod search;

#[cfg(test)]
mod compatibility_tests;

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, Default)]
pub struct GeoCoordinate {
    pub latitude: f64,
    pub longitude: f64,
}

impl GeoCoordinate {
    pub fn new(latitude: f64, longitude: f64) -> Self {
        Self {
            latitude,
            longitude,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, Default)]
pub struct BoundingBox {
    pub min_lat: f64,
    pub min_lon: f64,
    pub max_lat: f64,
    pub max_lon: f64,
}

impl BoundingBox {
    pub fn new(min_lat: f64, min_lon: f64, max_lat: f64, max_lon: f64) -> Self {
        Self {
            min_lat,
            min_lon,
            max_lat,
            max_lon,
        }
    }

    pub fn contains(&self, coord: &GeoCoordinate) -> bool {
        coord.latitude >= self.min_lat
            && coord.latitude <= self.max_lat
            && coord.longitude >= self.min_lon
            && coord.longitude <= self.max_lon
    }

    pub fn center(&self) -> GeoCoordinate {
        GeoCoordinate::new(
            (self.min_lat + self.max_lat) / 2.0,
            (self.min_lon + self.max_lon) / 2.0,
        )
    }
}
