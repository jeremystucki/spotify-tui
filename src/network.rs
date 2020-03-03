use crate::app::{
    ActiveBlock, AlbumTableContext, App, Artist, ArtistBlock, RouteId, SelectedAlbum,
    SelectedFullAlbum, TrackTableContext,
};
use crate::config::ClientConfig;
use rspotify::{
    client::Spotify,
    model::{
        album::SimplifiedAlbum,
        offset::for_position,
        page::Page,
        playlist::{PlaylistTrack, SimplifiedPlaylist},
        recommend::Recommendations,
        track::FullTrack,
    },
    oauth2::{SpotifyClientCredentials, SpotifyOAuth, TokenInfo},
    senum::{Country, RepeatState},
    util::get_token,
};
use serde_json::{map::Map, Value};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use tokio::try_join;

#[derive(Debug)]
pub enum IoEvent {
    GetCurrentPlayback,
    RefreshAuthentication,
    GetPlaylists,
    GetDevices,
    GetSearchResults(String, Option<Country>),
    SetTracksToTable(Vec<FullTrack>),
    GetMadeForYouPlaylistTracks(String, u32),
    GetPlaylistTracks(String, u32),
    GetCurrentSavedTracks(Option<u32>, bool),
    StartPlayback(Option<String>, Option<Vec<String>>, Option<usize>),
    UpdateSearchLimits(u32, u32),
    Seek(u32),
    NextTrack,
    PreviousTrack,
    Shuffle(bool),
    Repeat(RepeatState),
    PausePlayback,
    ChangeVolume(u8),
    GetArtist(String, String, Option<Country>),
    GetAlbumTracks(SimplifiedAlbum),
    GetRecommendationsForSeed(
        Option<Vec<String>>,
        Option<Vec<String>>,
        Option<FullTrack>,
        Option<Country>,
    ),
    GetCurrentUserSavedAlbums(Option<u32>),
    CurrentUserSavedAlbumDelete(String),
    CurrentUserSavedAlbumAdd(String),
    UserUnfollowArtists(Vec<String>),
    UserFollowArtists(Vec<String>),
    UserFollowPlaylist(String, String, Option<bool>),
    UserUnfollowPlaylist(String, String),
    MadeForYouSearchAndAdd(String, Option<Country>),
    GetAudioAnalysis(String),
    GetUser,
    ToggleSaveTrack(String),
    GetRecommendationsForTrackId(String, Option<Country>),
    GetRecentlyPlayed,
    GetFollowedArtists(Option<String>),
    GetAlbum(String),
    SetDeviceIdInConfig(String),
}

pub fn get_spotify(token_info: TokenInfo) -> (Spotify, Instant) {
    let token_expiry = Instant::now()
        + Duration::from_secs(token_info.expires_in.into())
        // Set 10 seconds early
        - Duration::from_secs(10);

    let client_credential = SpotifyClientCredentials::default()
        .token_info(token_info)
        .build();

    let spotify = Spotify::default()
        .client_credentials_manager(client_credential)
        .build();

    (spotify, token_expiry)
}

#[derive(Debug, Clone)]
pub struct Network {
    oauth: SpotifyOAuth,
    spotify: Spotify,
    spotify_token_expiry: Instant,
    large_search_limit: u32,
    small_search_limit: u32,
    client_config: ClientConfig,
}

type AppArc = Arc<Mutex<App>>;

impl Network {
    pub fn new(
        oauth: SpotifyOAuth,
        spotify: Spotify,
        spotify_token_expiry: Instant,
        client_config: ClientConfig,
    ) -> Self {
        Network {
            oauth,
            spotify,
            spotify_token_expiry,
            large_search_limit: 20,
            small_search_limit: 4,
            client_config,
        }
    }

