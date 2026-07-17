use std::env;

pub struct Config {
    pub port: u16,
    pub database_url: Option<String>,
    pub aggregation_base_url: String,
    pub session_idle_minutes: i64,
    pub session_absolute_hours: i64,
}

impl Config {
    pub fn from_env() -> Self {
        let port = env::var("VERASENS_DASHBOARD_PORT")
            .or_else(|_| env::var("PORT"))
            .unwrap_or_else(|_| "7043".to_string())
            .parse()
            .unwrap_or(7043);

        let database_url = env::var("DATABASE_URL").ok();

        let aggregation_base_url =
            env::var("VERASENS_AGG_URL").unwrap_or_else(|_| "http://localhost:7044".to_string());

        let session_idle_minutes = env::var("VERASENS_SESSION_IDLE_MINUTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);

        let session_absolute_hours = env::var("VERASENS_SESSION_ABSOLUTE_HOURS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(12);

        Self {
            port,
            database_url,
            aggregation_base_url,
            session_idle_minutes,
            session_absolute_hours,
        }
    }
}
