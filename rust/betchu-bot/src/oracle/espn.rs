use anyhow::Result;
use chrono::{DateTime, Datelike, Timelike, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, warn};

const ESPN_CFB_URL: &str =
    "https://site.api.espn.com/apis/site/v2/sports/football/college-football/scoreboard";
const ESPN_CBB_URL: &str =
    "https://site.api.espn.com/apis/site/v2/sports/basketball/mens-college-basketball/scoreboard";
const ESPN_NFL_URL: &str =
    "https://site.api.espn.com/apis/site/v2/sports/football/nfl/scoreboard";
const ESPN_NBA_URL: &str =
    "https://site.api.espn.com/apis/site/v2/sports/basketball/nba/scoreboard";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EspnGame {
    pub id: String,
    pub name: String,
    pub short_name: String,
    pub home_team: String,
    pub home_abbrev: String,
    pub away_team: String,
    pub away_abbrev: String,
    pub home_score: Option<u32>,
    pub away_score: Option<u32>,
    pub status: String,
    pub is_final: bool,
    pub sport: String,
    pub date: String,
    pub start_epoch: u64,
    pub venue: String,
}

impl EspnGame {
    pub fn sport_abbrev(&self) -> &str {
        match self.sport.as_str() {
            "football_college" => "CFB",
            "football_nfl" => "NFL",
            "basketball_college" => "CBB",
            "basketball_nba" => "NBA",
            _ => &self.sport,
        }
    }

    /// Canonical description stored on-chain: "Kansas State Wildcats at Ohio State Buckeyes (CFB) — Sat, Nov 9, 7:30 PM UTC"
    pub fn normalized_description(&self) -> String {
        format!(
            "{} at {} ({}) — {}",
            self.away_team,
            self.home_team,
            self.sport_abbrev(),
            self.date,
        )
    }

    pub fn is_upcoming(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.start_epoch > now
    }
}

#[derive(Debug, Deserialize)]
struct EspnScoreboard {
    events: Option<Vec<EspnEvent>>,
}

#[derive(Debug, Deserialize)]
struct EspnEvent {
    id: String,
    date: Option<String>,
    name: Option<String>,
    #[serde(rename = "shortName")]
    short_name: Option<String>,
    competitions: Option<Vec<EspnCompetition>>,
    status: Option<EspnStatus>,
}

#[derive(Debug, Deserialize)]
struct EspnCompetition {
    competitors: Option<Vec<EspnCompetitor>>,
    venue: Option<EspnVenue>,
}

