//! Bot tools that run headless on every host through one entry, [`run`]. A host implements
//! [`ToolEnv`]; core registers the [`ToolSpec`]s from [`specs`]. Modules today: `files` (one file
//! reference for every source) and `knowledge` (on-demand RAG indexes in the workspace).
//! Contract: `todo/bots-capabilities/lanes/BOT-TOOLS.md`.

pub mod browser;
pub mod code;
pub mod computer;
pub mod configuration;
pub mod convert;
#[cfg(feature = "data")]
pub mod data;
pub mod data_backend;
#[cfg(feature = "connectors")]
mod data_catalog;
pub mod documents;
pub mod email;
pub mod embed;
pub mod env;
pub mod error;
pub mod files;
pub mod image;
mod interaction_image;
pub mod knowledge;
pub mod layout;
pub mod notify;
pub mod outcome;
pub mod people;
pub mod schedule;
pub mod secret_text;
pub mod secrets;
pub mod services;
pub mod shell;
pub mod skills;
pub mod spec;
pub mod tables;
pub mod user_context;
pub mod web;

#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;

use std::time::Instant;

use serde::de::DeserializeOwned;
use serde_json::Value;

pub use browser::{BrowserBackend, BrowserStep, PageState};
pub use code::{LanguageSupport, RunRequest, RunResponse, RunStatus, SandboxBackend};
pub use computer::{ComputerAction, ComputerBackend, Screen};
pub use convert::{ConvertInput, Converted, DocumentConverter};
pub use data_backend::{DataBackend, NetworkTable, SourceTable};
pub use documents::DocumentBackend;
pub use embed::{KnowledgeEmbedder, ModelEmbedder};
pub use env::{
    Access, AppGrant, AppTables, Attachment, Environment, FileData, FileEntry, FileMeta, Limits,
    MemorySessions, ModelRole, Progress, Scope, SessionCache, ToolEnv,
};
pub use error::{ToolError, ToolResult};
pub use files::FileRef;
pub use notify::{NoticeReceipt, OwnerNotice, OwnerNotifier, TurnSite};
pub use outcome::{Status, ToolImage, ToolOutcome, ToolOutput, Usage};
pub use people::{Contact, Delivery, PeopleBackend, PersonMessage};
pub use schedule::Scheduler;
pub use secrets::SecretInfo;
pub use shell::{LocalShell, ShellBackend, ShellRequest, ShellResponse};
pub use skills::{SkillHost, SkillToolSpec};
pub use spec::{Approval, ToolSpec, spec, specs};
pub use user_context::UserContextBackend;

fn parse_args<T: DeserializeOwned>(tool: &str, args: Value) -> ToolResult<T> {
    let args = if args.is_null() {
        Value::Object(Default::default())
    } else {
        args
    };
    serde_json::from_value(args)
        .map_err(|error| ToolError::invalid(format!("Bad arguments for {tool}: {error}")))
}

