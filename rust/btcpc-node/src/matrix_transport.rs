//! Matrix room transport — Phase 5 of the transport cascade.
//!
//! Propagates ledger entries through a Matrix federated chat room so nodes
//! across the open internet can relay entries without requiring direct libp2p
//! connectivity.  Each accepted entry is broadcast as a JSON text message in
//! the configured room; every room message is parsed and, if valid, applied
//! through the canonical `chain.apply_entry()` path.
//!
//! Design rules:
//!  - Matrix room members do NOT count toward `peer_count`.  The zero-peers
//!    hardline in api.rs is about libp2p peers only; Matrix cannot satisfy it.
//!  - Every received message goes through `chain.apply_entry()` — the same
//!    canonical validation path used by every other transport.
//!  - Accepted entries are re-broadcast to libp2p (`NetCmd::Broadcast`) so
//!    they propagate further via the gossipsub mesh.
//!  - Epoch seals are never generated from Matrix events.
//!
//! Env vars:
//!   BTCPC_MATRIX               — "true" to activate
//!   BTCPC_MATRIX_HOMESERVER    — e.g. "https://matrix.org"
//!   BTCPC_MATRIX_USER          — full MXID, e.g. "@btcpc:matrix.org"
//!   BTCPC_MATRIX_PASSWORD      — account password
//!   BTCPC_MATRIX_ROOM          — room ID or alias, e.g. "#btcpc-entries:matrix.org"

use std::sync::Arc;

use btcpc_types::LedgerEntry;
use matrix_sdk::{
    Client, Room,
    authentication::matrix::MatrixSession,
    config::SyncSettings,
    ruma::{
        OwnedRoomId, OwnedRoomOrAliasId,
        events::room::message::{
            MessageType, RoomMessageEventContent, SyncRoomMessageEvent,
        },
    },
};
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::chain::Chain;
use crate::net::NetCmd;

// ---------------------------------------------------------------------------
// Public handle (cheap clone — wraps an mpsc::Sender)
// ---------------------------------------------------------------------------

/// Clone-able handle used by the broadcast pump to forward accepted entries to
/// the Matrix room.
#[derive(Clone)]
pub struct MatrixHandle {
    tx: mpsc::Sender<LedgerEntry>,
}

impl MatrixHandle {
    /// Non-blocking: queues `entry` for delivery to the Matrix room.  Drops the
    /// entry silently if the internal channel is full — libp2p gossipsub is the
    /// reliable path; Matrix is opportunistic.
    pub fn send_entry(&self, entry: LedgerEntry) {
        let _ = self.tx.try_send(entry);
    }
}

// ---------------------------------------------------------------------------
// Start function — called from main.rs before AppState construction
// ---------------------------------------------------------------------------

