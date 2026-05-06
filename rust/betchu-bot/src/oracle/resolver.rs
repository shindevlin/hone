use crate::contract::{BetPool, ContractService};
use crate::oracle::EspnScraper;
use crate::oracle::espn::EspnGame;
use anyhow::Result;
use std::sync::Arc;
use tracing::{info, warn};

pub struct ScoreResolver {
    espn: Arc<EspnScraper>,
    contract: Arc<ContractService>,
}

impl ScoreResolver {
    pub fn new(espn: Arc<EspnScraper>, contract: Arc<ContractService>) -> Self {
        Self { espn, contract }
    }

    /// Scan recent pools, match against ESPN final games, and submit reportScore on-chain.
    /// Returns the pool IDs successfully resolved.
    pub async fn check_and_resolve(&self) -> Result<Vec<u32>> {
        // Only consider games ESPN has marked as final — this is the ESPN confirmation gate.
        let finished = match self.espn.get_finished_games().await {
            Ok(g) => g,
            Err(e) => {
                warn!("ESPN fetch failed: {}", e);
                return Ok(vec![]);
            }
        };

        if finished.is_empty() {
            return Ok(vec![]);
        }

        let pool_count = match self.contract.get_pool_count().await {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to get pool count: {}", e);
                return Ok(vec![]);
            }
        };

        let mut resolved_ids = Vec::new();
        let recent_start = pool_count.saturating_sub(50);

        for pool_id in recent_start..pool_count {
            let pool = match self.contract.get_pool(pool_id).await {
                Ok(p) => p,
                Err(e) => {
                    warn!("get_pool({}) failed: {}", pool_id, e);
                    continue;
                }
            };

            // Only resolve pools whose deadline has passed and are not yet resolved.
            if !pool.needs_resolution() {
                continue;
            }

            if let Some((winner_side, game)) = match_pool_to_game(&pool, &finished) {
                info!(
                    "Pool #{} matched '{}' → ESPN '{}' is_final=true → side {} wins",
                    pool_id, pool.description, game.name, winner_side
                );

                match self.contract.report_score(pool_id, winner_side).await {
                    Ok(tx_hash) => {
                        info!("reportScore(pool={}, side={}) tx={}", pool_id, winner_side, tx_hash);
                        resolved_ids.push(pool_id);
                    }
                    Err(e) => {
                        warn!("reportScore(pool={}) failed: {}", pool_id, e);
                    }
                }
            }
        }

        Ok(resolved_ids)
    }
}

/// Match a pool's normalized description to a finished ESPN game.
///
/// Description format: "{away} at {home} ({sport}) — {date}"
/// Side convention: Side 1 = Away team, Side 2 = Home team (matches description order).
///
/// Returns (winner_side, matched_game) or None.
fn match_pool_to_game<'a>(
    pool: &BetPool,
    games: &'a [EspnGame],
) -> Option<(u8, &'a EspnGame)> {
    // 1. Exact normalized description match (fast path — works for oracle-created pools)
    if let Some(game) = games.iter().find(|g| g.normalized_description() == pool.description) {
        return determine_winner(game).map(|side| (side, game));
    }

    // 2. Fuzzy fallback: both team names must appear in the description
    let desc_lower = pool.description.to_lowercase();
    let mut best: Option<(u8, &EspnGame)> = None;
    let mut best_score = 0usize;

    for game in games {
        let home = game.home_team.to_lowercase();
        let away = game.away_team.to_lowercase();

        let home_hit = desc_lower.contains(&home) || desc_lower.contains(&game.home_abbrev.to_lowercase());
        let away_hit = desc_lower.contains(&away) || desc_lower.contains(&game.away_abbrev.to_lowercase());

        if !home_hit || !away_hit {
            continue;
        }

        // Score by total matched characters (longer names = more specific match)
        let score = home.len() + away.len();
        if score > best_score {
            if let Some(side) = determine_winner(game) {
                best_score = score;
                best = Some((side, game));
            }
        }
    }

    best
}

/// Determine the winning side from ESPN scores.
/// Side 1 = Away team, Side 2 = Home team (matches "{away} at {home}" description order).
fn determine_winner(game: &EspnGame) -> Option<u8> {
    let home = game.home_score?;
    let away = game.away_score?;

    if away > home {
        Some(1) // Away team wins → Side A
    } else if home > away {
        Some(2) // Home team wins → Side B
    } else {
        None // Tie — don't auto-resolve
    }
}