async fn dispatch(tool: &str, args: Value, env: &dyn ToolEnv) -> ToolResult<ToolOutput> {
    if env.is_cancelled() {
        return Err(ToolError::new(
            error::code::CANCELLED,
            "The call was cancelled",
        ));
    }
    match tool {
        configuration::BOT_CONFIGURATION => {
            configuration::inspect(env, parse_args(tool, args)?).await
        }
        configuration::BOT_MODELS => configuration::models(env, parse_args(tool, args)?).await,
        configuration::BOT_UPDATE_CONFIGURATION => {
            configuration::update(env, parse_args(tool, args)?).await
        }
        files::FILE_LIST => files::file_list(env, parse_args(tool, args)?).await,
        files::FILE_READ => files::file_read(env, parse_args(tool, args)?).await,
        files::FILE_SAVE => files::file_save(env, parse_args(tool, args)?).await,
        files::FILE_WRITE => files::file_write(env, parse_args(tool, args)?).await,
        knowledge::KNOWLEDGE_CREATE => {
            knowledge::knowledge_create(env, parse_args(tool, args)?).await
        }
        knowledge::KNOWLEDGE_ADD => knowledge::knowledge_add(env, parse_args(tool, args)?).await,
        knowledge::KNOWLEDGE_SEARCH => {
            knowledge::knowledge_search(env, parse_args(tool, args)?).await
        }
        knowledge::KNOWLEDGE_LIST => knowledge::knowledge_list(env, parse_args(tool, args)?).await,
        knowledge::KNOWLEDGE_DROP => knowledge::knowledge_drop(env, parse_args(tool, args)?).await,
        documents::FILE_SUMMARIZE => documents::file_summarize(env, parse_args(tool, args)?).await,
        documents::FILE_EXTRACT => documents::file_extract(env, parse_args(tool, args)?).await,
        code::CODE_RUN => code::code_run(env, parse_args(tool, args)?).await,
        image::IMAGE_GENERATE => image::image_generate(env, parse_args(tool, args)?).await,
        notify::MESSAGE_OWNER => notify::message_owner(env, parse_args(tool, args)?).await,
        people::LIST_PEOPLE => people::list_people(env, parse_args(tool, args)?).await,
        people::MESSAGE_PERSON => people::message_person(env, parse_args(tool, args)?).await,
        user_context::USER_CONTEXT_AVAILABLE => {
            user_context::user_context_available(env, parse_args(tool, args)?).await
        }
        user_context::USER_CONTEXT_REQUEST => {
            user_context::user_context_request(env, parse_args(tool, args)?).await
        }
        user_context::USER_CONTEXT_SET_STATUS => {
            user_context::user_context_set_status(env, parse_args(tool, args)?).await
        }
        browser::BROWSER_OPEN => browser::browser_open(env, parse_args(tool, args)?).await,
        browser::BROWSER_SNAPSHOT => browser::browser_snapshot(env, parse_args(tool, args)?).await,
        browser::BROWSER_ACT => browser::browser_act(env, parse_args(tool, args)?).await,
        browser::BROWSER_SCREENSHOT => {
            browser::browser_screenshot(env, parse_args(tool, args)?).await
        }
        browser::BROWSER_CLOSE => browser::browser_close(env, parse_args(tool, args)?).await,
        computer::COMPUTER_SCREENSHOT => {
            computer::computer_screenshot(env, parse_args(tool, args)?).await
        }
        computer::COMPUTER_ACT => computer::computer_act(env, parse_args(tool, args)?).await,
        computer::COMPUTER_WAIT => computer::computer_wait(env, parse_args(tool, args)?).await,
        computer::COMPUTER_WINDOWS => {
            computer::computer_windows(env, parse_args(tool, args)?).await
        }
        computer::COMPUTER_WINDOW => computer::computer_window(env, parse_args(tool, args)?).await,
        computer::COMPUTER_LAUNCH => computer::computer_launch(env, parse_args(tool, args)?).await,
        shell::SHELL_RUN => shell::shell_run(env, parse_args(tool, args)?).await,
        secrets::SECRET_LIST => secrets::secret_list(env, parse_args(tool, args)?).await,
        secrets::SECRET_REQUEST => secrets::secret_request(env, parse_args(tool, args)?).await,
        documents::EXCEL_INSPECT => documents::excel_inspect(env, parse_args(tool, args)?).await,
        #[cfg(feature = "data")]
        data::DATA_SOURCES => data::data_sources(env, parse_args(tool, args)?).await,
        #[cfg(feature = "data")]
        data::DATA_QUERY => data::data_query(env, parse_args(tool, args)?).await,
        schedule::SCHEDULE_TASK => schedule::schedule_task(env, parse_args(tool, args)?).await,
        schedule::LIST_TASKS => schedule::list_tasks(env, parse_args(tool, args)?).await,
        schedule::CANCEL_TASK => schedule::cancel_task(env, parse_args(tool, args)?).await,
        skills::SKILL_LOAD => skills::skill_load(env, parse_args(tool, args)?).await,
        skills::SKILL_PROPOSE => skills::skill_propose(env, parse_args(tool, args)?).await,
        skills::SKILL_CALL => skills::skill_call(env, parse_args(tool, args)?).await,
        skills::SKILL_RUN => skills::skill_run(env, parse_args(tool, args)?).await,
        skills::SKILL_IMPORT => skills::import::run_import(env, parse_args(tool, args)?).await,
        email::EMAIL_SEND => email::email_send(env, parse_args(tool, args)?).await,
        email::EMAIL_REPLY => email::email_reply(env, parse_args(tool, args)?).await,
        documents::PDF_RENDER => documents::pdf_render(env, parse_args(tool, args)?).await,
        documents::DOCUMENT_CREATE => {
            documents::document_create(env, parse_args(tool, args)?).await
        }
        documents::DOCUMENT_FILL => documents::document_fill(env, parse_args(tool, args)?).await,
        services::CONNECTOR_TOOLS => services::connector_tools(env, parse_args(tool, args)?).await,
        services::CONNECTOR_CALL => services::connector_call(env, parse_args(tool, args)?).await,
        services::CONNECTOR_REFRESH => {
            services::connector_refresh(env, parse_args(tool, args)?).await
        }
        services::CONNECTOR_REQUEST => {
            services::connector_request(env, parse_args(tool, args)?).await
        }
        web::WEB_REQUEST => web::web_request(env, parse_args(tool, args)?).await,
        other => Err(ToolError::new(
            error::code::UNKNOWN_TOOL,
            format!("This crate has no tool '{other}'"),
        )),
    }
}

/// Runs one tool against the host's environment. Never panics on bad input: every failure is a
/// [`Status::Error`] outcome, and the result is capped to the environment's inline limit.
pub async fn run(tool: &str, args: Value, env: &dyn ToolEnv) -> ToolOutcome {
    let started = Instant::now();
    let cap = env.limits().inline_cap_bytes;
    match dispatch(tool, args, env).await {
        Ok(mut output) => {
            output.usage.cpu_ms = started.elapsed().as_millis() as u64;
            ToolOutcome::from_output(tool, output, cap)
        }
        Err(error) => {
            let mut outcome = ToolOutcome::from_error(tool, error);
            outcome.usage.cpu_ms = started.elapsed().as_millis() as u64;
            outcome
        }
    }
}
