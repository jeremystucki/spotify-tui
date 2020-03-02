use crate::app::{
    ActiveBlock, AlbumTableContext, App, Artist, ArtistBlock, RouteId, SelectedAlbum,
    TrackTableContext,
};
use crate::config::ClientConfig;
use rspotify::{
    client::Spotify,
    model::{
        album::{FullAlbum, SavedAlbum, SimplifiedAlbum},
        artist::FullArtist,
        audio::AudioAnalysis,
        context::FullPlayingContext,
        device::DevicePayload,
        offset::for_position,
        page::{CursorBasedPage, Page},
        playing::PlayHistory,
        playlist::{PlaylistTrack, SimplifiedPlaylist},
        recommend::Recommendations,
        search::{SearchAlbums, SearchArtists, SearchPlaylists, SearchTracks},
        track::{FullTrack, SavedTrack, SimplifiedTrack},
        user::PrivateUser,
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
    CurrentUserSavedTracksContains(Vec<String>),
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
                        self.handle_error(app, e);
                    }
                };

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
                self.change_volume(&app, volume);
            }
            IoEvent::GetArtist(artist_id, input_artist_name, country) => {
                self.get_artist(&app, artist_id, input_artist_name, country)
                    .await;
            }
            IoEvent::CurrentUserSavedTracksContains(ids) => {
                self.current_user_saved_tracks_contains(&app, ids).await;
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
        };

        let mut app = app.lock().await;
        app.is_loading = false;
    }

    async fn handle_error(&mut self, app: &AppArc, e: failure::Error) {
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
                self.handle_error(app, e);
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
                self.handle_error(app, e);
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
                self.handle_error(app, e);
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
                self.handle_error(app, e);
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
                self.get_current_playback(app);

                let mut app = app.lock().await;
                app.song_progress_ms = 0;
            }
            Err(e) => {
                self.handle_error(app, e);
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
                    self.get_current_playback(app);
                }
                Err(e) => {
                    self.handle_error(app, e);
                }
            };
        }
    }

    async fn next_track(&self, app: &AppArc) {
        match self.spotify.next_track(self.client_config.device_id).await {
            Ok(()) => {
                self.get_current_playback(app);
            }
            Err(e) => {
                self.handle_error(app, e);
            }
        };
    }

    async fn previous_track(&self, app: &AppArc) {
        match self
            .spotify
            .previous_track(self.client_config.device_id)
            .await
        {
            Ok(()) => {
                self.get_current_playback(app).await;
            }
            Err(e) => {
                self.handle_error(app, e);
            }
        };
    }

    async fn shuffle(&self, app: &AppArc, shuffle_state: bool) {
        match self
            .spotify
            .shuffle(!shuffle_state, self.client_config.device_id)
            .await
        {
            Ok(()) => {
                // Update the UI eagerly (otherwise the UI will wait until the next 5 second interval
                // due to polling playback context)
                let mut app = app.lock().await;
                if let Some(current_playback_context) = app.current_playback_context {
                    current_playback_context.shuffle_state = !shuffle_state;
                };
            }
            Err(e) => {
                self.handle_error(app, e);
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
            .repeat(next_repeat_state, self.client_config.device_id)
            .await
        {
            Ok(()) => {
                let mut app = app.lock().await;
                if let Some(current_playback_context) = app.current_playback_context {
                    current_playback_context.repeat_state = next_repeat_state;
                };
            }
            Err(e) => {
                self.handle_error(app, e);
            }
        };
    }

    async fn pause_playback(&self, app: &AppArc) {
        match self
            .spotify
            .pause_playback(self.client_config.device_id)
            .await
        {
            Ok(()) => {
                self.get_current_playback(app);
            }
            Err(e) => {
                self.handle_error(app, e);
            }
        };
    }

    async fn change_volume(&self, app: &AppArc, volume_percent: u8) {
        match self
            .spotify
            .volume(volume_percent, self.client_config.device_id)
            .await
        {
            Ok(()) => {
                let mut app = app.lock().await;
                if let Some(current_playback_context) = app.current_playback_context {
                    current_playback_context.device.volume_percent = volume_percent.into();
                };
            }
            Err(e) => {
                self.handle_error(app, e);
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
                self.handle_error(app, e);
            }
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
                    self.current_user_saved_tracks_contains(
                        app,
                        tracks
                            .items
                            .into_iter()
                            .filter_map(|item| item.id)
                            .collect::<Vec<String>>(),
                    )
                    .await;

                    let mut app = app.lock().await;
                    app.selected_album_simplified = Some(SelectedAlbum {
                        album,
                        tracks: tracks.clone(),
                        selected_index: 0,
                    });

                    app.album_table_context = AlbumTableContext::Simplified;
                    app.push_navigation_stack(RouteId::AlbumTracks, ActiveBlock::AlbumTracks);
                }
                Err(e) => {
                    self.handle_error(app, e);
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
                if let Some(mut recommended_tracks) =
                    self.extract_recommended_tracks(app, &result).await
                {
                    //custom first track
                    if let Some(track) = first_track {
                        recommended_tracks.insert(0, track.clone());
                    }
                    self.set_tracks_to_table(app, recommended_tracks).await;

                    self.start_playback(
                        app,
                        None,
                        Some(
                            recommended_tracks
                                .iter()
                                .map(|x| x.uri.clone())
                                .collect::<Vec<String>>(),
                        ),
                        Some(0),
                    )
                    .await;

                    let mut app = app.lock().await;
                    app.recommended_tracks = recommended_tracks.clone();
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
                self.handle_error(app, e);
            }
        }
    }

    async fn extract_recommended_tracks(
        &self,
        app: &AppArc,
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
        id: &str,
        country: Option<Country>,
    ) {
        if let Some(track) = self.spotify.track(id).await.ok() {
            let track_id_list: Option<Vec<String>> = match &track.id {
                Some(id) => Some(vec![id.to_string()]),
                None => None,
            };
            self.get_recommendations_for_seed(app, None, track_id_list, Some(track), country);
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
                            self.handle_error(app, e);
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
                            self.handle_error(app, e);
                        }
                    }
                }
            }
            Err(e) => {
                self.handle_error(app, e);
            }
        };
    }

    async fn get_artists(&self, app: &AppArc, offset: Option<String>) {
        match self
            .spotify
            .current_user_followed_artists(self.large_search_limit, offset)
            .await
        {
            Ok(saved_artists) => {
                let mut app = app.lock().await;
                app.artists = saved_artists.artists.items.to_owned();
                app.library.saved_artists.add_pages(saved_artists.artists);
            }
            Err(e) => {
                self.handle_error(app, e);
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
                self.handle_error(app, e);
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
                self.handle_error(app, e);
            }
        };
    }

    async fn current_user_saved_album_add(&self, app: &AppArc, artist_id: String) {
        if let Err(e) = self
            .spotify
            .current_user_saved_albums_add(&[artist_id.to_owned()])
            .await
        {
            self.handle_error(app, e);
        };
    }
}