    pub async fn handle_network_event(&mut self, io_event: IoEvent, app: &AppArc) {
        match io_event {
            IoEvent::RefreshAuthentication => {
                if let Some(new_token_info) = get_token(&mut self.oauth).await {
                    let (new_spotify, new_token_expiry) = get_spotify(new_token_info);
                    self.spotify = new_spotify;
                    self.spotify_token_expiry = new_token_expiry;
                } else {
                    println!("\nFailed to refresh authentication token");
                    // TODO panic!
                }
            }
            IoEvent::GetPlaylists => {
                self.get_current_user_playlists(&app).await;
            }
            IoEvent::GetUser => {
                self.get_user(&app).await;
            }
            IoEvent::GetDevices => {
                self.get_devices(&app).await;
            }
            IoEvent::GetCurrentPlayback => {
                self.get_current_playback(&app).await;
            }
            IoEvent::SetTracksToTable(full_tracks) => {
                self.set_tracks_to_table(&app, full_tracks).await;
            }
            IoEvent::GetSearchResults(search_term, country) => {
                self.get_search_results(&app, search_term, country).await;
            }
            IoEvent::GetMadeForYouPlaylistTracks(playlist_id, made_for_you_offset) => {
                self.get_made_for_you_playlist_tracks(&app, playlist_id, made_for_you_offset)
                    .await;
            }
            IoEvent::GetPlaylistTracks(playlist_id, playlist_offset) => {
                self.get_playlist_tracks(&app, playlist_id, playlist_offset)
                    .await;
            }
            IoEvent::GetCurrentSavedTracks(offset, should_navigate) => {
                self.get_current_user_saved_tracks(&app, offset, should_navigate)
                    .await;
            }
            IoEvent::StartPlayback(context_uri, uris, offset) => {
                self.start_playback(&app, context_uri, uris, offset).await;
            }
            IoEvent::UpdateSearchLimits(large_search_limit, small_search_limit) => {
                self.large_search_limit = large_search_limit;
                self.small_search_limit = small_search_limit;
            }
            IoEvent::Seek(position_ms) => {
                self.seek(&app, position_ms).await;
            }
            IoEvent::NextTrack => {
                self.next_track(&app).await;
            }
            IoEvent::PreviousTrack => {
                self.previous_track(&app).await;
            }
            IoEvent::Repeat(repeat_state) => {
                self.repeat(&app, repeat_state).await;
            }
            IoEvent::PausePlayback => {
                self.pause_playback(&app).await;
            }
            IoEvent::ChangeVolume(volume) => {
                self.change_volume(&app, volume).await;
            }
            IoEvent::GetArtist(artist_id, input_artist_name, country) => {
                self.get_artist(&app, artist_id, input_artist_name, country)
                    .await;
            }
            IoEvent::GetAlbumTracks(album) => {
                self.get_album_tracks(&app, album).await;
            }
            IoEvent::GetRecommendationsForSeed(seed_artists, seed_tracks, first_track, country) => {
                self.get_recommendations_for_seed(
                    app,
                    seed_artists,
                    seed_tracks,
                    first_track,
                    country,
                )
                .await;
            }
            IoEvent::GetCurrentUserSavedAlbums(offset) => {
                self.get_current_user_saved_albums(&app, offset).await;
            }
            IoEvent::CurrentUserSavedAlbumDelete(album_id) => {
                self.current_user_saved_album_delete(&app, album_id).await;
            }
            IoEvent::CurrentUserSavedAlbumAdd(album_id) => {
                self.current_user_saved_album_add(&app, album_id).await;
            }
            IoEvent::UserUnfollowArtists(artist_ids) => {
                self.user_unfollow_artists(&app, artist_ids).await;
            }
            IoEvent::UserFollowArtists(artist_ids) => {
                self.user_follow_artists(&app, artist_ids).await;
            }
            IoEvent::UserFollowPlaylist(playlist_owner_id, playlist_id, is_public) => {
                self.user_follow_playlist(&app, playlist_owner_id, playlist_id, is_public)
                    .await;
            }
            IoEvent::UserUnfollowPlaylist(user_id, playlist_id) => {
                self.user_unfollow_playlist(&app, user_id, playlist_id)
                    .await;
            }
            IoEvent::MadeForYouSearchAndAdd(search_term, country) => {
                self.made_for_you_search_and_add(&app, search_term, country)
                    .await;
            }
            IoEvent::GetAudioAnalysis(uri) => {
                self.get_audio_analysis(&app, uri).await;
            }
            IoEvent::ToggleSaveTrack(track_id) => {
                self.toggle_save_track(&app, track_id).await;
            }
            IoEvent::GetRecommendationsForTrackId(track_id, country) => {
                self.get_recommendations_for_track_id(&app, track_id, country)
                    .await;
            }
            IoEvent::GetRecentlyPlayed => {
                self.get_recently_played(&app).await;
            }
            IoEvent::GetFollowedArtists(after) => {
                self.get_followed_artists(&app, after).await;
            }
            IoEvent::GetAlbum(album_id) => {
                self.get_album(&app, album_id).await;
            }
            IoEvent::SetDeviceIdInConfig(device_id) => {
                self.set_device_id_in_config(&app, device_id).await;
            }
            IoEvent::Shuffle(shuffle_state) => {
                self.shuffle(app, shuffle_state).await;
            }
        };

        let mut app = app.lock().await;
        app.is_loading = false;
    }

