use crate::ui;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use std::{
    convert::TryFrom,
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

pub const MIDDLE_MUSIC_INDEX: usize = 0;
pub const MIDDLE_PLAYLIST_INDEX: usize = 1;
pub const MIDDLE_ARTIST_INDEX: usize = 2;
const SEARCH_SH_KEY: char = '/';
const HELP_SH_KEY: char = '?';
const NEXT_SH_KEY: char = 'n';
const PREV_SH_KEY: char = 'p';
const QUIT_SH_KEY: char = 'c';
const SEEK_F_KEY: char = '>';
const SEEK_B_KEY: char = '<';
const TOGGLE_PAUSE_KEY: char = ' ';
const REFRESH_RATE: u64 = 950;

#[derive(Clone)]
enum HeadTo {
    Initial,
    Next,
    Prev,
}

// Helper function to return the index of something depending the current position and direction to
// move to
fn advance_index(current: usize, limit: usize, direction: HeadTo) -> usize {
    // This means that the list is empty.
    if limit == 0 {
        return 0;
    }
    match direction {
        HeadTo::Next => (current + 1) % limit,
        HeadTo::Prev => current.checked_sub(1).unwrap_or(limit - 1) % limit,
        HeadTo::Initial => current,
    }
}

// Helper function to drop the first paramater and call the function in second paramater and
// optional arguments provided in later arguments
// This is used to drop the state and call the function as such pattern is found redundant while
// calling event handeling closure where unlocked state needs to be droppped before calling the
// corresponding handler
macro_rules! drop_and_call {
    // This will call the function in passe in second argument
    // passed function will not accept any argument
    ($state: expr, $callback: expr) => {{
        std::mem::drop($state);
        $callback()
    }};
    // This will call the function recived in second argument and pass the later arguments as that
    // function paramater
    ($state: expr, $callback: expr, $($args: expr)*) => {{
        std::mem::drop($state);
        $callback( $($args)* )
    }};
}

// Heklper function to get the next page depending on the current page and direction to move
// This was mainly created to fetch the next page of the musicbar/playlist bar when user
// hits NEXT_SH_KEY or PREV_SH_KEY
#[inline]
fn get_page(current: &Option<usize>, direction: HeadTo) -> usize {
    let page = match current {
        None => 0,
        Some(prev) => match direction {
            HeadTo::Initial => 0,
            HeadTo::Next => prev + 1,
            HeadTo::Prev => prev.checked_sub(1).unwrap_or_default(),
        },
    };
    page as usize
}

/*
* The event_sender function is running in it's own seperate thread.
* -> A loop is initilized where it waits for any event to happen (keypress and resize for now)
* and call the corresponding closure to handle event.
* -> Inside every closure state that are dependent to this event is checked. eg: checks active
* window shile handleing left/right direction key
* -> To fetch data, required data paramater is set in a state variable which is shared across all
* the threads. And another loop is ran in communicator.rs where it wait checks weather anything
* should be filled from diffrenet source.
*/
pub fn event_sender(state_original: &mut Arc<Mutex<ui::State>>, notifier: &mut Arc<Condvar>) {
    // Some predefined source
    let youtube_community_channels = vec![fetcher::ArtistUnit {
        name: "Youtube Music Global Charts".to_string(),
        id: "UCrKZcyOJVWnJ60zM1XWllNw".to_string(),
        video_count: "NaN".to_string(),
    }];

    // There is several option in sidebar like trending/ favourates,
    // this handler will change the selected option from sidebar depending on the direction user
    // move (Up or DOwn).
    let advance_sidebar = |direction: HeadTo| {
        let mut state = state_original.lock().unwrap();
        let current = state.sidebar.selected().unwrap_or_default();
        state.sidebar.select(Some(advance_index(
            current,
            ui::utils::SIDEBAR_LIST_COUNT,
            direction,
        )));
        notifier.notify_all();
    };
    // select the next or previous element in musicbar list. This is done simply by setting the
    // correct index in corresponding TableState
    let advance_music_list = |direction: HeadTo| {
        let mut state = state_original.lock().unwrap();
        let next_index;
        match state.musicbar.1.selected() {
            None => next_index = 0,
            Some(current) => {
                next_index = advance_index(current, state.musicbar.0.len(), direction);
            }
        }
        state.musicbar.1.select(Some(next_index));
        notifier.notify_all();
    };
    // simialr to advance_music_list but instead rotate data in `playlistbar` variable of state
    let advance_playlist_list = |direction: HeadTo| {
        let mut state = state_original.lock().unwrap();
        let next_index;
        match state.playlistbar.1.selected() {
            None => next_index = 0,
            Some(current) => {
                next_index = advance_index(current, state.playlistbar.0.len(), direction);
            }
        }
        state.playlistbar.1.select(Some(next_index));
        notifier.notify_all();
    };
    // simialr to advance_playlist_list but instead rotate data in `artistbar` variable of state
    let advance_artist_list = |direction: HeadTo| {
        let mut state = state_original.lock().unwrap();
        // if the list is empty then do nothing else.
        // It is necessary to return instantly otherwise the next_index will get value 0 and this
        // closure will endup doing select(Some(0)) to the empty list
        let next_index;
        match state.artistbar.1.selected() {
            None => next_index = 0,
            Some(current) => {
                next_index = advance_index(current, state.artistbar.0.len(), direction);
            }
        }
        state.artistbar.1.select(Some(next_index));
        notifier.notify_all();
    };
    // When active window is set to NONE, it means user had requested to quit the application,
    // This handle will fire when user hits QUIT_SH_KEY
    // Before breaking the loop which this function is running on
    // this closure will simply set the active_window (`active`) to None so that functions in other
    // thread can also respond to the event (which is usally again breking the running loop in
    // thread)
    let quit = || {
        // setting active window to None is to quit
        state_original.lock().unwrap().active = ui::Window::None;
        notifier.notify_all();
    };
    // This handler will fire up when user request to move between sections like musicbar, sidebar
    // etc. Similar handler moveto_next_window / moveto_prev_window are not merged as these
    // closures as these handlers are frequently called so avoid more branching
    let moveto_next_window = || {
        let mut state = state_original.lock().unwrap();
        state.active = state.active.next();
        notifier.notify_all();
    };
    let moveto_prev_window = || {
        let mut state = state_original.lock().unwrap();
        state.active = state.active.prev();
        notifier.notify_all();
    };
    // This handler is fired when user press ESC key,
    // for now esc key just clear the content in search bar and move to next window
    let handle_esc = || {
        let mut state = state_original.lock().unwrap();
        if state.active == ui::Window::Searchbar {
            state.search.0.clear();
            drop_and_call!(state, moveto_next_window);
        }
    };
    // This handler is fired when user press BACKSPACE key
    // backspace key will pop the last character from search query if pressed from searchbar
    // and if this key is pressed from somewhere else other than searchbar then will simply
    // move to previous window
    let handle_backspace = || {
        let mut state = state_original.lock().unwrap();
        match state.active {
            ui::Window::Searchbar => {
                state.search.0.pop();
                notifier.notify_all();
            }
            _ => drop_and_call!(state, moveto_prev_window),
        }
    };
    // This is fires when user press any character key
    // this will simpley push the recived character in search query term and update state
    // so can the added character becomes visible
    let handle_search_input = |ch| {
        state_original.lock().unwrap().search.0.push(ch);
        notifier.notify_all();
    };
    // This handler is fired when use press SEARCH_SH_KEY
    // this will move the curson to the searchbar from which user can start to type the query
    let activate_search = || {
        let mut state = state_original.lock().unwrap();
        state.active = ui::Window::Searchbar;
        notifier.notify_all();
    };
    // This handler will be fired when use press HELP_SH_KEY
    // this will simply show the basic help on how to use this program including
    // shortcuts key
    let show_help = || {
        // TODO: create a new window that covers all the screen and remove the previous screen
        // If this became the source of hassale then remove this optionfrom
        // mod.rs: Window
        // mod.rs: Active
        // Enum Window: Help
        eprintln!("Show help");
    };
    // This handler will be fired when user hits UP_ARROW or DOWN_ARROW key
    // UP_ARROW will set the direction to PREV and DOWN_ARROW to NEXT
    // for now, these key will only handle the moving of list
    // So, depending on the window which is currently active, this closure will call
    // the respective handler which will advance the corersponding list
    let handle_up_down = |direction: HeadTo| {
        let state = state_original.lock().unwrap();
        match state.active {
            ui::Window::Sidebar => drop_and_call!(state, advance_sidebar, direction),
            ui::Window::Musicbar => drop_and_call!(state, advance_music_list, direction),
            ui::Window::Playlistbar => drop_and_call!(state, advance_playlist_list, direction),
            ui::Window::Artistbar => drop_and_call!(state, advance_artist_list, direction),
            _ => match direction {
                HeadTo::Next => drop_and_call!(state, moveto_next_window),
                HeadTo::Prev => drop_and_call!(state, moveto_prev_window),
                _ => unreachable!(),
            },
        }
    };
    let start_search = || {
        let mut state = state_original.lock().unwrap();
        state.search.1 = state.search.0.trim().to_string();
        state.fetched_page = [Some(0); 3];
        state.filled_source.0 = ui::MusicbarSource::Search(state.search.1.clone());
        state.filled_source.1 = ui::PlaylistbarSource::Search(state.search.1.clone());
        state.filled_source.2 = ui::ArtistbarSource::Search(state.search.1.clone());
        state.help = "Searching..";
        notifier.notify_all();
    };
    let fill_trending_music = |direction: HeadTo| {
        let mut state = state_original.lock().unwrap();
        state.fetched_page[MIDDLE_MUSIC_INDEX] =
            Some(get_page(&state.fetched_page[MIDDLE_MUSIC_INDEX], direction));
        state.filled_source.0 = ui::MusicbarSource::Trending;
        state.help = "Fetching..";
        notifier.notify_all();
    };
    let fill_community_source = || {
        let mut state = state_original.lock().unwrap();
        state.artistbar.0 = youtube_community_channels.clone();
        state.active = ui::Window::Artistbar;
        notifier.notify_all();
    };
    let fill_recents_music = |_direction: HeadTo| {};
    let fill_favourates_music = |_direction: HeadTo| {};
    let fill_favourates_artist = |_direction: HeadTo| {};
    let fill_music_from_playlist = |direction: HeadTo| {
        let mut state = state_original.lock().unwrap();
        if let ui::MusicbarSource::Playlist(playlist_id) = &state.filled_source.0 {
            state.filled_source.0 = ui::MusicbarSource::Playlist(playlist_id.to_string());
            state.fetched_page[MIDDLE_MUSIC_INDEX] =
                Some(get_page(&state.fetched_page[MIDDLE_MUSIC_INDEX], direction));
            state.help = "Fetching playlist..";
            notifier.notify_all();
        }
    };
    let fill_music_from_artist = |direction: HeadTo| {
        let mut state = state_original.lock().unwrap();
        if let ui::MusicbarSource::Artist(artist_id) = &state.filled_source.0 {
            state.filled_source.0 = ui::MusicbarSource::Artist(artist_id.to_string());
            state.fetched_page[MIDDLE_MUSIC_INDEX] =
                Some(get_page(&state.fetched_page[MIDDLE_MUSIC_INDEX], direction));
            state.help = "Fetching channel..";
            notifier.notify_all();
        }
    };
    let fill_playlist_from_artist = |direction: HeadTo| {
        let mut state = state_original.lock().unwrap();
        if let ui::PlaylistbarSource::Artist(artist_id) = &state.filled_source.1 {
            state.filled_source.1 = ui::PlaylistbarSource::Artist(artist_id.to_string());
            state.fetched_page[MIDDLE_PLAYLIST_INDEX] = Some(get_page(
                &state.fetched_page[MIDDLE_PLAYLIST_INDEX],
                direction,
            ));
            state.help = "Fetching channel..";
            notifier.notify_all();
        }
    };
    // play next/previous song from queue
    let handle_play_advance = |_direction: HeadTo| {};

    // navigating page is just changing to fetched_page value to next/prev value
    let handle_page_nav = |direction: HeadTo| {
        let mut state = state_original.lock().unwrap();
        let target_index: usize;
        match state.active {
            ui::Window::Musicbar => target_index = MIDDLE_MUSIC_INDEX,
            ui::Window::Playlistbar => target_index = MIDDLE_PLAYLIST_INDEX,
            ui::Window::Artistbar => target_index = MIDDLE_ARTIST_INDEX,
            _ => {
                // If none os above windows are active then nothing to navigate.
                // Early return instead of initilizing `target_index`
                return;
            }
        }
        let page = get_page(&state.fetched_page[target_index], direction);
        state.fetched_page[target_index] = Some(page);
        notifier.notify_all();
    };
    let handle_enter = || {
        let mut state = state_original.lock().unwrap();
        let active_window = state.active.clone();
        match active_window {
            ui::Window::Sidebar => {
                let side_select =
                    ui::SidebarOption::try_from(state.sidebar.selected().unwrap()).unwrap();

                match side_select {
                    ui::SidebarOption::Trending => {
                        drop_and_call!(state, fill_trending_music, HeadTo::Initial);
                    }
                    ui::SidebarOption::YoutubeCommunity => {
                        drop_and_call!(state, fill_community_source);
                    }
                    ui::SidebarOption::Favourates => {
                        drop_and_call!(state, fill_favourates_music, HeadTo::Initial);
                    }
                    ui::SidebarOption::RecentlyPlayed => {
                        drop_and_call!(state, fill_recents_music, HeadTo::Initial);
                    }
                    ui::SidebarOption::Search => drop_and_call!(state, activate_search),
                    ui::SidebarOption::None => {}
                }
            }
            ui::Window::Searchbar => {
                drop_and_call!(state, start_search);
            }
            ui::Window::Musicbar => {
                if let Some(selected_index) = state.musicbar.1.selected() {
                    let music_id = state.musicbar.0[selected_index].id.clone();
                    state.play_music(&music_id);
                }
            }
            ui::Window::Playlistbar => {
                if let Some(selected_index) = state.playlistbar.1.selected() {
                    let playlist_id = state.playlistbar.0[selected_index].id.clone();
                    state.activate_playlist(&playlist_id);
                    state.filled_source.0 = ui::MusicbarSource::Playlist(playlist_id);
                    drop_and_call!(state, fill_music_from_playlist, HeadTo::Initial);
                    notifier.notify_all();
                }
            }
            ui::Window::Artistbar => {
                if let Some(selected_index) = state.artistbar.1.selected() {
                    let artist_id = state.artistbar.0[selected_index].id.clone();
                    state.filled_source.0 = ui::MusicbarSource::Artist(artist_id.clone());
                    state.filled_source.1 = ui::PlaylistbarSource::Artist(artist_id);
                    std::mem::drop(state);
                    fill_music_from_artist(HeadTo::Initial);
                    fill_playlist_from_artist(HeadTo::Initial);
                }
            }
            ui::Window::None | ui::Window::Helpbar => {}
        }
    };

    'listener_loop: loop {
        if event::poll(Duration::from_millis(REFRESH_RATE)).unwrap() {
            match event::read().unwrap() {
                Event::Key(key) => {
                    let is_with_control = key.modifiers.contains(KeyModifiers::CONTROL);
                    match key.code {
                        KeyCode::Down | KeyCode::PageDown => {
                            handle_up_down(HeadTo::Next);
                        }
                        KeyCode::Up | KeyCode::PageUp => {
                            handle_up_down(HeadTo::Prev);
                        }
                        KeyCode::Right | KeyCode::Tab => {
                            moveto_next_window();
                        }
                        KeyCode::Left | KeyCode::BackTab => {
                            moveto_prev_window();
                        }
                        KeyCode::Esc => {
                            handle_esc();
                        }
                        KeyCode::Enter => {
                            handle_enter();
                        }
                        KeyCode::Backspace | KeyCode::Delete => {
                            handle_backspace();
                        }
                        KeyCode::Char(ch) => {
                            /* If searchbar is active register every char key as input term */
                            if state_original.lock().unwrap().active == ui::Window::Searchbar {
                                handle_search_input(ch);
                            }
                            /* Handle single character key shortcut as it is not in input */
                            else if ch == SEARCH_SH_KEY {
                                activate_search();
                            } else if ch == HELP_SH_KEY {
                                show_help();
                            } else if ch == QUIT_SH_KEY && is_with_control {
                                quit();
                                break 'listener_loop;
                            } else if ch == NEXT_SH_KEY {
                                if is_with_control {
                                    handle_play_advance(HeadTo::Next);
                                } else {
                                    handle_page_nav(HeadTo::Next);
                                }
                            } else if ch == PREV_SH_KEY {
                                if is_with_control {
                                    handle_play_advance(HeadTo::Prev);
                                } else {
                                    handle_page_nav(HeadTo::Prev);
                                }
                            } else if ch == SEEK_F_KEY {
                            } else if ch == SEEK_B_KEY {
                            } else if ch == TOGGLE_PAUSE_KEY {
                                state_original.lock().unwrap().toggle_pause(notifier);
                            }
                        }
                        _ => {}
                    }
                }
                Event::Resize(..) => {
                    // just update the layout
                    notifier.notify_all();
                }
                Event::Mouse(..) => {}
            }
        } else {
            notifier.notify_all();
        }
    }
}
