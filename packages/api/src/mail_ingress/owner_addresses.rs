//! The mail addresses that count as a bot's owner: their account address, always, and the ones
//! they stated to the bot, which the bot's memory holds as profile facts about the user.

use crate::{
    entity::{bot, user},
    error::ApiError,
    routes::bots::memory::admin_for,
    state::AppState,
};
use flow_like_bot_memory::{
    facts::{Fact, FactKind},
    model::FactSource,
};
use flow_like_catalog_core::{InboundEmail, InboundEmailAuthentication};
use sea_orm::{ConnectionTrait, EntityTrait};

pub(crate) async fn account_address<C: ConnectionTrait>(
    db: &C,
    row: &bot::Model,
) -> Result<Option<String>, ApiError> {
    Ok(user::Entity::find_by_id(&row.owner_id)
        .one(db)
        .await?
        .and_then(|owner| owner.email)
        .map(|email| email.trim().to_ascii_lowercase())
        .filter(|email| !email.is_empty()))
}

fn subject_is_owner_mail(subject: &str) -> bool {
    matches!(subject, "user" | "user.email" | "user.mail")
}

/// Addresses in the active facts the owner stated about themselves. A profile fact can only be
/// kept with the owner's own message as evidence, so a guest's or a mail's claim never lands here.
pub(crate) fn addresses_in(facts: &[Fact]) -> Vec<String> {
    facts
        .iter()
        .filter(|fact| {
            fact.is_active()
                && fact.kind == FactKind::Profile
                && fact.source == FactSource::UserStated
                && subject_is_owner_mail(fact.subject.trim())
                && fact.text.to_ascii_lowercase().contains("mail")
        })
        .flat_map(|fact| {
            fact.text
                .split_whitespace()
                .map(|token| {
                    token
                        .trim_matches(|c: char| !c.is_alphanumeric() && c != '@')
                        .to_ascii_lowercase()
                })
                .filter(|token| token.matches('@').count() == 1 && token.contains('.'))
                .collect::<Vec<_>>()
        })
        .collect()
}

async fn stated_addresses(state: &AppState, row: &bot::Model) -> Vec<String> {
    let admin = match admin_for(state, &row.owner_id, &row.id).await {
        Ok(Some(admin)) => admin,
        Ok(None) => return Vec::new(),
        Err(error) => {
            tracing::warn!(bot_id = %row.id, %error, "Owner mail addresses: the bot's memory could not be opened");
            return Vec::new();
        }
    };
    match admin.list(false).await {
        Ok(facts) => addresses_in(&facts),
        Err(error) => {
            tracing::warn!(bot_id = %row.id, %error, "Owner mail addresses: the bot's facts could not be read");
            Vec::new()
        }
    }
}

pub(crate) fn dmarc_passed(authentication: Option<&InboundEmailAuthentication>) -> bool {
    authentication
        .and_then(|auth| auth.dmarc.as_ref())
        .is_some_and(|verdict| verdict.status.eq_ignore_ascii_case("PASS"))
}

pub(crate) fn from_addresses(email: &InboundEmail) -> Option<Vec<String>> {
    let from: Vec<String> = email
        .from
        .iter()
        .filter_map(|sender| sender.email.as_deref())
        .map(|address| address.trim().to_ascii_lowercase())
        .collect();
    (!from.is_empty() && from.len() == email.from.len()).then_some(from)
}

/// Mail from the owner: every From address is theirs and DMARC passed. DMARC passes only when
/// SPF or DKIM authenticated the From domain itself, so the verdict already covers alignment.
/// The bot's memory is read only for a mail that passed DMARC from an address the account
/// doesn't explain.
pub(crate) async fn sent_by_owner(
    state: &AppState,
    row: &bot::Model,
    email: &InboundEmail,
) -> bool {
    if !dmarc_passed(email.authentication.as_ref()) {
        return false;
    }
    let Some(from) = from_addresses(email) else {
        return false;
    };
    let account = account_address(&state.db, row).await.ok().flatten();
    if from.iter().all(|address| Some(address) == account.as_ref()) {
        return true;
    }
    let stated = stated_addresses(state, row).await;
    from.iter()
        .all(|address| Some(address) == account.as_ref() || stated.contains(address))
}