    async fn handle_error(&self, app: &AppArc, e: failure::Error) {
        let mut app = app.lock().await;
        app.handle_error(e);
    }

    async fn get_user(&self, app: &AppArc) {
        match self.spotify.current_user().await {
            Ok(user) => {
                let mut app = app.lock().await;
                app.user = Some(user);
            }
            Err(e) => {
                self.handle_error(app, e).await;
            }
        }
    }

    async fn get_devices(&self, app: &AppArc) {
        if let Ok(result) = self.spotify.device().await {
            let mut app = app.lock().await;
            app.push_navigation_stack(RouteId::SelectedDevice, ActiveBlock::SelectDevice);
            if !result.devices.is_empty() {
                app.devices = Some(result);
                // Select the first device in the list
                app.selected_device_index = Some(0);
            }
        }
    }

    async fn get_current_playback(&self, app: &AppArc) {
        let context = self.spotify.current_playback(None).await;
        if let Ok(ctx) = context {
            if let Some(c) = ctx {
                if let Some(track) = &c.item {
                    if let Some(track_id) = &track.id {
                        self.current_user_saved_tracks_contains(app, vec![track_id.to_owned()])
                            .await;
                    }
                }
                let mut app = app.lock().await;
                app.current_playback_context = Some(c.clone());
                app.instant_since_last_current_playback_poll = Instant::now();
            };
        }
    }

    async fn current_user_saved_tracks_contains(&self, app: &AppArc, ids: Vec<String>) {
        match self.spotify.current_user_saved_tracks_contains(&ids).await {
            Ok(is_saved_vec) => {
                for (i, id) in ids.iter().enumerate() {
                    if let Some(is_liked) = is_saved_vec.get(i) {
                        let mut app = app.lock().await;
                        if *is_liked {
                            app.liked_song_ids_set.insert(id.to_string());
                        } else {
                            // The song is not liked, so check if it should be removed
                            if app.liked_song_ids_set.contains(id) {
                                app.liked_song_ids_set.remove(id);
                            }
                        }
                    };
                }
            }
            Err(e) => {
                self.handle_error(app, e).await;
            }
        }
    }

    async fn get_playlist_tracks(&self, app: &AppArc, playlist_id: String, playlist_offset: u32) {
        if let Ok(playlist_tracks) = self
            .spotify
            .user_playlist_tracks(
                "spotify",
                &playlist_id,
                None,
                Some(self.large_search_limit),
                Some(playlist_offset),
                None,
            )
            .await
        {
            self.set_playlist_tracks_to_table(app, &playlist_tracks)
                .await;

            let mut app = app.lock().await;
            app.playlist_tracks = Some(playlist_tracks);
            if app.get_current_route().id != RouteId::TrackTable {
                app.push_navigation_stack(RouteId::TrackTable, ActiveBlock::TrackTable);
            };
        };
    }

