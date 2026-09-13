use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub const RETENTION_MS: i64 = 60 * 60 * 1000;
pub const MAX_PENDING: usize = 128;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GeoLocationSink {
    pub latitude: f64,
    pub longitude: f64,
    pub radius: f64,
    #[serde(default)]
    pub trigger_on: GeoTriggerType,
    #[serde(default)]
    pub background: bool,
    #[serde(default)]
    pub is_inside: Option<bool>,
    #[serde(default)]
    pub last_trigger_time: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GeoTriggerType {
    #[default]
    Enter,
    Exit,
    Both,
}

impl GeoLocationSink {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.latitude.is_finite() && (-90.0..=90.0).contains(&self.latitude),
            "Geofence latitude must be between -90 and 90"
        );
        ensure!(
            self.longitude.is_finite() && (-180.0..=180.0).contains(&self.longitude),
            "Geofence longitude must be between -180 and 180"
        );
        ensure!(
            self.radius.is_finite() && self.radius >= 100.0 && self.radius <= 100_000.0,
            "Geofence radius must be between 100 and 100000 meters"
        );
        Ok(())
    }

    pub fn registration(&self, scope: &str, app_id: &str, event_id: &str) -> NativeRegistration {
        let transition = match self.trigger_on {
            GeoTriggerType::Enter => "enter",
            GeoTriggerType::Exit => "exit",
            GeoTriggerType::Both => "both",
        };
        // The ID binds a transition to the exact account, Event, and region configuration.
        let content = serde_json::json!([
            scope,
            app_id,
            event_id,
            self.latitude,
            self.longitude,
            self.radius,
            transition,
            self.background
        ]);
        NativeRegistration {
            id: blake3::hash(content.to_string().as_bytes())
                .to_hex()
                .to_string(),
            app_id: app_id.into(),
            event_id: event_id.into(),
            latitude: self.latitude,
            longitude: self.longitude,
            radius_meters: self.radius,
            transition: transition.into(),
            background: self.background,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeRegistration {
    pub id: String,
    pub app_id: String,
    pub event_id: String,
    pub latitude: f64,
    pub longitude: f64,
    pub radius_meters: f64,
    pub transition: String,
    pub background: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transition {
    pub id: String,
    pub registration_id: String,
    pub scope: String,
    pub app_id: String,
    pub event_id: String,
    pub transition: String,
    pub occurred_at: i64,
    pub geometry: serde_json::Value,
    pub radius_meters: f64,
}

impl Transition {
    pub fn matches(&self, scope: &str, registration: &NativeRegistration, now: i64) -> bool {
        !self.id.is_empty()
            && self.id.len() <= 256
            && self.scope == scope
            && self.registration_id == registration.id
            && self.app_id == registration.app_id
            && self.event_id == registration.event_id
            && matches!(self.transition.as_str(), "enter" | "exit")
            && (registration.transition == "both" || self.transition == registration.transition)
            && self.occurred_at > 0
            && self.occurred_at <= now + 60_000
            && now.saturating_sub(self.occurred_at) <= RETENTION_MS
            && self.radius_meters == registration.radius_meters
            && self
                .geometry
                .get("type")
                .and_then(serde_json::Value::as_str)
                == Some("Point")
            && self
                .geometry
                .get("coordinates")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|coordinates| {
                    coordinates.len() == 2
                        && coordinates[0].as_f64() == Some(registration.longitude)
                        && coordinates[1].as_f64() == Some(registration.latitude)
                })
    }
}

#[derive(Debug, Clone)]
pub struct Scope {
    pub raw: String,
    pub origin: String,
    pub profile_id: String,
    pub subject: String,
}

impl Scope {
    pub fn parse(raw: String) -> Result<Self> {
        let [origin, profile_id, subject]: [String; 3] = serde_json::from_str(&raw)?;
        ensure!(
            !profile_id.is_empty() && !subject.is_empty(),
            "Native account scope is incomplete"
        );
        let url = reqwest::Url::parse(&origin)?;
        ensure!(
            matches!(url.scheme(), "https" | "http")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "Native account origin is invalid"
        );
        Ok(Self {
            raw,
            origin: origin.trim_end_matches('/').into(),
            profile_id,
            subject,
        })
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> GeoLocationSink {
        serde_json::from_value(
            serde_json::json!({"latitude":52.5,"longitude":13.4,"radius":200,"trigger_on":"Both"}),
        )
        .unwrap()
    }
    #[test]
    fn legacy_config_is_foreground_and_validates_wgs84_meters() {
        let mut c = config();
        assert!(!c.background);
        assert!(c.validate().is_ok());
        for value in [0.0, -1.0, f64::NAN, f64::INFINITY, 100_001.0] {
            c.radius = value;
            assert!(c.validate().is_err());
        }
        c = config();
        c.latitude = 91.0;
        assert!(c.validate().is_err());
        c = config();
        c.longitude = -181.0;
        assert!(c.validate().is_err());
    }
    #[test]
    fn account_app_event_and_config_changes_rotate_registration() {
        let c = config();
        let original = c.registration("scope", "app", "event").id;
        for next in [
            c.registration("other", "app", "event"),
            c.registration("scope", "other", "event"),
            c.registration("scope", "app", "other"),
        ] {
            assert_ne!(original, next.id);
        }
        let mut changed = c.clone();
        changed.background = true;
        assert_ne!(original, changed.registration("scope", "app", "event").id);
        changed = c.clone();
        changed.radius += 1.0;
        assert_ne!(original, changed.registration("scope", "app", "event").id);
    }
    #[test]
    fn transition_matching_rejects_stale_and_misbound_payloads() {
        let c = config();
        let r = c.registration("scope", "app", "event");
        let now = 10_000_000;
        let event = Transition {
            id: "transition".into(),
            registration_id: r.id.clone(),
            scope: "scope".into(),
            app_id: "app".into(),
            event_id: "event".into(),
            transition: "enter".into(),
            occurred_at: now,
            geometry: serde_json::json!({"type":"Point","coordinates":[13.4,52.5]}),
            radius_meters: 200.0,
        };
        assert!(event.matches("scope", &r, now));
        for field in [
            "scope",
            "appId",
            "eventId",
            "registrationId",
            "transition",
            "geometry",
            "radiusMeters",
            "occurredAt",
        ] {
            let mut v = serde_json::to_value(&event).unwrap();
            v[field] = match field {
                "geometry" => serde_json::json!({"type":"Point","coordinates":[0,0]}),
                "radiusMeters" => serde_json::json!(300.0),
                "occurredAt" => serde_json::json!(now - RETENTION_MS - 1),
                _ => serde_json::json!("wrong"),
            };
            assert!(
                !serde_json::from_value::<Transition>(v)
                    .unwrap()
                    .matches("scope", &r, now),
                "{field}"
            );
        }
        let mut enter = r;
        enter.transition = "exit".into();
        assert!(!event.matches("scope", &enter, now));
    }
    #[test]
    fn native_integer_coordinate_encoding_matches_a_double_region() {
        let mut config = config();
        config.latitude = 52.0;
        config.longitude = 13.0;
        let registration = config.registration("scope", "app", "event");
        let transition: Transition = serde_json::from_value(serde_json::json!({
            "id":"transition", "registrationId":registration.id,"scope":"scope",
            "appId":"app","eventId":"event","transition":"enter","occurredAt":10_000_000,
            "geometry":{"type":"Point","coordinates":[13,52]},"radiusMeters":200
        }))
        .unwrap();
        assert!(transition.matches("scope", &registration, 10_000_000));
        let mut with_altitude = transition;
        with_altitude.geometry["coordinates"] = serde_json::json!([13, 52, 0]);
        assert!(!with_altitude.matches("scope", &registration, 10_000_000));
    }
    #[test]
    fn scopes_require_an_origin_and_an_account_identity() {
        assert!(Scope::parse(r#"["https://example.com","profile","local"]"#.into()).is_ok());
        let with_base_path =
            Scope::parse(r#"["https://example.com/flow/","profile","sub"]"#.into()).unwrap();
        assert_eq!(with_base_path.origin, "https://example.com/flow");
        for raw in [
            r#"["https://u:p@example.com","profile","sub"]"#,
            r#"["file:///tmp","profile","sub"]"#,
            r#"["https://example.com?query","profile","sub"]"#,
            r#"["https://example.com/flow/#fragment","profile","sub"]"#,
            r#"["https://example.com","profile",""]"#,
        ] {
            assert!(Scope::parse(raw.into()).is_err());
        }
    }
}