#[derive(Debug, Deserialize)]
struct EspnVenue {
    #[serde(rename = "fullName")]
    full_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EspnCompetitor {
    #[serde(rename = "homeAway")]
    home_away: Option<String>,
    team: Option<EspnTeam>,
    score: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EspnTeam {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    abbreviation: Option<String>,
    #[serde(rename = "shortDisplayName")]
    short_display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EspnStatus {
    r#type: Option<EspnStatusType>,
}

#[derive(Debug, Deserialize)]
struct EspnStatusType {
    completed: Option<bool>,
    description: Option<String>,
    name: Option<String>,
}

pub struct EspnScraper {
    client: Client,
}

impl EspnScraper {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .user_agent("Mozilla/5.0 (compatible; betchu-bot/1.0)")
                .build()
                .expect("Failed to build ESPN client"),
        }
    }

    pub async fn get_games(&self) -> Result<Vec<EspnGame>> {
        let mut all_games = Vec::new();

        for (url, sport) in [
            (ESPN_CFB_URL, "football_college"),
            (ESPN_CBB_URL, "basketball_college"),
            (ESPN_NFL_URL, "football_nfl"),
            (ESPN_NBA_URL, "basketball_nba"),
        ] {
            match self.fetch_sport(url, sport).await {
                Ok(games) => all_games.extend(games),
                Err(e) => warn!("ESPN {} fetch failed: {}", sport, e),
            }
        }

        // Sort by start time ascending
        all_games.sort_by_key(|g| g.start_epoch);
        Ok(all_games)
    }

    async fn fetch_sport(&self, url: &str, sport: &str) -> Result<Vec<EspnGame>> {
        let resp = self
            .client
            .get(url)
            .query(&[("limit", "100")])
            .send()
            .await?;

        if !resp.status().is_success() {
            return Ok(vec![]);
        }

        let board: EspnScoreboard = resp.json().await?;
        let events = board.events.unwrap_or_default();

        let games: Vec<EspnGame> = events
            .into_iter()
            .filter_map(|e| parse_event(e, sport))
            .collect();

        debug!("ESPN {} returned {} games", sport, games.len());
        Ok(games)
    }

    pub async fn get_finished_games(&self) -> Result<Vec<EspnGame>> {
        let games = self.get_games().await?;
        Ok(games.into_iter().filter(|g| g.is_final).collect())
    }

    pub async fn get_upcoming_games(&self) -> Result<Vec<EspnGame>> {
        let games = self.get_games().await?;
        Ok(games.into_iter().filter(|g| !g.is_final).collect())
    }
}

fn parse_event(event: EspnEvent, sport: &str) -> Option<EspnGame> {
    let name = event
        .name
        .clone()
        .unwrap_or_else(|| event.id.clone());

    let short_name = event
        .short_name
        .clone()
        .unwrap_or_else(|| name.clone());

    let is_final = event
        .status
        .as_ref()
        .and_then(|s| s.r#type.as_ref())
        .and_then(|t| t.completed)
        .unwrap_or(false);

    let status_desc = event
        .status
        .as_ref()
        .and_then(|s| s.r#type.as_ref())
        .and_then(|t| t.description.as_deref().or(t.name.as_deref()))
        .unwrap_or("Scheduled")
        .to_string();

    let (date_str, start_epoch) = event
        .date
        .as_deref()
        .map(format_game_date)
        .unwrap_or_else(|| ("TBD".to_string(), 0));

    let competition = event
        .competitions
        .and_then(|mut c| if c.is_empty() { None } else { Some(c.remove(0)) });

    let venue = competition
        .as_ref()
        .and_then(|c| c.venue.as_ref())
        .and_then(|v| v.full_name.as_deref())
        .unwrap_or("")
        .to_string();

    let competitors = competition
        .map(|c| c.competitors.unwrap_or_default())
        .unwrap_or_default();

    let mut home_team = String::from("Home");
    let mut home_abbrev = String::new();
    let mut away_team = String::from("Away");
    let mut away_abbrev = String::new();
    let mut home_score: Option<u32> = None;
    let mut away_score: Option<u32> = None;

    for comp in competitors {
        let display = comp
            .team
            .as_ref()
            .and_then(|t| t.display_name.as_deref())
            .unwrap_or("Unknown")
            .to_string();
        let abbrev = comp
            .team
            .as_ref()
            .and_then(|t| t.abbreviation.as_deref())
            .unwrap_or("")
            .to_string();
        let score: Option<u32> = comp.score.as_deref().and_then(|s| s.parse().ok());

        match comp.home_away.as_deref() {
            Some("home") => {
                home_team = display;
                home_abbrev = abbrev;
                home_score = score;
            }
            Some("away") => {
                away_team = display;
                away_abbrev = abbrev;
                away_score = score;
            }
            _ => {}
        }
    }

    Some(EspnGame {
        id: event.id,
        name,
        short_name,
        home_team,
        home_abbrev,
        away_team,
        away_abbrev,
        home_score,
        away_score,
        status: status_desc,
        is_final,
        sport: sport.to_string(),
        date: date_str,
        start_epoch,
        venue,
    })
}

fn format_game_date(date_str: &str) -> (String, u64) {
    match DateTime::parse_from_rfc3339(date_str) {
        Ok(dt) => {
            let utc: DateTime<Utc> = dt.with_timezone(&Utc);
            let epoch = utc.timestamp() as u64;

            let month = match utc.month() {
                1 => "Jan", 2 => "Feb", 3 => "Mar", 4 => "Apr",
                5 => "May", 6 => "Jun", 7 => "Jul", 8 => "Aug",
                9 => "Sep", 10 => "Oct", 11 => "Nov", 12 => "Dec",
                _ => "?",
            };
            let day_name = match utc.weekday() {
                chrono::Weekday::Mon => "Mon",
                chrono::Weekday::Tue => "Tue",
                chrono::Weekday::Wed => "Wed",
                chrono::Weekday::Thu => "Thu",
                chrono::Weekday::Fri => "Fri",
                chrono::Weekday::Sat => "Sat",
                chrono::Weekday::Sun => "Sun",
            };

            let hour = utc.hour();
            let minute = utc.minute();
            let (am_pm, h12) = if hour == 0 {
                ("AM", 12u32)
            } else if hour < 12 {
                ("AM", hour)
            } else if hour == 12 {
                ("PM", 12)
            } else {
                ("PM", hour - 12)
            };

            let formatted = if minute == 0 {
                format!("{}, {} {}, {} {}", day_name, month, utc.day(), h12, am_pm)
            } else {
                format!("{}, {} {}, {}:{:02} {}", day_name, month, utc.day(), h12, minute, am_pm)
            };

            (formatted, epoch)
        }
        Err(_) => (date_str.to_string(), 0),
    }
}

/// Fuzzy-match a user's raw description against live ESPN games.
/// Returns the best matching game if confident enough.
pub fn find_matching_game<'a>(query: &str, games: &'a [EspnGame]) -> Option<&'a EspnGame> {
    let query_lower = query.to_lowercase();

    // Strip common verbs and separators; extract team keyword tokens
    let stripped = query_lower
        .replace(" vs.", " vs ")
        .replace(" v.", " v ")
        .replace('@', " vs ");

    // Words that are separators/verbs, not team names
    const NOISE: &[&str] = &[
        "vs", "v", "at", "beats", "over", "against", "beat", "defeats",
        "the", "a", "an", "in", "on", "game", "match", "bowl",
    ];

    let tokens: Vec<&str> = stripped
        .split_whitespace()
        .filter(|w| !NOISE.contains(w) && w.len() >= 2)
        .collect();

    if tokens.is_empty() {
        return None;
    }

    let mut best_score: i32 = 0;
    let mut best_game: Option<&EspnGame> = None;

    for game in games {
        let home_lower = game.home_team.to_lowercase();
        let away_lower = game.away_team.to_lowercase();
        let home_abbrev_lower = game.home_abbrev.to_lowercase();
        let away_abbrev_lower = game.away_abbrev.to_lowercase();

        let token_matches_team = |token: &str, team: &str, abbrev: &str| -> i32 {
            if abbrev == token {
                return 10; // exact abbreviation match
            }
            if team == token {
                return 20; // exact full-name word match
            }
            if team.contains(token) {
                return token.len() as i32; // substring match weighted by length
            }
            0
        };

        let mut home_score_val: i32 = 0;
        let mut away_score_val: i32 = 0;

        for token in &tokens {
            let hs = token_matches_team(token, &home_lower, &home_abbrev_lower);
            let as_ = token_matches_team(token, &away_lower, &away_abbrev_lower);
            home_score_val = home_score_val.max(hs);
            away_score_val = away_score_val.max(as_);
        }

        // Require both sides to match something meaningful
        if home_score_val < 3 || away_score_val < 3 {
            continue;
        }

        let total = home_score_val + away_score_val;
        if total > best_score {
            best_score = total;
            best_game = Some(game);
        }
    }

    best_game
}