    async fn set_playlist_tracks_to_table(
        &self,
        app: &AppArc,
        playlist_track_page: &Page<PlaylistTrack>,
    ) {
        self.set_tracks_to_table(
            app,
            playlist_track_page
                .items
                .clone()
                .into_iter()
                .map(|item| item.track.unwrap())
                .collect::<Vec<FullTrack>>(),
        )
        .await;
    }

    async fn set_tracks_to_table(&self, app: &AppArc, tracks: Vec<FullTrack>) {
        self.current_user_saved_tracks_contains(
            app,
            tracks
                .clone()
                .into_iter()
                .filter_map(|item| item.id)
                .collect::<Vec<String>>(),
        )
        .await;

        let mut app = app.lock().await;
        app.track_table.tracks = tracks;
    }

    async fn get_made_for_you_playlist_tracks(
        &self,
        app: &AppArc,
        playlist_id: String,
        made_for_you_offset: u32,
    ) {
        if let Ok(made_for_you_tracks) = self
            .spotify
            .user_playlist_tracks(
                "spotify",
                &playlist_id,
                None,
                Some(self.large_search_limit),
                Some(made_for_you_offset),
                None,
            )
            .await
        {
            self.set_playlist_tracks_to_table(app, &made_for_you_tracks)
                .await;

            let mut app = app.lock().await;
            app.made_for_you_tracks = Some(made_for_you_tracks);
            if app.get_current_route().id != RouteId::TrackTable {
                app.push_navigation_stack(RouteId::TrackTable, ActiveBlock::TrackTable);
            }
        }
    }

    async fn get_search_results(
        &self,
        app: &AppArc,
        search_term: String,
        country: Option<Country>,
    ) {
        let search_track =
            self.spotify
                .search_track(&search_term, self.small_search_limit, 0, country);

        let search_artist =
            self.spotify
                .search_artist(&search_term, self.small_search_limit, 0, country);

        let search_album =
            self.spotify
                .search_album(&search_term, self.small_search_limit, 0, country);

        let search_playlist =
            self.spotify
                .search_playlist(&search_term, self.small_search_limit, 0, country);

        // Run the futures concurrently
        match try_join!(search_track, search_artist, search_album, search_playlist) {
            Ok((track_results, artist_results, album_results, playlist_results)) => {
                self.set_tracks_to_table(app, track_results.tracks.items.clone())
                    .await;
                let mut app = app.lock().await;
                app.search_results.tracks = Some(track_results);
                app.search_results.artists = Some(artist_results);
                app.search_results.albums = Some(album_results);
                app.search_results.playlists = Some(playlist_results);
            }
            Err(e) => {
                self.handle_error(app, e).await;
            }
        };
    }

    async fn get_current_user_saved_tracks(
        &self,
        app: &AppArc,
        offset: Option<u32>,
        should_navigate: bool,
    ) {
        match self
            .spotify
            .current_user_saved_tracks(self.large_search_limit, offset)
            .await
        {
            Ok(saved_tracks) => {
                let mut app = app.lock().await;
                app.set_saved_tracks_to_table(&saved_tracks);

                app.library.saved_tracks.add_pages(saved_tracks);
                app.track_table.context = Some(TrackTableContext::SavedTracks);

                if should_navigate {
                    app.push_navigation_stack(RouteId::TrackTable, ActiveBlock::TrackTable);
                }
            }
            Err(e) => {
                self.handle_error(app, e).await;
            }
        }
    }

    async fn start_playback(
        &self,
        app: &AppArc,
        context_uri: Option<String>,
        uris: Option<Vec<String>>,
        offset: Option<usize>,
    ) {
        let (uris, context_uri) = if context_uri.is_some() {
            (None, context_uri)
        } else if uris.is_some() {
            (uris, None)
        } else {
            (None, None)
        };

        let offset = offset.and_then(|o| for_position(o as u32));

        let result = match &self.client_config.device_id {
            Some(device_id) => {
                self.spotify
                    .start_playback(
                        Some(device_id.to_string()),
                        context_uri.clone(),
                        uris.clone(),
                        offset.clone(),
                        None,
                    )
                    .await
            }
            None => Err(failure::err_msg("No device_id selected")),
        };

        match result {
            Ok(()) => {
                self.get_current_playback(app).await;

                let mut app = app.lock().await;
                app.song_progress_ms = 0;
            }
            Err(e) => {
                self.handle_error(app, e).await;
            }
        }
    }

