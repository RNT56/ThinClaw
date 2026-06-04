use axum::{extract::Path, http::header, response::IntoResponse};
use thinclaw_gateway::web::projects::{
    project_forbidden_error, project_not_found_error, validate_project_id,
};

pub(crate) async fn project_redirect_handler(Path(project_id): Path<String>) -> impl IntoResponse {
    axum::response::Redirect::permanent(&format!("/projects/{project_id}/"))
}

pub(crate) async fn project_index_handler(Path(project_id): Path<String>) -> impl IntoResponse {
    serve_project_file(&project_id, "index.html").await
}

pub(crate) async fn project_file_handler(
    Path((project_id, path)): Path<(String, String)>,
) -> impl IntoResponse {
    serve_project_file(&project_id, &path).await
}

pub(crate) async fn serve_project_file(project_id: &str, path: &str) -> axum::response::Response {
    if let Err(error) = validate_project_id(project_id) {
        return error.into_response();
    }

    let base = crate::platform::resolve_data_dir("projects").join(project_id);

    let file_path = base.join(path);

    let canonical = match file_path.canonicalize() {
        Ok(p) => p,
        Err(_) => return project_not_found_error().into_response(),
    };
    let base_canonical = match base.canonicalize() {
        Ok(p) => p,
        Err(_) => return project_not_found_error().into_response(),
    };
    if !canonical.starts_with(&base_canonical) {
        return project_forbidden_error().into_response();
    }

    match tokio::fs::read(&canonical).await {
        Ok(contents) => {
            let mime = mime_guess::from_path(&canonical)
                .first_or_octet_stream()
                .to_string();
            ([(header::CONTENT_TYPE, mime)], contents).into_response()
        }
        Err(_) => project_not_found_error().into_response(),
    }
}
