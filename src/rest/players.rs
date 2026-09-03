//! Player endpoints: list, get, update (the big PATCH), and destroy.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;

use player::AudioTrack;

use crate::node::AppState;
use crate::protocol::omissible::Omissible;
use crate::protocol::player::{Player, PlayerUpdate};
use crate::protocol::track::Track;
use crate::rest::error::{RestError, RestResult};
use crate::session::SocketContext;

fn session(state: &AppState, session_id: &str) -> RestResult<Arc<SocketContext>> {
    state
        .sockets()
        .get(session_id)
        .ok_or_else(|| RestError::not_found(format!("Session {session_id} not found")))
}

/// `GET /v4/sessions/{sessionId}/players`, list every player in a session.
pub async fn get_players(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> RestResult<Json<Vec<Player>>> {
    let context = session(&state, &session_id)?;
    let players = context.players().iter().map(|p| p.snapshot()).collect();
    Ok(Json(players))
}

/// `GET /v4/sessions/{sessionId}/players/{guildId}`, get one player, or `404`.
///
/// A GET never creates a player. Otherwise polling a guild that has none materialises an idle,
/// voice-less player and its event listener per stale guild id, and inflates `stats.players`.
pub async fn get_player(
    State(state): State<AppState>,
    Path((session_id, guild_id)): Path<(String, u64)>,
) -> RestResult<Json<Player>> {
    let context = session(&state, &session_id)?;
    let player = context
        .get_player(guild_id)
        .ok_or_else(|| RestError::not_found("Player not found"))?;
    Ok(Json(player.snapshot()))
}

/// `DELETE /v4/sessions/{sessionId}/players/{guildId}`, destroy a player.
pub async fn delete_player(
    State(state): State<AppState>,
    Path((session_id, guild_id)): Path<(String, u64)>,
) -> RestResult<StatusCode> {
    let context = session(&state, &session_id)?;
    context.remove_player(guild_id);
    Ok(StatusCode::NO_CONTENT)
}

/// `PATCH /v4/sessions/{sessionId}/players/{guildId}`, update a player.
///
/// Applies the [`PlayerUpdate`] in protocol order: voice, then paused and userData when no new track
/// is being loaded, volume, position and endTime under the same condition, filters, then the track
/// load, replace or stop. `?noReplace=true` skips the load when a track is already playing.
///
/// Track resolution happens before the per-player lock is taken, so the lock covers only the
/// mutation sequence and never a network load.
pub async fn patch_player(
    State(state): State<AppState>,
    Path((session_id, guild_id)): Path<(String, u64)>,
    Query(params): Query<HashMap<String, String>>,
    Json(update): Json<PlayerUpdate>,
) -> RestResult<Json<Player>> {
    let context = session(&state, &session_id)?;
    let no_replace = params
        .get("noReplace")
        .map(|v| v == "true")
        .unwrap_or(false);

    let has_track = update.track.is_present();
    let has_legacy = update.encoded_track.is_present() || update.identifier.is_present();
    if has_track && has_legacy {
        return Err(RestError::bad_request(
            "Cannot specify both 'track' and 'encodedTrack'/'identifier'",
        ));
    }

    // Normalise the track spec into (encoded, identifier, userData). These `update` fields are not
    // read again after this point, so move out of them instead of cloning.
    let (encoded, identifier, user_data) = match update.track {
        Omissible::Present(track) => (track.encoded, track.identifier, track.user_data),
        Omissible::Omitted => (update.encoded_track, update.identifier, Omissible::Omitted),
    };
    if encoded.is_present() && identifier.is_present() {
        return Err(RestError::bad_request(
            "Cannot specify both 'encoded' and 'identifier'",
        ));
    }

    if let Omissible::Present(filters) = &update.filters {
        let disabled = state.config().lavalink.server.disabled_filters();
        let rejected: Vec<String> = filters
            .set_filter_names()
            .into_iter()
            .filter(|name| disabled.contains(name))
            .collect();
        if !rejected.is_empty() {
            return Err(RestError::bad_request(format!(
                "These filters are disabled in the server config: {}",
                rejected.join(", ")
            )));
        }
    }

    if let Omissible::Present(voice) = &update.voice {
        if !voice.is_complete() {
            return Err(RestError::bad_request(
                "token, endpoint, sessionId and channelId must be provided in voice state",
            ));
        }
    }

    if let Omissible::Present(Some(end_time)) = &update.end_time {
        if *end_time <= 0 {
            return Err(RestError::bad_request("endTime must be greater than 0"));
        }
    }

    let player = context.get_or_create_player(guild_id);
    let updating_track = encoded.is_present() || identifier.is_present();

    // `noReplace` vetoes the load outright, so check it before paying for a resolve. Re-checked
    // under the lock below, where another PATCH may have started a track in the meantime.
    if updating_track && no_replace && player.has_track() {
        return Ok(Json(player.snapshot()));
    }

    // Resolve tracks before taking the per-player lock: `load_item` hits the network, and holding
    // the lock across it queues every other PATCH for this guild behind the load.
    let resolved_next = match update.next_track {
        Omissible::Present(Some(spec)) => {
            let track = resolve_track(&state, &spec.encoded, &spec.identifier)
                .await?
                .ok_or_else(|| RestError::bad_request("A nextTrack must identify a track"))?;
            let mut proto = Track::from_engine(state.manager(), track.as_ref())
                .map_err(|err| RestError::from_friendly(&err))?;
            if let Omissible::Present(user_data) = spec.user_data {
                proto.user_data = user_data;
            }
            Omissible::Present(Some((track, proto)))
        }
        Omissible::Present(None) => Omissible::Present(None),
        Omissible::Omitted => Omissible::Omitted,
    };

    // `None` with `updating_track` set means stop, from an explicit `encoded: null`.
    let resolved_track = if updating_track {
        resolve_track(&state, &encoded, &identifier)
            .await?
            .map(|track| {
                let mut proto = Track::from_engine(state.manager(), track.as_ref())
                    .map_err(|err| RestError::from_friendly(&err))?;
                if let Omissible::Present(user_data) = user_data.clone() {
                    proto.user_data = user_data;
                }
                Ok::<_, RestError>((track, proto))
            })
            .transpose()?
    } else {
        None
    };

    // Voice first, and outside the patch lock: holding that across the handshake's round-trips parks
    // the play PATCH behind it, delaying the decode thread. `apply_voice` serializes against itself.
    if let Omissible::Present(voice) = update.voice {
        player.apply_voice(voice).await?;
    }

    // Serialise the mutations per guild so a concurrent PATCH can't interleave halfway through.
    let _serialized = player.lock_patch().await;

    // Transition state is independent of a base-track replacement: the client may patch the held
    // successor while the current track keeps playing.
    if let Omissible::Present(next) = resolved_next {
        player.set_next_track(next);
    }

    if let Omissible::Present(crossfade) = update.crossfade {
        match crossfade {
            Some(settings) => player.set_crossfade(settings.to_engine()),
            None => player.reset_crossfade(),
        }
    }

    // `null` and `{"enable": false}` both mean off, since the tape has no node-wide default to fall
    // back to. Applied above `paused`, so one PATCH can configure the ramp and pause with it.
    if let Omissible::Present(tape) = update.tape {
        player.set_tape(tape.and_then(|settings| settings.to_engine()));
    }

    if matches!(update.transition, Omissible::Present(true)) {
        if updating_track {
            return Err(RestError::bad_request(
                "Cannot trigger a transition while replacing the current track",
            ));
        }
        if !player.transition_now() {
            return Err(RestError::bad_request(
                "No active crossfade/gapless transition is available",
            ));
        }
    }

    // Field order: paused, userData, volume, position, endTime, filters, then the load. The first
    // four apply here only when no track is being loaded.
    if !updating_track {
        if let Omissible::Present(paused) = update.paused {
            player.set_paused(paused);
        }
        if let Omissible::Present(user_data) = user_data {
            player.set_user_data(user_data);
        }
    }
    if let Omissible::Present(volume) = update.volume {
        player.set_volume(volume);
    }
    if !updating_track {
        // Guarded on holding a track: without it a trackless player emits a player update for a
        // seek that had nothing to move.
        if let Omissible::Present(position) = update.position {
            if player.has_track() {
                player.seek(position);
            }
        }
        if let Omissible::Present(end_time) = update.end_time.clone() {
            player.set_end_time(end_time);
        }
    }
    if let Omissible::Present(filters) = update.filters {
        player.set_filters(filters);
    }

    if updating_track {
        // Another PATCH may have started a track while we were resolving, unlocked, above.
        if no_replace && player.has_track() {
            return Ok(Json(player.snapshot()));
        }

        match resolved_track {
            Some((track, proto)) => {
                // A new track defaults to playing unless `paused` is explicitly set.
                let paused = match update.paused {
                    Omissible::Present(paused) => paused,
                    Omissible::Omitted => false,
                };
                player.set_paused(paused);

                // `position` is the new track's start position rather than a seek afterwards, so
                // decoding begins there and no audio from 0 leaks out.
                let start_position = match update.position {
                    Omissible::Present(position) => position.max(0) as u64,
                    Omissible::Omitted => 0,
                };
                player.play_at(track, proto, start_position);

                if let Omissible::Present(end_time) = update.end_time {
                    player.set_end_time(end_time);
                }
            }
            None => player.stop(),
        }
    }

    Ok(Json(player.snapshot()))
}

// Resolve the track to play from a base64 `encoded` or an `identifier` spec. `Ok(None)` means stop,
// from an explicit `encoded: null`.
async fn resolve_track(
    state: &AppState,
    encoded: &Omissible<Option<String>>,
    identifier: &Omissible<String>,
) -> RestResult<Option<Box<dyn AudioTrack>>> {
    if let Omissible::Present(encoded) = encoded {
        return match encoded {
            Some(encoded) => {
                let track = state
                    .manager()
                    .decode_track(encoded)
                    .map_err(|err| RestError::from_friendly(&err))?
                    .ok_or_else(|| RestError::bad_request("Could not decode the provided track"))?;
                Ok(Some(track))
            }
            None => Ok(None),
        };
    }

    if let Omissible::Present(identifier) = identifier {
        return match state.manager().load_item(identifier.clone()).await {
            player::LoadResult::Track(track) => Ok(Some(track)),
            // A PATCH plays a single track, so the client has to load a playlist or search result
            // and pick from it first.
            player::LoadResult::Playlist(_) => Err(RestError::bad_request(
                "Cannot play a playlist or search result",
            )),
            player::LoadResult::NoMatches => {
                Err(RestError::bad_request("No matches found for identifier"))
            }
            player::LoadResult::LoadFailed(err) => Err(RestError::from_friendly(&err)),
        };
    }

    Ok(None)
}