    async fn seek(&self, app: &AppArc, position_ms: u32) {
        if let Some(device_id) = &self.client_config.device_id {
            match self
                .spotify
                .seek_track(position_ms, Some(device_id.to_string()))
                .await
            {
                Ok(()) => {
                    self.get_current_playback(app).await;
                }
                Err(e) => {
                    self.handle_error(app, e).await;
                }
            };
        }
    }

    async fn next_track(&self, app: &AppArc) {
        match self
            .spotify
            .next_track(self.client_config.device_id.clone())
            .await
        {
            Ok(()) => {
                self.get_current_playback(app).await;
            }
            Err(e) => {
                self.handle_error(app, e).await;
            }
        };
    }

    async fn previous_track(&self, app: &AppArc) {
        match self
            .spotify
            .previous_track(self.client_config.device_id.clone())
            .await
        {
            Ok(()) => {
                self.get_current_playback(app).await;
            }
            Err(e) => {
                self.handle_error(app, e).await;
            }
        };
    }

    async fn shuffle(&self, app: &AppArc, shuffle_state: bool) {
        match self
            .spotify
            .shuffle(!shuffle_state, self.client_config.device_id.clone())
            .await
        {
            Ok(()) => {
                // Update the UI eagerly (otherwise the UI will wait until the next 5 second interval
                // due to polling playback context)
                let mut app = app.lock().await;
                if let Some(current_playback_context) = &mut app.current_playback_context {
                    current_playback_context.shuffle_state = !shuffle_state;
                };
            }
            Err(e) => {
                self.handle_error(app, e).await;
            }
        };
    }

    async fn repeat(&self, app: &AppArc, repeat_state: RepeatState) {
        let next_repeat_state = match repeat_state {
            RepeatState::Off => RepeatState::Context,
            RepeatState::Context => RepeatState::Track,
            RepeatState::Track => RepeatState::Off,
        };
        match self
            .spotify
            .repeat(next_repeat_state, self.client_config.device_id.clone())
            .await
        {
            Ok(()) => {
                let mut app = app.lock().await;
                if let Some(current_playback_context) = &mut app.current_playback_context {
                    current_playback_context.repeat_state = next_repeat_state;
                };
            }
            Err(e) => {
                self.handle_error(app, e).await;
            }
        };
    }

    async fn pause_playback(&self, app: &AppArc) {
        match self
            .spotify
            .pause_playback(self.client_config.device_id.clone())
            .await
        {
            Ok(()) => {
                self.get_current_playback(app).await;
            }
            Err(e) => {
                self.handle_error(app, e).await;
            }
        };
    }

    async fn change_volume(&self, app: &AppArc, volume_percent: u8) {
        match self
            .spotify
            .volume(volume_percent, self.client_config.device_id.clone())
            .await
        {
            Ok(()) => {
                let mut app = app.lock().await;
                if let Some(current_playback_context) = &mut app.current_playback_context {
                    current_playback_context.device.volume_percent = volume_percent.into();
                };
            }
            Err(e) => {
                self.handle_error(app, e).await;
            }
        };
    }