/// Initialise the Matrix transport.  Returns `None` if `BTCPC_MATRIX` is not
/// set to `"true"` or if the connection/login attempt fails.
///
/// On success spawns two tasks (send loop + sync loop) and returns a cloneable
/// [`MatrixHandle`] the caller can use to forward entries into the room.
pub async fn start_matrix(
    chain: Arc<Chain>,
    cmd_tx: mpsc::Sender<NetCmd>,
) -> Option<MatrixHandle> {
    if std::env::var("BTCPC_MATRIX").as_deref() != Ok("true") {
        return None;
    }

    let homeserver = std::env::var("BTCPC_MATRIX_HOMESERVER").unwrap_or_default();
    let user       = std::env::var("BTCPC_MATRIX_USER").unwrap_or_default();
    let password   = std::env::var("BTCPC_MATRIX_PASSWORD").unwrap_or_default();
    let room_spec  = std::env::var("BTCPC_MATRIX_ROOM").unwrap_or_default();

    if homeserver.is_empty() || user.is_empty() || password.is_empty() || room_spec.is_empty() {
        warn!("matrix: BTCPC_MATRIX=true but required env vars are missing — disabled");
        return None;
    }

    match try_start(chain, cmd_tx, homeserver, user, password, room_spec).await {
        Some(handle) => Some(handle),
        None => {
            warn!("matrix: transport not started — see warnings above");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Internal setup
// ---------------------------------------------------------------------------

async fn try_start(
    chain:      Arc<Chain>,
    cmd_tx:     mpsc::Sender<NetCmd>,
    homeserver: String,
    user:       String,
    password:   String,
    room_spec:  String,
) -> Option<MatrixHandle> {
    // Build the Matrix client (in-memory state store — no sqlite dependency).
    let client = match Client::builder()
        .homeserver_url(&homeserver)
        .build()
        .await
    {
        Ok(c) => c,
        Err(e) => {
            warn!("matrix: failed to create client for {}: {}", homeserver, e);
            return None;
        }
    };

    // Restore a previous session if we stored one, otherwise do a fresh login.
    let logged_in = restore_or_login(&client, &chain, &user, &password).await;
    if !logged_in {
        return None;
    }

    // Persist the new session tokens so we can reconnect without re-logging in.
    persist_session(&client, &chain);

    // Join (or confirm membership of) the target room.
    let room_id: OwnedRoomId = match join_room(&client, &room_spec).await {
        Some(id) => id,
        None => return None,
    };

    info!("matrix: connected to {} in room {}", homeserver, room_id);

    // Channel used by the handle to send entries → the outbound send loop.
    let (tx, rx) = mpsc::channel::<LedgerEntry>(256);
    let handle = MatrixHandle { tx };

    // Register the inbound message handler before starting sync.
    {
        let chain_ref  = chain.clone();
        let cmd_tx_ref = cmd_tx.clone();
        let room_id_ref = room_id.clone();

        // Share chain + p2p sender with the event handler via Arc.
        let shared: Arc<(Arc<Chain>, mpsc::Sender<NetCmd>)> =
            Arc::new((chain_ref, cmd_tx_ref));
        client.add_event_handler_context(shared.clone());

        client.add_event_handler(
            move |ev: SyncRoomMessageEvent, room: Room, ctx: matrix_sdk::event_handler::Ctx<Arc<(Arc<Chain>, mpsc::Sender<NetCmd>)>>| {
                let room_id_expected = room_id_ref.clone();
                async move {
                    // Ignore events from rooms other than ours.
                    if room.room_id() != room_id_expected {
                        return;
                    }
                    // ctx.0 is Arc<(Arc<Chain>, Sender<NetCmd>)>
                    handle_message(ev, &ctx.0.0, &ctx.0.1).await;
                }
            },
        );
    }

    // Outbound send loop — waits for entries on `rx` and posts to the room.
    {
        let client_for_send = client.clone();
        let room_id_for_send = room_id.clone();
        tokio::spawn(async move {
            run_send_loop(client_for_send, room_id_for_send, rx).await;
        });
    }

    // Inbound sync loop — drives the Matrix client's background sync.
    tokio::spawn(async move {
        run_sync_loop(client).await;
    });

    Some(handle)
}

// ---------------------------------------------------------------------------
// Login / session restore
// ---------------------------------------------------------------------------

async fn restore_or_login(
    client:   &Client,
    chain:    &Arc<Chain>,
    user:     &str,
    password: &str,
) -> bool {
    // Try to restore a previously saved session.
    if let Some(session) = load_session(chain) {
        use matrix_sdk::store::RoomLoadSettings;
        match client.matrix_auth().restore_session(session, RoomLoadSettings::default()).await {
            Ok(()) => {
                info!("matrix: restored previous session");
                return true;
            }
            Err(e) => {
                warn!("matrix: session restore failed ({}), re-logging in", e);
                // Fall through to fresh login.
            }
        }
    }

    // Fresh password login.
    match client.matrix_auth()
        .login_username(user, password)
        .initial_device_display_name("btcpc-node")
        .send()
        .await
    {
        Ok(_) => {
            info!("matrix: logged in as {}", user);
            true
        }
        Err(e) => {
            warn!("matrix: login failed: {}", e);
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Session persistence (stored in chain's RocksDB state store)
// ---------------------------------------------------------------------------

fn load_session(chain: &Arc<Chain>) -> Option<MatrixSession> {
    let bytes = chain.store.state_get("matrix:session")?;
    serde_json::from_slice(&bytes)
        .map_err(|e| warn!("matrix: failed to deserialise stored session: {}", e))
        .ok()
}

fn persist_session(client: &Client, chain: &Arc<Chain>) {
    if let Some(session) = client.matrix_auth().session() {
        match serde_json::to_vec(&session) {
            Ok(bytes) => {
                let _ = chain.store.state_set("matrix:session", &bytes);
            }
            Err(e) => {
                warn!("matrix: failed to serialise session for storage: {}", e);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Room join helper
// ---------------------------------------------------------------------------

async fn join_room(client: &Client, room_spec: &str) -> Option<OwnedRoomId> {
    // Parse the room spec as a room ID or alias.
    let alias: OwnedRoomOrAliasId = match room_spec.try_into() {
        Ok(a) => a,
        Err(e) => {
            warn!("matrix: invalid room ID or alias '{}': {}", room_spec, e);
            return None;
        }
    };

    match client.join_room_by_id_or_alias(&alias, &[]).await {
        Ok(room) => Some(room.room_id().to_owned()),
        Err(e) => {
            warn!("matrix: failed to join room '{}': {}", room_spec, e);
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Inbound: handle a Matrix room message event
// ---------------------------------------------------------------------------

async fn handle_message(
    ev:     SyncRoomMessageEvent,
    chain:  &Arc<Chain>,
    cmd_tx: &mpsc::Sender<NetCmd>,
) {
    // Extract the plain-text body from the event.
    let body = match ev.as_original() {
        Some(original) => {
            match &original.content.msgtype {
                MessageType::Text(text) => text.body.clone(),
                _ => return, // Not a text message — ignore.
            }
        }
        None => return, // Redacted event — ignore.
    };

    // Parse the JSON envelope: {"entry": <LedgerEntry>}.
    let json: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => return, // Not JSON — silently drop.
    };

    let Some(entry_val) = json.get("entry") else { return; };

    let entry: LedgerEntry = match serde_json::from_value(entry_val.clone()) {
        Ok(e) => e,
        Err(_) => return, // Unrecognised entry shape — silently drop.
    };

    // Canonical validation path — same as every other transport.
    match chain.apply_entry(&entry) {
        Ok(_) => {
            debug!("matrix: applied entry received from room");

            // Re-broadcast to libp2p so the entry propagates to peers that are
            // not listening in the Matrix room.
            let envelope = serde_json::json!({ "entry": &entry });
            if let Ok(bytes) = serde_json::to_vec(&envelope) {
                let _ = cmd_tx.send(NetCmd::Broadcast {
                    topic: "btcpc/entries",
                    data: bytes,
                }).await;
            }
        }
        Err(_) => {
            // Validation failure (duplicate, bad sig, wrong state, etc.).
            // Silently drop — do not re-broadcast.
        }
    }
}

// ---------------------------------------------------------------------------
// Outbound: send loop — forwards entries queued via MatrixHandle::send_entry
// ---------------------------------------------------------------------------

async fn run_send_loop(
    client:  Client,
    room_id: OwnedRoomId,
    mut rx:  mpsc::Receiver<LedgerEntry>,
) {
    while let Some(entry) = rx.recv().await {
        let room = match client.get_room(&room_id) {
            Some(r) => r,
            None => {
                debug!("matrix: room handle lost, dropping outbound entry");
                continue;
            }
        };

        let body = match serde_json::to_string(&serde_json::json!({ "entry": &entry })) {
            Ok(s) => s,
            Err(e) => {
                warn!("matrix: failed to serialise entry: {}", e);
                continue;
            }
        };

        if let Err(e) = room.send(RoomMessageEventContent::text_plain(body)).await {
            debug!("matrix: failed to send entry to room: {}", e);
        }
    }
}

// ---------------------------------------------------------------------------
// Sync loop — drives the Matrix client event pump
// ---------------------------------------------------------------------------

async fn run_sync_loop(client: Client) {
    // sync() blocks indefinitely, processing all incoming events and calling
    // registered event handlers.  On network error it retries automatically.
    if let Err(e) = client.sync(SyncSettings::default()).await {
        warn!("matrix: sync ended with error: {}", e);
    }
}
