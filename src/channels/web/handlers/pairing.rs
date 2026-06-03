use axum::{Json, extract::Path, http::StatusCode};

use crate::channels::web::types::*;
use thinclaw_gateway::web::pairing::{
    PairingRequestInfoInput, pairing_approve_response, pairing_approved_info,
    pairing_error_response, pairing_invalid_code_response, pairing_list_response,
    pairing_request_info,
};

pub(crate) async fn pairing_list_handler(
    Path(channel): Path<String>,
) -> Result<Json<PairingListResponse>, (StatusCode, String)> {
    let store = crate::pairing::PairingStore::new();
    let requests = store
        .list_pending(&channel)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let infos = requests
        .into_iter()
        .map(|r| {
            pairing_request_info(PairingRequestInfoInput {
                code: r.code,
                sender_id: r.id,
                meta: r.meta,
                created_at: r.created_at,
            })
        })
        .collect();
    let approved = store
        .read_allow_from(&channel)
        .unwrap_or_default()
        .into_iter()
        .map(pairing_approved_info)
        .collect();

    Ok(Json(pairing_list_response(channel, infos, approved)))
}

pub(crate) async fn pairing_approve_handler(
    Path(channel): Path<String>,
    Json(req): Json<PairingApproveRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let store = crate::pairing::PairingStore::new();
    match store.approve(&channel, &req.code) {
        Ok(Some(approved)) => Ok(Json(pairing_approve_response(approved.id))),
        Ok(None) => Ok(Json(pairing_invalid_code_response())),
        Err(crate::pairing::PairingStoreError::ApproveRateLimited) => Err((
            StatusCode::TOO_MANY_REQUESTS,
            "Too many failed approve attempts; try again later".to_string(),
        )),
        Err(e) => Ok(Json(pairing_error_response(e.to_string()))),
    }
}