    async fn get_artist(
        &self,
        app: &AppArc,
        artist_id: String,
        input_artist_name: String,
        country: Option<Country>,
    ) {
        let albums = self.spotify.artist_albums(
            &artist_id,
            None,
            country,
            Some(self.large_search_limit),
            Some(0),
        );
        let mut artist_name = String::from("");
        if input_artist_name == "" {
            if let Ok(full_artist) = self.spotify.artist(&artist_id).await {
                artist_name = full_artist.name;
            }
        } else {
            artist_name = String::from(input_artist_name);
        }
        let top_tracks = self.spotify.artist_top_tracks(&artist_id, country);
        let related_artist = self.spotify.artist_related_artists(&artist_id);

        if let Ok((albums, top_tracks, related_artist)) =
            try_join!(albums, top_tracks, related_artist)
        {
            let mut app = app.lock().await;
            app.artist = Some(Artist {
                artist_name,
                albums,
                related_artists: related_artist.artists,
                top_tracks: top_tracks.tracks,
                selected_album_index: 0,
                selected_related_artist_index: 0,
                selected_top_track_index: 0,
                artist_hovered_block: ArtistBlock::TopTracks,
                artist_selected_block: ArtistBlock::Empty,
            });
        }
    }

    async fn get_album_tracks(&mut self, app: &AppArc, album: SimplifiedAlbum) {
        if let Some(album_id) = &album.id {
            match self
                .spotify
                .album_track(&album_id.clone(), self.large_search_limit, 0)
                .await
            {
                Ok(tracks) => {
                    let track_ids = tracks
                        .items
                        .iter()
                        .filter_map(|item| item.id.clone())
                        .collect::<Vec<String>>();

                    self.current_user_saved_tracks_contains(app, track_ids)
                        .await;

                    let mut app = app.lock().await;
                    app.selected_album_simplified = Some(SelectedAlbum {
                        album,
                        tracks,
                        selected_index: 0,
                    });

                    app.album_table_context = AlbumTableContext::Simplified;
                    app.push_navigation_stack(RouteId::AlbumTracks, ActiveBlock::AlbumTracks);
                }
                Err(e) => {
                    self.handle_error(app, e).await;
                }
            }
        }
    }

    async fn get_recommendations_for_seed(
        &self,
        app: &AppArc,
        seed_artists: Option<Vec<String>>,
        seed_tracks: Option<Vec<String>>,
        first_track: Option<FullTrack>,
        country: Option<Country>,
    ) {
        let empty_payload: Map<String, Value> = Map::new();

        match self
            .spotify
            .recommendations(
                seed_artists,            // artists
                None,                    // genres
                seed_tracks,             // tracks
                self.large_search_limit, // adjust playlist to screen size
                country,                 // country
                &empty_payload,          // payload
            )
            .await
        {
            Ok(result) => {
                if let Some(mut recommended_tracks) = self.extract_recommended_tracks(&result).await
                {
                    //custom first track
                    if let Some(track) = first_track {
                        recommended_tracks.insert(0, track.clone());
                    }

                    let track_ids = recommended_tracks
                        .iter()
                        .map(|x| x.uri.clone())
                        .collect::<Vec<String>>();

                    self.set_tracks_to_table(app, recommended_tracks.clone())
                        .await;
                    self.start_playback(app, None, Some(track_ids), Some(0))
                        .await;

                    let mut app = app.lock().await;
                    app.recommended_tracks = recommended_tracks;
                    app.track_table.context = Some(TrackTableContext::RecommendedTracks);

                    if app.get_current_route().id != RouteId::Recommendations {
                        app.push_navigation_stack(
                            RouteId::Recommendations,
                            ActiveBlock::TrackTable,
                        );
                    };
                }
            }
            Err(e) => {
                self.handle_error(app, e).await;
            }
        }
    }

    async fn extract_recommended_tracks(
        &self,
        recommendations: &Recommendations,
    ) -> Option<Vec<FullTrack>> {
        let tracks = recommendations
            .clone()
            .tracks
            .into_iter()
            .map(|item| item.uri)
            .collect::<Vec<String>>();
        if let Ok(result) = self
            .spotify
            .tracks(tracks.iter().map(|x| &x[..]).collect::<Vec<&str>>(), None)
            .await
        {
            return Some(result.tracks);
        }

        None
    }

    async fn get_recommendations_for_track_id(
        &self,
        app: &AppArc,
        id: String,
        country: Option<Country>,
    ) {
        if let Some(track) = self.spotify.track(&id).await.ok() {
            let track_id_list: Option<Vec<String>> = match &track.id {
                Some(id) => Some(vec![id.to_string()]),
                None => None,
            };
            self.get_recommendations_for_seed(app, None, track_id_list, Some(track), country)
                .await;
        }
    }

