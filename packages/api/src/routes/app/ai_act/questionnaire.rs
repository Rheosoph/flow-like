//! Canonical EU AI Act questionnaire schema + deterministic classifier.
//!
//! The schema is served to the publishing wizard (Screens 0-4 and the
//! High-risk branch H1-H3, see todo/EU-AI.md §3). The classifier
//! ([`classify`]) is the single source of truth for risk category,
//! conformity score and transparency obligations and is always recomputed
//! server-side (§4) so the client cannot tamper with the result.

use super::signals::Signals;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Schema types (served to the frontend)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum QuestionKind {
    /// Single choice from `options`.
    Select,
    /// Multiple choice from `options`.
    Multi,
    /// Yes / No toggle (values "yes" | "no").
    YesNo,
    /// Free text.
    Text,
    /// Responsible person contact (name + email object).
    Contact,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuestionOption {
    pub value: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub key: String,
    pub label: String,
    pub kind: QuestionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<QuestionOption>,
    /// Whether the question must be answered to submit.
    #[serde(default)]
    pub required: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Screen {
    pub id: String,
    pub title: String,
    pub description: String,
    pub questions: Vec<Question>,
    /// Only shown when the app is (or may be) high-risk.
    #[serde(default)]
    pub high_risk_only: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuestionnaireSchema {
    pub version: i32,
    pub screens: Vec<Screen>,
}

/// Questionnaire schema version. Bump when questions change so stored
/// assessments can be migrated / re-reviewed.
pub const QUESTIONNAIRE_VERSION: i32 = 1;

// ---------------------------------------------------------------------------
// Stable keys
// ---------------------------------------------------------------------------

pub mod keys {
    pub const PROHIBITED: &str = "prohibited_practices";
    pub const PURPOSE: &str = "purpose";
    pub const EU_USERS: &str = "eu_users";
    pub const CONSEQUENTIAL: &str = "consequential_decisions";
    pub const CHATBOT: &str = "interacts_with_people";
    pub const GENAI: &str = "generates_content";
    pub const EMOTION_BIOMETRIC: &str = "emotion_or_biometric";
    pub const HUMAN_OVERSIGHT: &str = "human_oversight";
    pub const PERSONAL_DATA: &str = "uses_personal_data";
    pub const INSTRUCTIONS: &str = "instructions_documented";
    pub const RESPONSIBLE: &str = "responsible_person";
}

/// Sentinel value meaning "I am not sure" for pivotal questions; forces an
/// UNDETERMINED outcome so the result is never silently optimistic.
pub const UNSURE: &str = "unsure";

// ---------------------------------------------------------------------------
// Prohibited (Art. 5) and consequential (Annex III) catalogues
// ---------------------------------------------------------------------------

/// Art. 5 prohibited practices. Selecting any one blocks publication.
pub const PROHIBITED_PRACTICES: &[(&str, &str)] = &[
    (
        "social_scoring",
        "Scores or ranks people based on social behaviour or personal traits",
    ),
    (
        "subliminal_manipulation",
        "Uses subliminal or manipulative techniques to distort behaviour",
    ),
    (
        "exploit_vulnerabilities",
        "Exploits vulnerabilities of age, disability or social/economic situation",
    ),
    (
        "biometric_categorisation_sensitive",
        "Infers sensitive attributes (race, beliefs, sexual orientation) from biometrics",
    ),
    (
        "realtime_biometric_public",
        "Performs real-time remote biometric identification in public spaces",
    ),
    (
        "predictive_policing_individual",
        "Predicts criminal offences based solely on profiling a person",
    ),
    (
        "emotion_workplace_education",
        "Infers emotions in the workplace or educational settings",
    ),
    (
        "facial_scraping",
        "Builds facial-recognition databases by untargeted scraping",
    ),
];

/// Annex III consequential decision domains. Selecting any one makes the app
/// high-risk.
pub const CONSEQUENTIAL_DOMAINS: &[(&str, &str)] = &[
    (
        "biometric_id",
        "Biometric identification or categorisation of people",
    ),
    (
        "critical_infrastructure",
        "Safety of critical infrastructure",
    ),
    (
        "education_access",
        "Access to education or evaluation of learning",
    ),
    (
        "employment",
        "Recruitment, hiring, promotion or termination",
    ),
    (
        "essential_services",
        "Access to essential public or private services / credit",
    ),
    (
        "law_enforcement",
        "Law-enforcement use affecting individuals",
    ),
    (
        "migration_border",
        "Migration, asylum or border-control management",
    ),
    (
        "justice_democracy",
        "Administration of justice or democratic processes",
    ),
];

// ---------------------------------------------------------------------------
// Classification result
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum RiskCategory {
    Prohibited,
    High,
    Limited,
    Minimal,
    Undetermined,
}

impl RiskCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            RiskCategory::Prohibited => "PROHIBITED",
            RiskCategory::High => "HIGH",
            RiskCategory::Limited => "LIMITED",
            RiskCategory::Minimal => "MINIMAL",
            RiskCategory::Undetermined => "UNDETERMINED",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConformityBand {
    Green,
    Amber,
    Red,
}

impl ConformityBand {
    pub fn from_score(score: i32) -> Self {
        if score >= 80 {
            ConformityBand::Green
        } else if score >= 50 {
            ConformityBand::Amber
        } else {
            ConformityBand::Red
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ConformityBand::Green => "green",
            ConformityBand::Amber => "amber",
            ConformityBand::Red => "red",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransparencyObligation {
    /// Art. 50(1): tell users they are interacting with an AI system.
    DiscloseAiInteraction,
    /// Art. 50(2): mark AI-generated content as artificial.
    LabelGeneratedContent,
    /// Art. 50(3): inform people subject to emotion/biometric systems.
    DiscloseEmotionBiometric,
    /// Annex III high-risk: human oversight required.
    HumanOversight,
    /// Annex III high-risk: technical documentation & logging.
    TechnicalDocumentation,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Classification {
    pub risk_category: RiskCategory,
    /// 0-100; `None` only when prohibited (publication blocked) or undetermined.
    pub conformity_score: Option<i32>,
    pub conformity_band: Option<ConformityBand>,
    pub transparency_obligations: Vec<TransparencyObligation>,
    /// True when an Art. 5 prohibited practice was selected -> hard block.
    pub blocked: bool,
    /// Human-readable explanation of the dominant factors (audit trail).
    pub rationale: Vec<String>,
}

// ---------------------------------------------------------------------------
// Answer access helpers
// ---------------------------------------------------------------------------

fn answer_str<'a>(answers: &'a Value, key: &str) -> Option<&'a str> {
    answers.get(key).and_then(Value::as_str)
}

fn answer_yes(answers: &Value, key: &str) -> Option<bool> {
    match answer_str(answers, key) {
        Some("yes") => Some(true),
        Some("no") => Some(false),
        _ => None,
    }
}

fn answer_list<'a>(answers: &'a Value, key: &str) -> Vec<&'a str> {
    answers
        .get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

fn is_unsure(answers: &Value, key: &str) -> bool {
    answer_str(answers, key) == Some(UNSURE)
}

// ---------------------------------------------------------------------------
// Classifier
// ---------------------------------------------------------------------------

/// Deterministically classify an app from its questionnaire answers and the
/// auto-derived [`Signals`]. Pure function — no I/O — so it is trivially
/// testable and identical on every deployment target.
pub fn classify(answers: &Value, signals: &Signals) -> Classification {
    let mut rationale: Vec<String> = Vec::new();

    // 1. Prohibited (Art. 5) — hard block, short-circuit.
    let prohibited = answer_list(answers, keys::PROHIBITED);
    if !prohibited.is_empty() {
        rationale.push(format!(
            "Art. 5 prohibited practice declared: {}",
            prohibited.join(", ")
        ));
        return Classification {
            risk_category: RiskCategory::Prohibited,
            conformity_score: None,
            conformity_band: None,
            transparency_obligations: Vec::new(),
            blocked: true,
            rationale,
        };
    }

    // 2. Undetermined — any pivotal question answered "unsure".
    let pivotal = [
        keys::CONSEQUENTIAL,
        keys::CHATBOT,
        keys::GENAI,
        keys::EMOTION_BIOMETRIC,
    ];
    if pivotal.iter().any(|k| is_unsure(answers, k)) {
        rationale.push("One or more pivotal questions answered as 'not sure'".to_string());
        return Classification {
            risk_category: RiskCategory::Undetermined,
            conformity_score: None,
            conformity_band: None,
            transparency_obligations: Vec::new(),
            blocked: false,
            rationale,
        };
    }

    // 3. High-risk (Annex III) — any consequential domain selected.
    let consequential = answer_list(answers, keys::CONSEQUENTIAL);
    let is_high = !consequential.is_empty();

    // 4. Transparency obligations (Art. 50) — independent of high-risk.
    let mut obligations: Vec<TransparencyObligation> = Vec::new();
    let chatbot = answer_yes(answers, keys::CHATBOT).unwrap_or(signals.capabilities.has_chatbot);
    let genai = answer_yes(answers, keys::GENAI).unwrap_or(signals.capabilities.has_genai);
    let emotion = answer_yes(answers, keys::EMOTION_BIOMETRIC)
        .unwrap_or(signals.capabilities.has_emotion_biometric);

    if chatbot {
        obligations.push(TransparencyObligation::DiscloseAiInteraction);
    }
    if genai {
        obligations.push(TransparencyObligation::LabelGeneratedContent);
    }
    if emotion {
        obligations.push(TransparencyObligation::DiscloseEmotionBiometric);
    }

    let risk_category = if is_high {
        rationale.push(format!(
            "Annex III high-risk domain(s) selected: {}",
            consequential.join(", ")
        ));
        obligations.push(TransparencyObligation::HumanOversight);
        obligations.push(TransparencyObligation::TechnicalDocumentation);
        RiskCategory::High
    } else if !obligations.is_empty() {
        rationale.push("Limited-risk transparency obligations apply (Art. 50)".to_string());
        RiskCategory::Limited
    } else {
        rationale.push("No high-risk or transparency triggers detected".to_string());
        RiskCategory::Minimal
    };

    // 5. Conformity score (§4.2).
    let (score, score_rationale) = conformity_score(answers, signals, risk_category, &obligations);
    rationale.extend(score_rationale);
    let band = ConformityBand::from_score(score);

    Classification {
        risk_category,
        conformity_score: Some(score),
        conformity_band: Some(band),
        transparency_obligations: obligations,
        blocked: false,
        rationale,
    }
}

/// Weighted conformity score (0-100). Weights per todo/EU-AI.md §4.2:
/// transparency 25, board security/governance 25, human oversight 20,
/// data governance 15, model posture 15.
fn conformity_score(
    answers: &Value,
    signals: &Signals,
    risk: RiskCategory,
    obligations: &[TransparencyObligation],
) -> (i32, Vec<String>) {
    let mut rationale = Vec::new();

    // Transparency (25): satisfied when all triggered obligations are
    // acknowledged. With no obligations, full marks.
    let transparency = if obligations.is_empty() {
        25.0
    } else {
        // Acknowledged when the corresponding answer is affirmative; the
        // wizard requires explicit acknowledgement of each obligation.
        let ack = answer_yes(answers, "ack_transparency").unwrap_or(false);
        if ack { 25.0 } else { 8.0 }
    };

    // Board security/governance (25): scale MIN(security, governance) 0-10 -> 0-25.
    let sec = signals.min_security.unwrap_or(5);
    let gov = signals.min_governance.unwrap_or(5);
    let board_min = sec.min(gov).clamp(0, 10) as f32;
    let board = board_min / 10.0 * 25.0;
    if signals.min_security.is_none() || signals.min_governance.is_none() {
        rationale.push("Board not fully scored; assumed neutral board posture".to_string());
    }

    // Human oversight (20): only weighted when high-risk; otherwise auto-granted.
    let oversight = if risk == RiskCategory::High {
        match answer_yes(answers, keys::HUMAN_OVERSIGHT) {
            Some(true) => 20.0,
            Some(false) => 0.0,
            None => 6.0,
        }
    } else {
        20.0
    };

    // Data governance (15): personal-data handling acknowledged.
    let data = match answer_yes(answers, keys::PERSONAL_DATA) {
        Some(true) => {
            // Personal data used -> require documented instructions.
            if answer_yes(answers, keys::INSTRUCTIONS).unwrap_or(false) {
                15.0
            } else {
                6.0
            }
        }
        Some(false) => 15.0,
        None => 9.0,
    };

    // Model posture (15): penalise unvetted / systemic-risk / dynamic models.
    let posture = model_posture_score(signals);
    if signals.models.iter().any(|m| m.dynamic_selector) {
        rationale
            .push("Dynamic model selector present; posture cannot be fully verified".to_string());
    }

    let total = (transparency + board + oversight + data + posture).round() as i32;
    rationale.push(format!(
        "Score breakdown — transparency {:.0}, board {:.0}, oversight {:.0}, data {:.0}, posture {:.0}",
        transparency, board, oversight, data, posture
    ));
    (total.clamp(0, 100), rationale)
}

/// 0-15 model posture component. Full marks when no models or all observed
/// models are vetted with a known posture; reduced for dynamic selectors and
/// unknown models.
fn model_posture_score(signals: &Signals) -> f32 {
    if signals.models.is_empty() {
        return 15.0;
    }
    let has_dynamic = signals.models.iter().any(|m| m.dynamic_selector);
    let known = signals
        .models
        .iter()
        .filter(|m| m.provider.is_some() && !m.dynamic_selector)
        .count();
    let ratio = known as f32 / signals.models.len() as f32;
    let base = 15.0 * ratio;
    if has_dynamic {
        (base - 4.0).max(0.0)
    } else {
        base
    }
}

// ---------------------------------------------------------------------------
// Conformity recommendations
// ---------------------------------------------------------------------------

/// A single actionable tip for raising the conformity score (or unblocking a
/// blocked / undetermined assessment). Mirrors the weighting in
/// [`conformity_score`] so the estimated uplift is faithful to the model.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Recommendation {
    /// Stable identifier for the tip (used as a React key).
    pub id: String,
    /// Short, imperative title shown as the tip headline.
    pub title: String,
    /// One or two sentences explaining what to do and why.
    pub detail: String,
    /// Grouping label: Transparency, Board, Oversight, Data, Posture or
    /// Classification.
    pub category: String,
    /// Estimated points the conformity score could gain by resolving this tip.
    pub potential_points: i32,
    /// Relevant EU AI Act article reference, when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub article: Option<String>,
    /// Questionnaire answer key the reviewer/owner should revisit, when the tip
    /// maps to a specific question.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer_key: Option<String>,
}

/// Build the ordered list of conformity recommendations for an app from its
/// current answers, auto-derived signals and live classification. Pure
/// function — the same weighting as [`conformity_score`] — so the estimated
/// uplift always matches what would actually happen if the tip were resolved.
pub fn recommendations(
    answers: &Value,
    signals: &Signals,
    classification: &Classification,
) -> Vec<Recommendation> {
    let mut recs: Vec<Recommendation> = Vec::new();

    if classification.blocked {
        recs.push(Recommendation {
            id: "remove-prohibited".to_string(),
            title: "Remove the prohibited practice".to_string(),
            detail:
                "An Art. 5 prohibited practice is declared, which blocks publication entirely. Remove the prohibited capability or correct the questionnaire answer before this app can be scored."
                    .to_string(),
            category: "Classification".to_string(),
            potential_points: 0,
            article: Some("Art. 5".to_string()),
            answer_key: Some(keys::PROHIBITED.to_string()),
        });
        return recs;
    }

    if classification.risk_category == RiskCategory::Undetermined {
        recs.push(Recommendation {
            id: "resolve-unsure".to_string(),
            title: "Resolve 'not sure' answers".to_string(),
            detail:
                "One or more pivotal questions are answered 'not sure', so the system cannot be classified or scored. Provide a definite answer to compute the conformity score."
                    .to_string(),
            category: "Classification".to_string(),
            potential_points: 0,
            article: None,
            answer_key: Some(keys::CONSEQUENTIAL.to_string()),
        });
        return recs;
    }

    let obligations = &classification.transparency_obligations;
    let risk = classification.risk_category;

    // Transparency (25): acknowledge the triggered Art. 50 duties.
    if !obligations.is_empty() && !answer_yes(answers, "ack_transparency").unwrap_or(false) {
        recs.push(Recommendation {
            id: "ack-transparency".to_string(),
            title: "Acknowledge transparency obligations".to_string(),
            detail:
                "Confirm that the triggered Art. 50 duties — disclosing AI interaction, labelling generated content and informing people of emotion/biometric processing where relevant — are implemented. This lifts the transparency component to full marks."
                    .to_string(),
            category: "Transparency".to_string(),
            potential_points: 17,
            article: Some("Art. 50".to_string()),
            answer_key: Some("ack_transparency".to_string()),
        });
    }

    // Board security/governance (25): scales with MIN(security, governance).
    let sec = signals.min_security.unwrap_or(5);
    let gov = signals.min_governance.unwrap_or(5);
    let board_min = sec.min(gov).clamp(0, 10);
    let board = board_min as f32 / 10.0 * 25.0;
    let board_gap = (25.0 - board).round() as i32;
    if signals.min_security.is_none() || signals.min_governance.is_none() {
        recs.push(Recommendation {
            id: "scan-board".to_string(),
            title: "Run a full board security scan".to_string(),
            detail:
                "Board security and governance are not fully scored, so a neutral posture is assumed. Run the static analysis on every board to score this component on real data."
                    .to_string(),
            category: "Board".to_string(),
            potential_points: board_gap.max(0),
            article: None,
            answer_key: None,
        });
    } else if board_gap > 0 {
        let weakest = if sec <= gov { "security" } else { "governance" };
        recs.push(Recommendation {
            id: "improve-board".to_string(),
            title: format!("Raise the board {weakest} score (now {board_min}/10)"),
            detail:
                "The conformity score scales with the lowest of your security and governance board scores. Fix the flagged low-score nodes to lift this component."
                    .to_string(),
            category: "Board".to_string(),
            potential_points: board_gap,
            article: None,
            answer_key: None,
        });
    }

    // Human oversight (20): only weighted for high-risk systems.
    if risk == RiskCategory::High {
        match answer_yes(answers, keys::HUMAN_OVERSIGHT) {
            Some(true) => {}
            Some(false) => recs.push(Recommendation {
                id: "human-oversight".to_string(),
                title: "Add meaningful human oversight".to_string(),
                detail:
                    "High-risk systems require a human able to review and override decisions before they take effect (Art. 14). Introduce an oversight step, then update the questionnaire."
                        .to_string(),
                category: "Oversight".to_string(),
                potential_points: 20,
                article: Some("Art. 14".to_string()),
                answer_key: Some(keys::HUMAN_OVERSIGHT.to_string()),
            }),
            None => recs.push(Recommendation {
                id: "human-oversight-confirm".to_string(),
                title: "Confirm human oversight".to_string(),
                detail:
                    "This high-risk system has no answer for human oversight, so only partial credit is given. Confirm whether a person can review and override decisions (Art. 14)."
                        .to_string(),
                category: "Oversight".to_string(),
                potential_points: 14,
                article: Some("Art. 14".to_string()),
                answer_key: Some(keys::HUMAN_OVERSIGHT.to_string()),
            }),
        }
    }

    // Data governance (15): documented instructions when personal data is used.
    match answer_yes(answers, keys::PERSONAL_DATA) {
        Some(true) => {
            if !answer_yes(answers, keys::INSTRUCTIONS).unwrap_or(false) {
                recs.push(Recommendation {
                    id: "document-instructions".to_string(),
                    title: "Document intended use and limitations".to_string(),
                    detail:
                        "This app processes personal data but its intended use and limitations are not documented. Publish clear usage instructions to satisfy data-governance duties."
                            .to_string(),
                    category: "Data".to_string(),
                    potential_points: 9,
                    article: Some("Art. 13".to_string()),
                    answer_key: Some(keys::INSTRUCTIONS.to_string()),
                });
            }
        }
        Some(false) => {}
        None => recs.push(Recommendation {
            id: "confirm-personal-data".to_string(),
            title: "Confirm personal-data handling".to_string(),
            detail:
                "Whether the app processes personal data is unanswered, so only partial data-governance credit is given. Confirm the answer to score this component."
                    .to_string(),
            category: "Data".to_string(),
            potential_points: 6,
            article: None,
            answer_key: Some(keys::PERSONAL_DATA.to_string()),
        }),
    }

    // Model posture (15): vet and pin attached models.
    if !signals.models.is_empty() {
        let posture = model_posture_score(signals);
        let gap = (15.0 - posture).round() as i32;
        if gap > 0 {
            let has_dynamic = signals.models.iter().any(|m| m.dynamic_selector);
            let detail = if has_dynamic {
                "A dynamic model selector means the model actually used cannot be verified. Pin to specific, vetted models and register their GPAI posture in the model registry."
            } else {
                "Some attached models have an unknown provider or posture. Register them in the GPAI model registry with a known posture to raise this component."
            };
            recs.push(Recommendation {
                id: "vet-models".to_string(),
                title: "Vet and pin attached models".to_string(),
                detail: detail.to_string(),
                category: "Posture".to_string(),
                potential_points: gap,
                article: Some("Art. 53".to_string()),
                answer_key: None,
            });
        }
    }

    recs.sort_by(|a, b| b.potential_points.cmp(&a.potential_points));
    recs
}

// ---------------------------------------------------------------------------
// Schema definition
// ---------------------------------------------------------------------------

fn options_from(pairs: &[(&str, &str)]) -> Vec<QuestionOption> {
    pairs
        .iter()
        .map(|(value, label)| QuestionOption {
            value: (*value).to_string(),
            label: (*label).to_string(),
            help: None,
        })
        .collect()
}

/// The canonical questionnaire served to the publishing wizard.
pub fn questionnaire_schema() -> QuestionnaireSchema {
    QuestionnaireSchema {
        version: QUESTIONNAIRE_VERSION,
        screens: vec![
            Screen {
                id: "prohibited".to_string(),
                title: "Prohibited uses".to_string(),
                description: "EU AI Act Art. 5 forbids these uses entirely. Select any that apply — selecting one blocks publication.".to_string(),
                questions: vec![Question {
                    key: keys::PROHIBITED.to_string(),
                    label: "Does your app do any of the following?".to_string(),
                    kind: QuestionKind::Multi,
                    help: Some("If none apply, leave all unchecked.".to_string()),
                    options: options_from(PROHIBITED_PRACTICES),
                    required: false,
                }],
                high_risk_only: false,
            },
            Screen {
                id: "purpose".to_string(),
                title: "Purpose & reach".to_string(),
                description: "Describe what your app does and who uses it.".to_string(),
                questions: vec![
                    Question {
                        key: keys::PURPOSE.to_string(),
                        label: "In one sentence, what does your app do?".to_string(),
                        kind: QuestionKind::Text,
                        help: Some("We pre-fill a suggestion from your boards — edit as needed.".to_string()),
                        options: vec![],
                        required: true,
                    },
                    Question {
                        key: keys::EU_USERS.to_string(),
                        label: "Could people in the EU use it?".to_string(),
                        kind: QuestionKind::YesNo,
                        help: None,
                        options: vec![],
                        required: true,
                    },
                ],
                high_risk_only: false,
            },
            Screen {
                id: "consequential".to_string(),
                title: "Consequential decisions".to_string(),
                description: "EU AI Act Annex III. Select any domain where your app materially influences decisions about people.".to_string(),
                questions: vec![Question {
                    key: keys::CONSEQUENTIAL.to_string(),
                    label: "Does your app influence decisions in any of these areas?".to_string(),
                    kind: QuestionKind::Multi,
                    help: Some("Choose 'Not sure' below if uncertain — we will mark it for review.".to_string()),
                    options: {
                        let mut o = options_from(CONSEQUENTIAL_DOMAINS);
                        o.push(QuestionOption {
                            value: UNSURE.to_string(),
                            label: "Not sure".to_string(),
                            help: None,
                        });
                        o
                    },
                    required: false,
                }],
                high_risk_only: false,
            },
            Screen {
                id: "transparency".to_string(),
                title: "Interaction & content".to_string(),
                description: "Limited-risk transparency duties (Art. 50).".to_string(),
                questions: vec![
                    Question {
                        key: keys::CHATBOT.to_string(),
                        label: "Do people interact with your app conversationally (chatbot/agent)?".to_string(),
                        kind: QuestionKind::YesNo,
                        help: None,
                        options: vec![],
                        required: true,
                    },
                    Question {
                        key: keys::GENAI.to_string(),
                        label: "Does your app generate text, images, audio or video?".to_string(),
                        kind: QuestionKind::YesNo,
                        help: None,
                        options: vec![],
                        required: true,
                    },
                    Question {
                        key: keys::EMOTION_BIOMETRIC.to_string(),
                        label: "Does your app infer emotions or categorise people from biometrics?".to_string(),
                        kind: QuestionKind::YesNo,
                        help: None,
                        options: vec![],
                        required: true,
                    },
                ],
                high_risk_only: false,
            },
            Screen {
                id: "high_risk".to_string(),
                title: "High-risk obligations".to_string(),
                description: "Because your app may influence consequential decisions, a few extra duties apply.".to_string(),
                questions: vec![
                    Question {
                        key: keys::HUMAN_OVERSIGHT.to_string(),
                        label: "Is there meaningful human oversight before decisions take effect?".to_string(),
                        kind: QuestionKind::YesNo,
                        help: None,
                        options: vec![],
                        required: true,
                    },
                    Question {
                        key: keys::PERSONAL_DATA.to_string(),
                        label: "Does your app process personal data?".to_string(),
                        kind: QuestionKind::YesNo,
                        help: None,
                        options: vec![],
                        required: true,
                    },
                    Question {
                        key: keys::INSTRUCTIONS.to_string(),
                        label: "Have you documented intended use and limitations for users?".to_string(),
                        kind: QuestionKind::YesNo,
                        help: None,
                        options: vec![],
                        required: true,
                    },
                ],
                high_risk_only: true,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn prohibited_blocks() {
        let answers = json!({ keys::PROHIBITED: ["social_scoring"] });
        let c = classify(&answers, &Signals::default());
        assert_eq!(c.risk_category, RiskCategory::Prohibited);
        assert!(c.blocked);
        assert!(c.conformity_score.is_none());
    }

    #[test]
    fn unsure_is_undetermined() {
        let answers = json!({ keys::CONSEQUENTIAL: UNSURE });
        let c = classify(&answers, &Signals::default());
        assert_eq!(c.risk_category, RiskCategory::Undetermined);
    }

    #[test]
    fn high_risk_from_consequential() {
        let answers = json!({ keys::CONSEQUENTIAL: ["employment"] });
        let c = classify(&answers, &Signals::default());
        assert_eq!(c.risk_category, RiskCategory::High);
        assert!(
            c.transparency_obligations
                .contains(&TransparencyObligation::HumanOversight)
        );
    }

    #[test]
    fn limited_from_chatbot() {
        let answers = json!({ keys::CHATBOT: "yes" });
        let c = classify(&answers, &Signals::default());
        assert_eq!(c.risk_category, RiskCategory::Limited);
        assert!(
            c.transparency_obligations
                .contains(&TransparencyObligation::DiscloseAiInteraction)
        );
    }

    #[test]
    fn minimal_default() {
        let answers =
            json!({ keys::CHATBOT: "no", keys::GENAI: "no", keys::EMOTION_BIOMETRIC: "no" });
        let c = classify(&answers, &Signals::default());
        assert_eq!(c.risk_category, RiskCategory::Minimal);
        assert!(c.conformity_score.is_some());
    }

    #[test]
    fn recommends_transparency_ack_for_chatbot() {
        let answers = json!({ keys::CHATBOT: "yes" });
        let signals = Signals::default();
        let c = classify(&answers, &signals);
        let recs = recommendations(&answers, &signals, &c);
        assert!(recs.iter().any(|r| r.id == "ack-transparency"));
    }

    #[test]
    fn recommends_unblock_for_prohibited() {
        let answers = json!({ keys::PROHIBITED: ["social_scoring"] });
        let signals = Signals::default();
        let c = classify(&answers, &signals);
        let recs = recommendations(&answers, &signals, &c);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].id, "remove-prohibited");
    }

    #[test]
    fn recommendations_sorted_by_uplift() {
        let answers = json!({
            keys::CONSEQUENTIAL: ["employment"],
            keys::HUMAN_OVERSIGHT: "no",
            keys::PERSONAL_DATA: "yes",
            keys::INSTRUCTIONS: "no",
        });
        let signals = Signals::default();
        let c = classify(&answers, &signals);
        let recs = recommendations(&answers, &signals, &c);
        assert!(recs.len() >= 2);
        for pair in recs.windows(2) {
            assert!(pair[0].potential_points >= pair[1].potential_points);
        }
    }
}
