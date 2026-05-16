use crate::state::AppState;
use axum::{
    Router,
    routing::{get, post, put},
};

pub mod access;
pub mod app_links;
pub mod app_refs;
pub mod asset_references;
pub mod assets;
pub mod attempts;
pub mod certificates;
pub mod challenges;
pub mod courses;
pub mod enrollment;
pub mod leaderboard;
pub mod lessons;
pub mod modules;
pub mod paths;
pub mod progress;
pub mod shared_app;
pub mod translate;
pub mod weekly;

pub fn routes() -> Router<AppState> {
    Router::new()
        // Courses
        .route("/", get(courses::list_courses))
        .route(
            "/{course_id}",
            get(courses::get_course)
                .put(courses::upsert_course)
                .delete(courses::delete_course),
        )
        .route("/{course_id}/structure", get(courses::get_course_structure))
        .route("/{course_id}/meta/media", put(courses::push_course_media))
        // Course assets (referenced via @AssetName in lesson content)
        .route(
            "/{course_id}/assets",
            get(assets::list_course_assets).post(assets::create_course_asset),
        )
        .route(
            "/{course_id}/assets/{asset_id}",
            put(assets::update_course_asset).delete(assets::delete_course_asset),
        )
        .route(
            "/{course_id}/assets/{asset_id}/optimize",
            post(assets::optimize_course_asset),
        )
        // Modules
        .route(
            "/{course_id}/modules/{module_id}",
            put(modules::upsert_module).delete(modules::delete_module),
        )
        // Lessons
        .route(
            "/{course_id}/modules/{module_id}/lessons/{lesson_id}",
            get(lessons::get_lesson)
                .put(lessons::upsert_lesson)
                .delete(lessons::delete_lesson),
        )
        // Challenges
        .route(
            "/{course_id}/lessons/{lesson_id}/challenges/{challenge_id}",
            put(challenges::upsert_challenge).delete(challenges::delete_challenge),
        )
        // App links (course-level)
        .route("/{course_id}/app-links", get(app_links::list_app_links))
        .route(
            "/{course_id}/app-links/{link_id}",
            put(app_links::upsert_app_link).delete(app_links::delete_app_link),
        )
        // App refs (lesson-level)
        .route(
            "/{course_id}/lessons/{lesson_id}/refs/{ref_id}",
            put(app_refs::upsert_app_ref).delete(app_refs::delete_app_ref),
        )
        // Enrollment
        .route(
            "/{course_id}/links/{alias}/open",
            post(shared_app::open_shared_app),
        )
        .route("/{course_id}/translate", get(translate::translate))
        .route("/{course_id}/enroll", post(enrollment::enroll))
        .route("/enrollments/me", get(enrollment::get_my_enrollments))
        // Progress
        .route(
            "/{course_id}/progress/me",
            get(progress::get_my_course_progress),
        )
        .route(
            "/lessons/{lesson_id}/complete",
            post(progress::mark_lesson_complete),
        )
        // Challenge attempts
        .route(
            "/challenges/{challenge_id}/attempt",
            post(attempts::submit_attempt),
        )
        // Certificates
        .route(
            "/{course_id}/certificate",
            post(certificates::issue_certificate),
        )
        .route("/certificates/me", get(certificates::list_my_certificates))
        .route(
            "/certificates/verify/{cert_id}",
            get(certificates::verify_certificate),
        )
        // Learning paths
        .route("/paths", get(paths::list_learning_paths))
        .route(
            "/paths/{path_id}",
            get(paths::get_learning_path)
                .put(paths::upsert_learning_path)
                .delete(paths::delete_learning_path),
        )
        .route(
            "/paths/{path_id}/courses/{course_id}",
            put(paths::upsert_learning_path_step).delete(paths::delete_learning_path_step),
        )
        // Leaderboard
        .route("/leaderboard", get(leaderboard::get_leaderboard))
        .route(
            "/leaderboard/me",
            get(leaderboard::get_my_opt_in).put(leaderboard::update_my_opt_in),
        )
        // Weekly challenge
        .route("/weekly", get(weekly::get_current_weekly))
        .route("/weekly/rotate", post(weekly::rotate_weekly))
}