    async fn toggle_save_track(&self, app: &AppArc, track_id: String) {
        match self
            .spotify
            .current_user_saved_tracks_contains(&[track_id.clone()])
            .await
        {
            Ok(saved) => {
                if saved.first() == Some(&true) {
                    match self
                        .spotify
                        .current_user_saved_tracks_delete(&[track_id.clone()])
                        .await
                    {
                        Ok(()) => {
                            let mut app = app.lock().await;
                            app.liked_song_ids_set.remove(&track_id);
                        }
                        Err(e) => {
                            self.handle_error(app, e).await;
                        }
                    }
                } else {
                    match self
                        .spotify
                        .current_user_saved_tracks_add(&[track_id.clone()])
                        .await
                    {
                        Ok(()) => {
                            // TODO: This should ideally use the same logic as `self.current_user_saved_tracks_contains`
                            let mut app = app.lock().await;
                            app.liked_song_ids_set.insert(track_id);
                        }
                        Err(e) => {
                            self.handle_error(app, e).await;
                        }
                    }
                }
            }
            Err(e) => {
                self.handle_error(app, e).await;
            }
        };
    }

    async fn get_followed_artists(&self, app: &AppArc, after: Option<String>) {
        match self
            .spotify
            .current_user_followed_artists(self.large_search_limit, after)
            .await
        {
            Ok(saved_artists) => {
                let mut app = app.lock().await;
                app.artists = saved_artists.artists.items.to_owned();
                app.library.saved_artists.add_pages(saved_artists.artists);
            }
            Err(e) => {
                self.handle_error(app, e).await;
            }
        };
    }

    async fn get_current_user_saved_albums(&self, app: &AppArc, offset: Option<u32>) {
        match self
            .spotify
            .current_user_saved_albums(self.large_search_limit, offset)
            .await
        {
            Ok(saved_albums) => {
                // not to show a blank page
                if !saved_albums.items.is_empty() {
                    let mut app = app.lock().await;
                    app.library.saved_albums.add_pages(saved_albums);
                }
            }
            Err(e) => {
                self.handle_error(app, e).await;
            }
        };
    }

    pub async fn current_user_saved_album_delete(&self, app: &AppArc, album_id: String) {
        match self
            .spotify
            .current_user_saved_albums_delete(&[album_id.to_owned()])
            .await
        {
            Ok(_) => {
                self.get_current_user_saved_albums(app, None).await;
            }
            Err(e) => {
                self.handle_error(app, e).await;
            }
        };
    }

    async fn current_user_saved_album_add(&self, app: &AppArc, artist_id: String) {
        if let Err(e) = self
            .spotify
            .current_user_saved_albums_add(&[artist_id.to_owned()])
            .await
        {
            self.handle_error(app, e).await;
        };
    }

    async fn user_unfollow_artists(&self, app: &AppArc, artist_ids: Vec<String>) {
        match self.spotify.user_unfollow_artists(&artist_ids).await {
            Ok(_) => {
                self.get_followed_artists(app, None).await;
            }
            Err(e) => {
                self.handle_error(app, e).await;
            }
        }
    }

    async fn user_follow_artists(&self, app: &AppArc, artist_ids: Vec<String>) {
        match self.spotify.user_follow_artists(&artist_ids).await {
            Ok(_) => {
                self.get_followed_artists(app, None).await;
            }
            Err(e) => {
                self.handle_error(app, e).await;
            }
        }
    }

    async fn user_follow_playlist(
        &self,
        app: &AppArc,
        playlist_owner_id: String,
        playlist_id: String,
        is_public: Option<bool>,
    ) {
        match self
            .spotify
            .user_playlist_follow_playlist(&playlist_owner_id, &playlist_id, is_public)
            .await
        {
            Ok(_) => {
                self.get_current_user_playlists(app).await;
            }
            Err(e) => {
                self.handle_error(app, e).await;
            }
        }
    }

