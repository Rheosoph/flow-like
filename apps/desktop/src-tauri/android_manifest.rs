use std::{fs, io, path::Path};

const MARKERS: [&str; 2] = [
    "<!-- DEEP LINK PLUGIN. AUTO-GENERATED. DO NOT REMOVE. -->",
    "<!-- tauri-file-associations. AUTO-GENERATED. DO NOT REMOVE. -->",
];

pub fn update(path: &Path) -> io::Result<()> {
    let original = fs::read_to_string(path)?;
    let rewritten = main_activity_filters(&original)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if rewritten != original {
        fs::write(path, rewritten)?;
    }
    Ok(())
}

// Tauri inserts these generated blocks into every activity. Auxiliary activities keep only
// their own intent filters, so app links and imported files reach the main activity.
fn main_activity_filters(xml: &str) -> Result<String, String> {
    let mut cursor = 0;
    let mut activity: Option<bool> = None;
    let mut main_count = 0;
    let mut block: Option<(usize, usize)> = None;
    let mut removals = Vec::new();

    while let Some(relative) = xml[cursor..].find('<') {
        let start = cursor + relative;
        if xml[start..].starts_with("<!--") {
            let end = start
                + 4
                + xml[start + 4..]
                    .find("-->")
                    .ok_or("Unterminated manifest comment")?
                + 3;
            if let Some(marker) = MARKERS.iter().position(|value| *value == &xml[start..end]) {
                let is_main = activity.ok_or("Generated filter block is outside an activity")?;
                let line_start = xml[..start].rfind('\n').map_or(0, |index| index + 1);
                let line_end = xml[end..]
                    .find('\n')
                    .map_or(xml.len(), |index| end + index + 1);
                if !xml[line_start..start].trim().is_empty()
                    || !xml[end..line_end].trim().is_empty()
                {
                    return Err("Generated filter markers must be on separate lines".into());
                }
                if let Some((opened, block_start)) = block.take() {
                    if opened != marker {
                        return Err("Generated filter markers are nested or mismatched".into());
                    }
                    if !is_main {
                        removals.push((block_start, line_end));
                    }
                } else {
                    block = Some((marker, line_start));
                }
            }
            cursor = end;
            continue;
        }

        let end = tag_end(xml, start)?;
        let tag = xml[start + 1..end - 1].trim();
        let closing = tag.starts_with('/');
        let body = tag.trim_start_matches('/');
        let name = body
            .split(|character: char| character.is_whitespace() || character == '/')
            .next()
            .unwrap_or_default();
        if name == "activity" {
            if block.is_some() {
                return Err("Generated filter block crosses an activity boundary".into());
            }
            if closing {
                if activity.take().is_none() {
                    return Err("Closing activity has no opening tag".into());
                }
            } else {
                if activity.is_some() {
                    return Err("Manifest activities cannot be nested".into());
                }
                let is_main = matches!(
                    activity_name(body)?,
                    "MainActivity" | ".MainActivity" | "com.flow_like.app.MainActivity"
                );
                if is_main {
                    main_count += 1;
                }
                if !body.ends_with('/') {
                    activity = Some(is_main);
                }
            }
        }
        cursor = end;
    }

    if block.is_some() || activity.is_some() {
        return Err("Manifest has an unclosed activity or generated filter block".into());
    }
    if main_count != 1 {
        return Err(format!("Expected one MainActivity, found {main_count}"));
    }

    let mut rewritten = String::with_capacity(xml.len());
    let mut copied = 0;
    for (start, end) in removals {
        rewritten.push_str(&xml[copied..start]);
        copied = end;
    }
    rewritten.push_str(&xml[copied..]);
    Ok(rewritten
        .split('\n')
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n"))
}

fn tag_end(xml: &str, start: usize) -> Result<usize, String> {
    let mut quote = None;
    for (offset, character) in xml[start + 1..].char_indices() {
        match (quote, character) {
            (Some(opened), value) if opened == value => quote = None,
            (None, '\'' | '"') => quote = Some(character),
            (None, '>') => return Ok(start + offset + 2),
            _ => {}
        }
    }
    Err("Unterminated manifest tag".into())
}

fn activity_name(tag: &str) -> Result<&str, String> {
    let mut attributes = tag["activity".len()..].trim().trim_end_matches('/').trim();
    let mut name = None;
    while !attributes.is_empty() {
        let (key, rest) = attributes
            .split_once('=')
            .ok_or("Malformed activity attribute")?;
        let key = key.trim();
        if key.is_empty() || key.chars().any(char::is_whitespace) {
            return Err("Malformed activity attribute name".into());
        }
        let rest = rest.trim_start();
        let quote = rest
            .chars()
            .next()
            .ok_or("Missing activity attribute value")?;
        if !matches!(quote, '\'' | '"') {
            return Err("Activity attributes must be quoted".into());
        }
        let (value, rest) = rest[1..]
            .split_once(quote)
            .ok_or("Unterminated activity attribute")?;
        if key == "android:name" && name.replace(value).is_some() {
            return Err("Activity has duplicate android:name attributes".into());
        }
        attributes = rest.trim_start();
    }
    name.ok_or_else(|| "Activity is missing android:name".into())
}

#[cfg(test)]
mod tests {
    use super::{MARKERS, main_activity_filters};

    const LINKS: &str = "<intent-filter><data android:pathPrefix='/bots' /><data android:pathPrefix='/inbox' /></intent-filter>";
    const FILES: &str =
        "<intent-filter><data android:mimeType='application/vnd.flow-like.flow' /></intent-filter>";

    fn generated(marker: &str, content: &str) -> String {
        format!("            {marker}\n            {content}\n            {marker}\n")
    }

    fn manifest() -> String {
        format!(
            "<manifest><application>\n\
             <activity\n android:label='Main > screen'\n android:name='com.flow_like.app.MainActivity'>\n\
             <intent-filter><action android:name='android.intent.action.MAIN' /></intent-filter>\n\
             {}{}</activity>\n\
             <activity android:name='.NativeAutomationActivity' android:exported='true'>\n\
             <intent-filter><action android:name='com.flow_like.app.AUTOMATE' /></intent-filter>\n\
             {}{}</activity>\n\
             <activity android:name='.NativeSearchActivity'>\n\
             <intent-filter><data android:scheme='flow-like' android:host='native-search' /></intent-filter>\n\
             {}</activity>\n\
             <activity android:name='.BotActionActivity' android:exported='false' />\n\
             <service android:name='.BackgroundHostingService' android:foregroundServiceType='specialUse' />\n\
             </application></manifest>\n",
            generated(MARKERS[0], LINKS),
            generated(MARKERS[1], FILES),
            generated(
                MARKERS[0],
                "<intent-filter><data android:scheme='flow-like' /></intent-filter>"
            ),
            generated(
                MARKERS[1],
                "<intent-filter><data android:mimeType='application/vnd.flow-like.flow' /></intent-filter>"
            ),
            generated(
                MARKERS[0],
                "<intent-filter><data android:scheme='flow-like' /></intent-filter>"
            ),
        )
    }

    #[test]
    fn retains_main_filters_and_custom_auxiliary_components() {
        let output = main_activity_filters(&manifest()).unwrap();
        for marker in MARKERS {
            assert_eq!(output.matches(marker).count(), 2);
        }
        for retained in [
            "android.intent.action.MAIN",
            "android:pathPrefix='/bots'",
            "android:pathPrefix='/inbox'",
            "com.flow_like.app.AUTOMATE",
            "android:host='native-search'",
            "android:name='.BotActionActivity'",
            "android:foregroundServiceType='specialUse'",
        ] {
            assert!(
                output.contains(retained),
                "Lost custom manifest content: {retained}"
            );
        }
        assert_eq!(output.matches("application/vnd.flow-like.flow").count(), 1);
    }

    #[test]
    fn regeneration_and_repeated_cleanup_produce_the_same_manifest() {
        let clean = main_activity_filters(&manifest()).unwrap();
        assert_eq!(main_activity_filters(&clean).unwrap(), clean);
        let mut injected = clean.clone();
        for (marker, content) in [(MARKERS[0], LINKS), (MARKERS[1], FILES)] {
            let mut inside = false;
            let without_old = injected
                .split('\n')
                .filter(|line| {
                    if line.contains(marker) {
                        inside = !inside;
                        false
                    } else {
                        !inside
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            injected = without_old.replace(
                "</activity>",
                &format!("{}</activity>", generated(marker, content)),
            );
        }
        let regenerated = main_activity_filters(&injected).unwrap();
        assert_eq!(regenerated, clean);
        assert_eq!(main_activity_filters(&regenerated).unwrap(), regenerated);
    }

    #[test]
    fn recognizes_only_exact_main_activity_names_and_trims_line_endings() {
        for name in [
            "MainActivity",
            ".MainActivity",
            "com.flow_like.app.MainActivity",
        ] {
            let input = format!("<activity android:name='{name}'>  \n</activity>\t\n");
            assert_eq!(
                main_activity_filters(&input).unwrap(),
                input.replace("  ", "").replace('\t', "")
            );
        }
        assert!(main_activity_filters("<activity android:name='.OtherMainActivity' />").is_err());
        assert!(
            main_activity_filters(
                "<activity android:name='.MainActivity' /><activity android:name='.MainActivity' />"
            )
            .is_err()
        );
    }

    #[test]
    fn refuses_unbalanced_mismatched_and_out_of_activity_markers() {
        for input in [
            format!(
                "<activity android:name='.MainActivity'>\n{}\n</activity>\n",
                MARKERS[0]
            ),
            format!(
                "<activity android:name='.MainActivity'>\n{}\n{}\n</activity>\n",
                MARKERS[0], MARKERS[1]
            ),
            format!(
                "{}\n<activity android:name='.MainActivity' />\n",
                MARKERS[0]
            ),
            "<activity android:name='.MainActivity'>\n<!-- unfinished".into(),
            "<activity android:name='.MainActivity'".into(),
        ] {
            assert!(
                main_activity_filters(&input).is_err(),
                "Accepted malformed manifest: {input}"
            );
        }
    }
}
