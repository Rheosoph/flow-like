use super::model::{
    GRID_SLOTS, LayoutDoc, PlacementContent, PlacementDoc, ROTATION_DEFAULT, RailKey,
    SLOT_CATEGORIES, SLOT_HERO, SLOT_STAT, SLOT_UNPLACED, SlotArea, SlotDoc,
};

/// Placement ids are stable, so a materialized default draft diffs clean against a default live edition.
fn placement(id: &str, name: &str, content: PlacementContent) -> PlacementDoc {
    PlacementDoc {
        id: id.to_owned(),
        kind: content.kind(),
        name: name.to_owned(),
        enabled: true,
        starts_at: None,
        ends_at: None,
        audience: Vec::new(),
        content,
        items: Vec::new(),
        status: None,
        updated_at: None,
        created_at: None,
    }
}

fn rail(id: &str, name: &str, rail: RailKey) -> PlacementDoc {
    placement(id, name, PlacementContent::Rail { rail, title: None })
}

/// The layout an edition without a header row resolves to: an auto-filled spotlight, the stat and category
/// tiles and the system rails.
pub fn default_layout() -> LayoutDoc {
    let grid = GRID_SLOTS
        .iter()
        .enumerate()
        .map(|(position, key)| SlotDoc {
            key: (*key).to_owned(),
            area: SlotArea::Grid,
            position: position as i32,
            placements: match *key {
                SLOT_HERO => vec![placement(
                    "default-hero",
                    "Popular right now",
                    PlacementContent::Spotlight {
                        rotation_seconds: ROTATION_DEFAULT,
                        auto_fill: true,
                    },
                )],
                SLOT_STAT => vec![rail(
                    "default-new-count",
                    "New this week",
                    RailKey::NewCount,
                )],
                SLOT_CATEGORIES => vec![rail(
                    "default-categories",
                    "Categories",
                    RailKey::ByCategory,
                )],
                _ => Vec::new(),
            },
        });
    let rows = [
        (
            "row:trending",
            vec![rail(
                "default-trending",
                "Popular right now",
                RailKey::Trending,
            )],
        ),
        (
            "row:suites",
            vec![rail(
                "default-suites",
                "Suites & Platforms",
                RailKey::Suites,
            )],
        ),
        (
            "row:top-paid",
            vec![rail("default-top-paid", "Top paid", RailKey::TopPaid)],
        ),
        (
            "row:builders",
            vec![
                rail("default-builders", "For builders", RailKey::ForBuilders),
                rail("default-builders-new", "New this week", RailKey::New),
            ],
        ),
    ]
    .into_iter()
    .enumerate()
    .map(|(position, (key, placements))| SlotDoc {
        key: key.to_owned(),
        area: SlotArea::Row,
        position: position as i32,
        placements,
    });
    let unplaced = SlotDoc {
        key: SLOT_UNPLACED.to_owned(),
        area: SlotArea::Unplaced,
        position: 0,
        placements: Vec::new(),
    };
    LayoutDoc {
        slots: grid.chain(rows).chain(std::iter::once(unplaced)).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::explore::model::{Platform, Projection, Viewer, validate_layout};
    use crate::routes::explore::resolve::{Hydrated, candidates, select};

    #[test]
    fn default_layout_validates() {
        let layout = default_layout();
        validate_layout(&layout).unwrap();
        let keys: Vec<&str> = layout.slots.iter().map(|slot| slot.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "hero",
                "notice",
                "feature",
                "collection",
                "stat",
                "categories",
                "row:trending",
                "row:suites",
                "row:top-paid",
                "row:builders",
                "unplaced",
            ]
        );
        assert_eq!(layout, default_layout());
    }

    #[test]
    fn default_layout_resolves_from_system_rails_only() {
        let viewer = Viewer {
            dev: false,
            signed_in: false,
            platform: Platform::Web,
            language: "en".into(),
        };
        let layout = default_layout();
        let candidates = candidates(&layout, &viewer, chrono::Utc::now());
        assert_eq!(
            candidates.rail_keys(),
            [
                RailKey::Trending,
                RailKey::NewCount,
                RailKey::ByCategory,
                RailKey::TopPaid,
                RailKey::ForBuilders,
                RailKey::New,
            ]
        );
        let (view, traces) = select(&candidates, &Hydrated::default(), &viewer, Projection::All);
        assert_eq!(view.rows.len(), 1);
        assert_eq!(traces.len(), 10);
    }
}