    async fn user_unfollow_playlist(&self, app: &AppArc, user_id: String, playlist_id: String) {
        match self
            .spotify
            .user_playlist_unfollow(&user_id, &playlist_id)
            .await
        {
            Ok(_) => {
                self.get_current_user_playlists(app).await;
            }
            Err(e) => {
                self.handle_error(app, e).await;
            }
        }
    }

    async fn made_for_you_search_and_add(
        &self,
        app: &AppArc,
        search_string: String,
        country: Option<Country>,
    ) {
        const SPOTIFY_ID: &str = "spotify";

        match self
            .spotify
            .search_playlist(&search_string, self.large_search_limit, 0, country)
            .await
        {
            Ok(mut search_playlists) => {
                let mut filtered_playlists = search_playlists
                    .playlists
                    .items
                    .iter()
                    .filter(|playlist| {
                        playlist.owner.id == SPOTIFY_ID && playlist.name == search_string
                    })
                    .map(|playlist| playlist.to_owned())
                    .collect::<Vec<SimplifiedPlaylist>>();

                let mut app = app.lock().await;
                if !app.library.made_for_you_playlists.pages.is_empty() {
                    app.library
                        .made_for_you_playlists
                        .get_mut_results(None)
                        .unwrap()
                        .items
                        .append(&mut filtered_playlists);
                } else {
                    search_playlists.playlists.items = filtered_playlists;
                    app.library
                        .made_for_you_playlists
                        .add_pages(search_playlists.playlists);
                }
            }
            Err(e) => {
                self.handle_error(app, e).await;
            }
        }
    }

    async fn get_audio_analysis(&self, app: &AppArc, uri: String) {
        match self.spotify.audio_analysis(&uri).await {
            Ok(result) => {
                let mut app = app.lock().await;
                app.audio_analysis = Some(result);
                app.push_navigation_stack(RouteId::Analysis, ActiveBlock::Analysis);
            }
            Err(e) => {
                self.handle_error(app, e).await;
            }
        }
    }

    async fn get_current_user_playlists(&self, app: &AppArc) {
        let playlists = self
            .spotify
            .current_user_playlists(self.large_search_limit, None)
            .await;

        match playlists {
            Ok(p) => {
                let mut app = app.lock().await;
                app.playlists = Some(p);
                // Select the first playlist
                app.selected_playlist_index = Some(0);
            }
            Err(e) => {
                self.handle_error(app, e).await;
            }
        };
    }

    async fn get_recently_played(&self, app: &AppArc) {
        match self
            .spotify
            .current_user_recently_played(self.large_search_limit)
            .await
        {
            Ok(result) => {
                let track_ids = result
                    .items
                    .iter()
                    .filter_map(|item| item.track.id.clone())
                    .collect::<Vec<String>>();

                self.current_user_saved_tracks_contains(app, track_ids)
                    .await;

                let mut app = app.lock().await;

                app.recently_played.result = Some(result.clone());
                app.push_navigation_stack(RouteId::RecentlyPlayed, ActiveBlock::RecentlyPlayed);
            }
            Err(e) => {
                self.handle_error(app, e).await;
            }
        }
    }

    async fn get_album(&self, app: &AppArc, album_id: String) {
        match self.spotify.album(&album_id).await {
            Ok(album) => {
                let selected_album = SelectedFullAlbum {
                    album,
                    selected_index: 0,
                };

                let mut app = app.lock().await;

                app.selected_album_full = Some(selected_album);
                app.album_table_context = AlbumTableContext::Full;
                app.push_navigation_stack(RouteId::AlbumTracks, ActiveBlock::AlbumTracks);
            }
            Err(e) => {
                self.handle_error(app, e).await;
            }
        }
    }

    async fn set_device_id_in_config(&mut self, app: &AppArc, device_id: String) {
        match self.client_config.set_device_id(device_id) {
            Ok(()) => {
                let mut app = app.lock().await;
                app.pop_navigation_stack();
            }
            Err(e) => {
                self.handle_error(app, e).await;
            }
        };
    }
}
