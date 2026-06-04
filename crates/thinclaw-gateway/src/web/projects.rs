//! Root-independent project file serving policies.

use axum::http::StatusCode;

pub const INVALID_PROJECT_ID_MESSAGE: &str = "Invalid project ID";
pub const PROJECT_NOT_FOUND_MESSAGE: &str = "Not found";
pub const PROJECT_FORBIDDEN_MESSAGE: &str = "Forbidden";

pub fn validate_project_id(project_id: &str) -> Result<(), (StatusCode, &'static str)> {
    if project_id.contains('/')
        || project_id.contains('\\')
        || project_id.contains("..")
        || project_id.is_empty()
    {
        return Err(project_invalid_id_error());
    }
    Ok(())
}

pub fn project_invalid_id_error() -> (StatusCode, &'static str) {
    (StatusCode::BAD_REQUEST, INVALID_PROJECT_ID_MESSAGE)
}

pub fn project_not_found_error() -> (StatusCode, &'static str) {
    (StatusCode::NOT_FOUND, PROJECT_NOT_FOUND_MESSAGE)
}

pub fn project_forbidden_error() -> (StatusCode, &'static str) {
    (StatusCode::FORBIDDEN, PROJECT_FORBIDDEN_MESSAGE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_id_validation_rejects_path_traversal_and_empty_ids() {
        assert_eq!(validate_project_id("project-a"), Ok(()));

        for invalid in ["", "../secret", "a/b", r"a\b", "project..bak"] {
            assert_eq!(
                validate_project_id(invalid),
                Err(project_invalid_id_error())
            );
        }
    }

    #[test]
    fn project_file_errors_preserve_existing_statuses_and_messages() {
        assert_eq!(
            project_invalid_id_error(),
            (StatusCode::BAD_REQUEST, INVALID_PROJECT_ID_MESSAGE)
        );
        assert_eq!(
            project_not_found_error(),
            (StatusCode::NOT_FOUND, PROJECT_NOT_FOUND_MESSAGE)
        );
        assert_eq!(
            project_forbidden_error(),
            (StatusCode::FORBIDDEN, PROJECT_FORBIDDEN_MESSAGE)
        );
    }
}
