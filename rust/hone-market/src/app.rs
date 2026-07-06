use crate::{config::Config, state::SharedState};
use std::sync::Arc;

/// Cloneable application state passed to all Axum handlers.
#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub state: SharedState,
    pub jwt_secret: String,
}

impl AppState {
    pub fn new(cfg: Config, state: SharedState) -> Self {
        let jwt_secret = cfg.jwt_secret.clone();
        AppState { cfg: Arc::new(cfg), state, jwt_secret }
    }
}
