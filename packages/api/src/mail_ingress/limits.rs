//! Daily counters for outbound recipients and alias edits.
use super::{digest, statement};
use crate::{error::ApiError, state::AppState};
use sea_orm::ConnectionTrait;

const DAY_MS: i64 = 86_400_000;

/// The current UTC day; counters reset at its end.
pub(crate) struct Window {
    day: i64,
    resets_at: i64,
}

impl Window {
    pub(crate) fn at(now: i64) -> Self {
        let day = now.div_euclid(DAY_MS);
        Self {
            day,
            resets_at: (day + 1) * DAY_MS,
        }
    }

    pub(crate) fn retry_after_secs(&self, now: i64) -> u64 {
        ((self.resets_at - now).max(1_000) as u64).div_ceil(1_000)
    }

    fn id(&self, counter: &Counter<'_>) -> String {
        digest(&[counter.scope, counter.subject, &self.day.to_string()])
    }
}

pub(crate) struct Counter<'a> {
    pub(crate) scope: &'static str,
    pub(crate) subject: &'a str,
    pub(crate) limit: u32,
}

async fn increment(
    state: &AppState,
    id: String,
    amount: i64,
    limit: i64,
    expires_at: i64,
) -> Result<bool, ApiError> {
    state
        .transaction(move |tx| {
            let id = id.clone();
            Box::pin(async move {
                let result = tx
                    .execute_raw(statement(
                        r#"INSERT INTO "MailAutomationQuota" ("id","count","expiresAt") VALUES ($1,$2,$3) ON CONFLICT ("id") DO UPDATE SET "count"="MailAutomationQuota"."count"+$2 WHERE "MailAutomationQuota"."count"+$2 <= $4"#,
                        vec![id.into(), amount.into(), expires_at.into(), limit.into()],
                    ))
                    .await?;
                Ok::<_, ApiError>(result.rows_affected() == 1)
            })
        })
        .await
}

/// Adds `amount` to every counter, or to none of them. Returns the first exhausted counter.
pub(crate) async fn consume(
    state: &AppState,
    window: &Window,
    counters: &[Counter<'_>],
    amount: u32,
) -> Result<Option<usize>, ApiError> {
    for (index, counter) in counters.iter().enumerate() {
        let allowed = if amount > counter.limit {
            Ok(false)
        } else {
            increment(
                state,
                window.id(counter),
                i64::from(amount),
                i64::from(counter.limit),
                window.resets_at + DAY_MS,
            )
            .await
        };
        match allowed {
            Ok(true) => {}
            Ok(false) => {
                refund(state, window, &counters[..index], amount).await;
                return Ok(Some(index));
            }
            Err(error) => {
                refund(state, window, &counters[..index], amount).await;
                return Err(error);
            }
        }
    }
    Ok(None)
}

/// Best effort: a failed refund leaves the counter conservative, never permissive.
pub(crate) async fn refund(
    state: &AppState,
    window: &Window,
    counters: &[Counter<'_>],
    amount: u32,
) {
    for counter in counters {
        if let Err(error) = state
            .db
            .execute_raw(statement(
                r#"UPDATE "MailAutomationQuota" SET "count"="count"-$1 WHERE "id"=$2 AND "count">=$1"#,
                vec![i64::from(amount).into(), window.id(counter).into()],
            ))
            .await
        {
            tracing::warn!(scope = counter.scope, %error, "Mail limit refund failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_reset_at_utc_midnight_and_key_each_scope_separately() {
        let noon = 20_000 * DAY_MS + DAY_MS / 2;
        let window = Window::at(noon);
        assert_eq!(window.retry_after_secs(noon), 43_200);
        assert_eq!(window.retry_after_secs(window.resets_at), 1);
        assert_eq!(Window::at(window.resets_at - 1).day, window.day);
        assert_eq!(Window::at(window.resets_at).day, window.day + 1);
        let counter = |scope, subject| Counter {
            scope,
            subject,
            limit: 1,
        };
        assert_ne!(
            window.id(&counter("app", "a")),
            window.id(&counter("principal", "a"))
        );
        assert_ne!(
            window.id(&counter("app", "a")),
            Window::at(window.resets_at).id(&counter("app", "a"))
        );
    }
}
