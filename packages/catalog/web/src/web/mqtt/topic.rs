#[cfg(feature = "execute")]
pub(super) fn valid_filter(filter: &str) -> bool {
    if filter.is_empty() || filter.len() > u16::MAX as usize || filter.contains('\0') {
        return false;
    }
    let parts = filter.split('/').collect::<Vec<_>>();
    parts.iter().enumerate().all(|(index, part)| {
        (!part.contains('+') || *part == "+")
            && (!part.contains('#') || (*part == "#" && index + 1 == parts.len()))
    })
}

#[cfg(feature = "execute")]
pub(super) fn topic_matches(filter: &str, topic: &str) -> bool {
    if topic.starts_with('$') && !filter.starts_with('$') {
        return false;
    }
    let mut levels = topic.split('/');
    for part in filter.split('/') {
        if part == "#" {
            return true;
        }
        match levels.next() {
            Some(level) if part == "+" || part == level => {}
            _ => return false,
        }
    }
    levels.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_matching_preserves_utf8_levels_and_system_topic_rules() {
        for (filter, topic, expected) in [
            ("温度/+", "温度/one", true),
            ("+/one", "温度/one", true),
            ("温度/#", "温度", true),
            ("温度/+", "温度", false),
            ("a/+", "a/", true),
            ("+/+", "/", true),
            ("a/#", "a", true),
            ("a/+", "a/b/c", false),
            ("a", "a/", false),
            ("#", "$SYS/broker/uptime", false),
            ("+/broker/#", "$SYS/broker/uptime", false),
            ("$SYS/#", "$SYS/broker/uptime", true),
            ("$SYS/+", "$SYS/温度", true),
            ("a/+", "a/$value", true),
        ] {
            assert!(valid_filter(filter));
            assert_eq!(topic_matches(filter, topic), expected, "{filter}: {topic}");
        }
        for filter in ["", "a/#/b", "a+b", "a/#b", "a\0/b"] {
            assert!(!valid_filter(filter), "{filter}");
        }
    }
}
