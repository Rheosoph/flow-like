use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RuleSection {
    Icann,
    Private,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RuleForm {
    Normal,
    Wildcard,
    Exception,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Rule {
    pub section: RuleSection,
    pub form: RuleForm,
    pub domain: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RulesFile {
    pub psl_version: String,
    pub psl_unix_secs: i64,
    pub rules: Vec<Rule>,
}

pub(super) fn parse_rules_file(text: &str) -> Result<RulesFile, String> {
    let mut psl_version = None;
    let mut rules = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if let Some(header) = line.strip_prefix('#') {
            if let Some(version) = header.trim().strip_prefix("psl-version:") {
                psl_version = Some(version.trim().to_string());
            }
            continue;
        }
        if line.is_empty() {
            continue;
        }
        let rule = parse_rule(line)
            .ok_or_else(|| format!("public_suffix_rules.txt line {}: {line:?}", index + 1))?;
        rules.push(rule);
    }
    let psl_version =
        psl_version.ok_or_else(|| "public_suffix_rules.txt has no psl-version".to_string())?;
    let psl_unix_secs = psl_version_unix_secs(&psl_version)
        .ok_or_else(|| format!("unreadable psl-version {psl_version:?}"))?;
    if !rules.iter().any(|rule| rule.section == RuleSection::Icann) {
        return Err("public_suffix_rules.txt has no ICANN rules".to_string());
    }
    Ok(RulesFile {
        psl_version,
        psl_unix_secs,
        rules,
    })
}

fn parse_rule(line: &str) -> Option<Rule> {
    let (section, rule) = line.split_once(' ')?;
    let section = match section {
        "i" => RuleSection::Icann,
        "p" => RuleSection::Private,
        _ => return None,
    };
    let (form, domain) = if let Some(domain) = rule.strip_prefix("*.") {
        (RuleForm::Wildcard, domain)
    } else if let Some(domain) = rule.strip_prefix('!') {
        (RuleForm::Exception, domain)
    } else {
        (RuleForm::Normal, rule)
    };
    let labels_valid = domain.split('.').all(|label| {
        !label.is_empty()
            && label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    });
    let min_labels = if form == RuleForm::Exception { 2 } else { 1 };
    (labels_valid && domain.split('.').count() >= min_labels).then(|| Rule {
        section,
        form,
        domain: domain.to_string(),
    })
}

/// `YYYY-MM-DD_HH-MM-SS_UTC`, the PSL `VERSION` format.
pub(super) fn psl_version_unix_secs(version: &str) -> Option<i64> {
    let rest = version.strip_suffix("_UTC")?;
    let (date, time) = rest.split_once('_')?;
    let numbers = |part: &str| -> Option<Vec<i64>> {
        let values: Vec<i64> = part
            .split('-')
            .map(|value| value.parse().ok())
            .collect::<Option<_>>()?;
        (values.len() == 3).then_some(values)
    };
    let date = numbers(date)?;
    let time = numbers(time)?;
    let valid = (1..=12).contains(&date[1])
        && (1..=31).contains(&date[2])
        && (0..24).contains(&time[0])
        && (0..60).contains(&time[1])
        && (0..=60).contains(&time[2]);
    valid.then(|| {
        days_from_civil(date[0], date[1], date[2]) * 86_400
            + time[0] * 3_600
            + time[1] * 60
            + time[2]
    })
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let shifted_month = (month + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// One PSL section as the formal algorithm reads it.
#[derive(Debug, Default)]
pub(super) struct SuffixRules {
    normal: HashSet<String>,
    wildcard: HashSet<String>,
    exception: HashSet<String>,
}

impl SuffixRules {
    pub fn insert(&mut self, rule: &Rule) {
        let set = match rule.form {
            RuleForm::Normal => &mut self.normal,
            RuleForm::Wildcard => &mut self.wildcard,
            RuleForm::Exception => &mut self.exception,
        };
        set.insert(rule.domain.clone());
    }

    /// Label count of the public suffix of `host` under this section's
    /// prevailing rule; `implicit_star` applies the PSL default rule `*`.
    pub fn suffix_labels(&self, host: &str, implicit_star: bool) -> Option<usize> {
        let starts = label_starts(host);
        let count = starts.len();
        if let Some(index) = starts
            .iter()
            .position(|&start| self.exception.contains(&host[start..]))
        {
            return Some(count - index - 1);
        }
        let matched = (0..count).find(|&index| {
            self.normal.contains(&host[starts[index]..])
                || starts
                    .get(index + 1)
                    .is_some_and(|&next| self.wildcard.contains(&host[next..]))
        });
        match matched {
            Some(index) => Some(count - index),
            None => implicit_star.then_some(1),
        }
    }
}

pub(super) fn label_starts(host: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(host.match_indices('.').map(|(index, _)| index + 1))
        .collect()
}

pub(super) fn label_count(host: &str) -> usize {
    host.split('.').count()
}

/// The rightmost `labels` labels of `host`.
pub(super) fn last_labels(host: &str, labels: usize) -> &str {
    let starts = label_starts(host);
    let index = starts.len().saturating_sub(labels);
    &host[starts[index]..]
}

/// Strict ancestors of `domain`, nearest first.
pub(super) fn strict_ancestors(domain: &str) -> impl Iterator<Item = &str> {
    domain
        .match_indices('.')
        .map(move |(index, _)| &domain[index + 1..])
}
