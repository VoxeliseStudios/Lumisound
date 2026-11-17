//! Library Hub (the Library tab's home screen): every shelf in one pass over
//! the library — recently added / played, most played, forgotten favourites,
//! genres, top artists, decades, deeper cuts, the weekly recap — plus tile
//! collages and listened fractions. Play counts and last-played dates come
//! from the core's play history store. Returns song ids; Swift maps them back
//! to its `Song` values. (Was `HubContentBuilder` in Swift.)

use crate::store::now;
use std::collections::{HashMap, HashSet};

#[derive(uniffi::Record, Clone, Debug)]
pub struct HubSong {
    pub id: String,
    /// Raw artist tag (may be empty).
    pub artist: String,
    /// Display artist ("Unknown Artist" when the tag is empty).
    pub artist_name: String,
    pub genre: String,
    pub year: String,
    pub duration: f64,
    /// Unix seconds.
    pub date_added: Option<f64>,
    /// Album name used for grouping (collage variety).
    pub album_key: String,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct HubGroup {
    pub name: String,
    pub song_ids: Vec<String>,
    /// Lifetime plays across the group (top artists); 0 elsewhere.
    pub play_count: i64,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct HubRecap {
    pub songs_played: i64,
    pub estimated_minutes: i64,
    pub top_artist: Option<String>,
}

#[derive(uniffi::Record, Clone, Debug, Default, PartialEq)]
pub struct HubSnapshot {
    pub recently_added: Vec<String>,
    pub most_played: Vec<String>,
    pub forgotten_favorites: Vec<String>,
    pub recently_played: Vec<String>,
    pub genre_groups: Vec<HubGroup>,
    pub top_artist_groups: Vec<HubGroup>,
    pub decade_groups: Vec<HubGroup>,
    pub deeper_cuts: Vec<String>,
    pub weekly_recap: Option<HubRecap>,
}

/// Limits per shelf (the Swift defaults).
#[derive(uniffi::Record, Clone, Debug)]
pub struct HubLimits {
    pub songs: u32,
    pub genres: u32,
    pub artists: u32,
    pub decades: u32,
}

struct History(HashMap<String, (i64, Option<f64>)>);

impl History {
    fn load() -> Self {
        History(crate::stores::play_history_all().into_iter().map(|r| (r.song_id, (r.play_count, r.last_played_at))).collect())
    }
    fn count(&self, id: &str) -> i64 {
        self.0.get(id).map(|h| h.0).unwrap_or(0)
    }
    fn last(&self, id: &str) -> Option<f64> {
        self.0.get(id).and_then(|h| h.1)
    }
}

const DAY: f64 = 86_400.0;

fn ids<'a>(it: impl Iterator<Item = &'a HubSong>, limit: usize) -> Vec<String> {
    it.take(limit).map(|s| s.id.clone()).collect()
}

/// Groups in first-seen order of their key, with the songs in library order.
fn bucket<K: std::hash::Hash + Eq + Clone>(songs: &[HubSong], key: impl Fn(&HubSong) -> Option<K>) -> Vec<(K, Vec<&HubSong>)> {
    let mut index: HashMap<K, usize> = HashMap::new();
    let mut out: Vec<(K, Vec<&HubSong>)> = Vec::new();
    for s in songs {
        let Some(k) = key(s) else { continue };
        let i = *index.entry(k.clone()).or_insert_with(|| {
            out.push((k, Vec::new()));
            out.len() - 1
        });
        out[i].1.push(s);
    }
    out
}

/// The first item with the largest key (ties keep the earliest).
fn first_max<T>(it: impl Iterator<Item = T>, key: impl Fn(&T) -> i64) -> Option<T> {
    let mut best: Option<(i64, T)> = None;
    for x in it {
        let k = key(&x);
        if best.as_ref().is_none_or(|b| k > b.0) {
            best = Some((k, x));
        }
    }
    best.map(|b| b.1)
}

fn xorshift_shuffle<T>(v: &mut [T]) {
    let mut seed = (now() * 1e6) as u64 | 1;
    for i in (1..v.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        v.swap(i, (seed % (i as u64 + 1)) as usize);
    }
}

fn snapshot_with(songs: &[HubSong], favorites: &HashSet<String>, h: &History, t: f64, lim: &HubLimits) -> HubSnapshot {
    let n = lim.songs as usize;
    let by_desc = |a: f64, b: f64| b.partial_cmp(&a).unwrap_or(std::cmp::Ordering::Equal);

    let mut added: Vec<&HubSong> = songs.iter().collect();
    added.sort_by(|a, b| by_desc(a.date_added.unwrap_or(f64::MIN), b.date_added.unwrap_or(f64::MIN)));

    let mut most: Vec<(&HubSong, i64)> = songs.iter().map(|s| (s, h.count(&s.id))).filter(|x| x.1 > 0).collect();
    most.sort_by_key(|x| std::cmp::Reverse(x.1));

    // Favourites not played for 60 days (or ever), stalest first.
    let cutoff = t - 60.0 * DAY;
    let mut forgotten: Vec<&HubSong> = songs
        .iter()
        .filter(|s| favorites.contains(&s.id) && h.last(&s.id).unwrap_or(f64::MIN) < cutoff)
        .collect();
    forgotten.sort_by(|a, b| h.last(&a.id).unwrap_or(f64::MIN).partial_cmp(&h.last(&b.id).unwrap_or(f64::MIN)).unwrap_or(std::cmp::Ordering::Equal));

    let mut played: Vec<(&HubSong, f64)> = songs.iter().filter_map(|s| h.last(&s.id).map(|l| (s, l))).collect();
    played.sort_by(|a, b| by_desc(a.1, b.1));

    let genres = bucket(songs, |s| Some(s.genre.trim().to_string()).filter(|g| !g.is_empty()));
    let mut genre_groups: Vec<HubGroup> = genres
        .iter()
        .map(|(g, ss)| HubGroup { name: g.clone(), song_ids: ss.iter().map(|s| s.id.clone()).collect(), play_count: 0 })
        .collect();
    genre_groups.sort_by_key(|g| std::cmp::Reverse(g.song_ids.len()));
    genre_groups.truncate(lim.genres as usize);

    let mut artists: Vec<HubGroup> = bucket(songs, |s| Some(s.artist.trim().to_string()).filter(|a| !a.is_empty()))
        .into_iter()
        .map(|(a, ss)| HubGroup {
            name: a,
            play_count: ss.iter().map(|s| h.count(&s.id)).sum(),
            song_ids: ss.iter().map(|s| s.id.clone()).collect(),
        })
        .filter(|g| g.play_count > 0)
        .collect();
    artists.sort_by_key(|g| std::cmp::Reverse(g.play_count));
    artists.truncate(lim.artists as usize);

    let mut decades: Vec<(i64, Vec<&HubSong>)> = bucket(songs, |s| {
        s.year.trim().parse::<i64>().ok().filter(|y| (1900..=2100).contains(y)).map(|y| y / 10 * 10)
    });
    decades.sort_by_key(|d| std::cmp::Reverse(d.0));
    let decade_groups = decades
        .into_iter()
        .take(lim.decades as usize)
        .map(|(d, ss)| HubGroup { name: format!("{d}s"), song_ids: ss.iter().map(|s| s.id.clone()).collect(), play_count: 0 })
        .collect();

    // Unplayed songs from the genre with the most lifetime plays.
    let deeper_cuts = {
        let top = first_max(
            genres.iter().map(|(g, ss)| (g, ss, ss.iter().map(|s| h.count(&s.id)).sum::<i64>())).filter(|x| x.2 > 0),
            |x| x.2,
        );
        match top {
            Some((_, ss, _)) => {
                let mut unplayed: Vec<&HubSong> = ss.iter().copied().filter(|s| h.count(&s.id) == 0).collect();
                xorshift_shuffle(&mut unplayed);
                ids(unplayed.into_iter(), n)
            }
            None => vec![],
        }
    };

    // This week: distinct songs last played in the past 7 days.
    let week_cutoff = t - 7.0 * DAY;
    let this_week: Vec<&HubSong> = songs.iter().filter(|s| h.last(&s.id).is_some_and(|l| l >= week_cutoff)).collect();
    let weekly_recap = (!this_week.is_empty()).then(|| {
        let total: f64 = this_week.iter().map(|s| s.duration).sum();
        let mut counts: Vec<(&str, i64)> = Vec::new();
        for s in &this_week {
            match counts.iter_mut().find(|c| c.0 == s.artist_name) {
                Some(c) => c.1 += 1,
                None => counts.push((&s.artist_name, 1)),
            }
        }
        let top = first_max(counts.iter(), |c| c.1).map(|c| c.0.to_string());
        HubRecap { songs_played: this_week.len() as i64, estimated_minutes: (total / 60.0) as i64, top_artist: top }
    });

    HubSnapshot {
        recently_added: ids(added.into_iter(), n),
        most_played: ids(most.into_iter().map(|x| x.0), n),
        forgotten_favorites: ids(forgotten.into_iter(), n),
        recently_played: ids(played.into_iter().map(|x| x.0), n),
        genre_groups,
        top_artist_groups: artists,
        decade_groups,
        deeper_cuts,
        weekly_recap,
    }
}

/// Every Library Hub shelf in one call.
#[uniffi::export]
pub fn hub_snapshot(songs: Vec<HubSong>, favorite_ids: Vec<String>, limits: HubLimits) -> HubSnapshot {
    let favs: HashSet<String> = favorite_ids.into_iter().collect();
    snapshot_with(&songs, &favs, &History::load(), now(), &limits)
}

#[derive(uniffi::Enum, Clone, Copy, Debug)]
pub enum HubShelf {
    RecentlyAdded,
    MostPlayed,
    RecentlyPlayed,
}

/// One song shelf on its own (cheaper than a whole snapshot).
#[uniffi::export]
pub fn hub_shelf(songs: Vec<HubSong>, shelf: HubShelf, limit: u32) -> Vec<String> {
    let h = History::load();
    let n = limit as usize;
    match shelf {
        HubShelf::RecentlyAdded => {
            let mut v: Vec<&HubSong> = songs.iter().collect();
            v.sort_by(|a, b| b.date_added.unwrap_or(f64::MIN).partial_cmp(&a.date_added.unwrap_or(f64::MIN)).unwrap_or(std::cmp::Ordering::Equal));
            ids(v.into_iter(), n)
        }
        HubShelf::MostPlayed => {
            let mut v: Vec<(&HubSong, i64)> = songs.iter().map(|s| (s, h.count(&s.id))).filter(|x| x.1 > 0).collect();
            v.sort_by_key(|x| std::cmp::Reverse(x.1));
            ids(v.into_iter().map(|x| x.0), n)
        }
        HubShelf::RecentlyPlayed => {
            let mut v: Vec<(&HubSong, f64)> = songs.iter().filter_map(|s| h.last(&s.id).map(|l| (s, l))).collect();
            v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            ids(v.into_iter().map(|x| x.0), n)
        }
    }
}

/// Up to `limit` songs for a tile collage, preferring distinct albums.
#[uniffi::export]
pub fn hub_collage(songs: Vec<HubSong>, limit: u32) -> Vec<String> {
    let limit = limit as usize;
    if songs.len() <= limit {
        return songs.into_iter().map(|s| s.id).collect();
    }
    let mut seen = HashSet::new();
    let mut picked: Vec<String> = Vec::with_capacity(limit);
    for s in &songs {
        if seen.insert(s.album_key.as_str()) {
            picked.push(s.id.clone());
            if picked.len() == limit {
                return picked;
            }
        }
    }
    for s in &songs {
        if picked.len() == limit {
            break;
        }
        if !picked.contains(&s.id) {
            picked.push(s.id.clone());
        }
    }
    picked
}

/// Share (0-1) of these songs played at least once; None for no songs.
#[uniffi::export]
pub fn hub_listened_fraction(song_ids: Vec<String>) -> Option<f64> {
    if song_ids.is_empty() {
        return None;
    }
    let h = History::load();
    let played = song_ids.iter().filter(|id| h.count(id) > 0).count();
    Some(played as f64 / song_ids.len() as f64)
}

/// Songs in the library last played today (local time).
#[uniffi::export]
pub fn hub_songs_played_today(library_ids: Vec<String>) -> u32 {
    use chrono::{Local, TimeZone};
    let today = Local::now().date_naive();
    let lib: HashSet<String> = library_ids.into_iter().collect();
    crate::stores::play_history_all()
        .into_iter()
        .filter(|r| lib.contains(&r.song_id))
        .filter_map(|r| r.last_played_at)
        .filter(|t| Local.timestamp_opt(*t as i64, 0).single().is_some_and(|d| d.date_naive() == today))
        .count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(id: &str, artist: &str, genre: &str, year: &str, added: Option<f64>, album: &str) -> HubSong {
        HubSong {
            id: id.into(),
            artist: artist.into(),
            artist_name: if artist.trim().is_empty() { "Unknown Artist".into() } else { artist.into() },
            genre: genre.into(),
            year: year.into(),
            duration: 180.0,
            date_added: added,
            album_key: album.into(),
        }
    }

    #[test]
    fn shelves() {
        let t = 1_000_000_000.0;
        let songs = vec![
            song("a", "X", "Rock", "1994", Some(t - 10.0), "A1"),
            song("b", "X", "Rock", "1999", Some(t - 5.0), "A1"),
            song("c", "Y", "Jazz", "2012", None, "A2"),
            song("d", "", " ", "abc", Some(t - 1.0), "A3"),
            song("e", "Y", "Rock", "1890", Some(t - 2.0), "A4"),
        ];
        let h = History(HashMap::from([
            ("a".to_string(), (5, Some(t - 3.0 * DAY))),
            ("c".to_string(), (2, Some(t - 100.0 * DAY))),
            ("d".to_string(), (1, Some(t - 1.0))),
        ]));
        let favs: HashSet<String> = ["c", "b"].iter().map(|s| s.to_string()).collect();
        let lim = HubLimits { songs: 20, genres: 12, artists: 12, decades: 10 };
        let s = snapshot_with(&songs, &favs, &h, t, &lim);
        assert_eq!(s.recently_added, vec!["d", "e", "b", "a", "c"]);
        assert_eq!(s.most_played, vec!["a", "c", "d"]);
        assert_eq!(s.forgotten_favorites, vec!["b", "c"]); // b never played, c 100 days ago
        assert_eq!(s.recently_played, vec!["d", "a", "c"]);
        assert_eq!(s.genre_groups.iter().map(|g| (g.name.as_str(), g.song_ids.len())).collect::<Vec<_>>(), vec![("Rock", 3), ("Jazz", 1)]);
        assert_eq!(s.top_artist_groups.iter().map(|g| (g.name.as_str(), g.play_count)).collect::<Vec<_>>(), vec![("X", 5), ("Y", 2)]);
        assert_eq!(s.decade_groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), vec!["2010s", "1990s"]);
        assert_eq!(s.decade_groups[1].song_ids, vec!["a", "b"]);
        let mut cuts = s.deeper_cuts.clone();
        cuts.sort();
        assert_eq!(cuts, vec!["b", "e"]); // Rock's unplayed songs
        // a (X) and d (Unknown Artist) tie at one song each; the first seen wins.
        assert_eq!(s.weekly_recap, Some(HubRecap { songs_played: 2, estimated_minutes: 6, top_artist: Some("X".into()) }));
    }

    #[test]
    fn collage_prefers_distinct_albums() {
        let songs: Vec<HubSong> = ["1", "2", "3", "4", "5", "6"]
            .iter()
            .zip(["A", "A", "B", "B", "C", "D"])
            .map(|(id, al)| song(id, "x", "", "", None, al))
            .collect();
        assert_eq!(hub_collage(songs.clone(), 4), vec!["1", "3", "5", "6"]);
        assert_eq!(hub_collage(songs[..3].to_vec(), 4), vec!["1", "2", "3"]);
        let same: Vec<HubSong> = songs.iter().cloned().map(|mut s| { s.album_key = "Z".into(); s }).collect();
        assert_eq!(hub_collage(same, 4), vec!["1", "2", "3", "4"]);
    }
}
