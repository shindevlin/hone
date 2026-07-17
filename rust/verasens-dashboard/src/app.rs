use std::sync::Arc;

use crate::agg_client::AggregationClient;
use crate::config::Config;
use crate::store::Store;
use crate::stripe_client::StripeClient;

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn Store>,
    pub stripe: Arc<dyn StripeClient>,
    pub agg: Arc<dyn AggregationClient>,
    pub cfg: Arc<Config>,
}

impl AppState {
    pub fn new(
        store: Arc<dyn Store>,
        stripe: Arc<dyn StripeClient>,
        agg: Arc<dyn AggregationClient>,
        cfg: Config,
    ) -> Self {
        Self { store, stripe, agg, cfg: Arc::new(cfg) }
    }
}
