//! Fuzzy search and type filters for the clip list.
//!
//! Every word of the query must appear in a clip, either as a plain substring
//! (best) or as its letters in order within a short stretch, so "inv pd"
//! finds "Invoice paid". Matches at the start of a word score higher.

/// Characters of a clip's text that are searched. Longer clips are searched
/// by their beginning, which keeps typing fast with thousands of clips.
pub const SEARCH_CHARS: usize = 4096;

/// Which kinds of clip the list shows. Tab in the popup cycles through these.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Text,
    Links,
    Images,
    Files,
    Colors,
    Code,
    Pinned,
}

impl Filter {
    /// In Tab order; the popup uses the same order for its labels.
    pub const ALL: [Filter; 8] = [
        Filter::All,
        Filter::Text,
        Filter::Links,
        Filter::Images,
        Filter::Files,
        Filter::Colors,
        Filter::Code,
        Filter::Pinned,
    ];

    pub fn from_index(i: i32) -> Filter {
        Self::ALL.get(i.max(0) as usize).copied().unwrap_or_default()
    }

    pub fn index(self) -> i32 {
        Self::ALL.iter().position(|f| *f == self).unwrap_or(0) as i32
    }

    /// A SQL condition on the `clips` table, aliased `c`.
    pub(crate) fn sql(self) -> &'static str {
        match self {
            Filter::All => "1",
            Filter::Text => "c.kind = 'text'",
            Filter::Links => "c.kind = 'url'",
            Filter::Images => "c.kind = 'image'",
            Filter::Files => "c.kind = 'files'",
            Filter::Colors => "c.kind = 'color'",
            Filter::Code => "c.kind = 'code'",
            Filter::Pinned => "c.pinned = 1",
        }
    }
}

/// A parsed search query.
pub struct Query {
    terms: Vec<Vec<char>>,
}

impl Query {
    pub fn new(query: &str) -> Query {
        let terms = query
            .split_whitespace()
            .map(|t| t.chars().flat_map(char::to_lowercase).collect::<Vec<_>>())
            .filter(|t| !t.is_empty())
            .collect();
        Query { terms }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// Score `haystack` against the query; `None` when a word doesn't match.
    pub fn score(&self, haystack: &str) -> Option<u32> {
        if self.terms.is_empty() {
            return Some(0);
        }
        let hay: Vec<char> = haystack.chars().take(SEARCH_CHARS).flat_map(char::to_lowercase).collect();
        self.terms.iter().try_fold(0u32, |sum, t| term_score(&hay, t).map(|s| sum + s))
    }
}

fn is_word_start(hay: &[char], i: usize) -> bool {
    i == 0 || !hay[i - 1].is_alphanumeric()
}

fn term_score(hay: &[char], term: &[char]) -> Option<u32> {
    let n = term.len();
    if n > hay.len() {
        return None;
    }

    // Plain substring: the strongest kind of match.
    let mut best: Option<u32> = None;
    for i in 0..=hay.len() - n {
        if hay[i..i + n] == *term {
            let mut s = 1000 + 40 * n as u32;
            if is_word_start(hay, i) {
                s += 200;
            }
            if i == 0 {
                s += 100;
            }
            if i + n == hay.len() || !hay[i + n].is_alphanumeric() {
                s += 50;
            }
            s -= (i as u32).min(100);
            best = best.max(Some(s));
        }
    }
    if best.is_some() {
        return best;
    }

    // Letters in order. Find a match window, shrink it from the right end
    // back to the latest possible start, score it, then look for a tighter
    // window further on (a few times at most).
    let max_span = n * 3 + 4;
    let mut from = 0;
    for _ in 0..8 {
        let mut ti = 0;
        let mut end = None;
        for (i, &c) in hay.iter().enumerate().skip(from) {
            if c == term[ti] {
                ti += 1;
                if ti == n {
                    end = Some(i);
                    break;
                }
            }
        }
        let Some(end) = end else { break };
        let mut ti = n - 1;
        let mut start = end;
        for i in (from..=end).rev() {
            if hay[i] == term[ti] {
                if ti == 0 {
                    start = i;
                    break;
                }
                ti -= 1;
            }
        }
        if end - start < max_span {
            best = best.max(Some(window_score(hay, term, start, end)));
        }
        from = start + 1;
    }
    best
}

fn window_score(hay: &[char], term: &[char], start: usize, end: usize) -> u32 {
    let mut s: i32 = 0;
    let mut ti = 0;
    let mut prev: Option<usize> = None;
    for i in start..=end {
        if ti < term.len() && hay[i] == term[ti] {
            s += 16;
            if prev == Some(i.wrapping_sub(1)) {
                s += 16;
            }
            if is_word_start(hay, i) {
                s += 24;
            }
            prev = Some(i);
            ti += 1;
        } else {
            s -= 2;
        }
    }
    s.max(1) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(q: &str, hay: &str) -> Option<u32> {
        Query::new(q).score(hay)
    }

    #[test]
    fn every_word_must_match() {
        assert!(score("starfall notes", "Notes for the Starfall launch").is_some());
        assert!(score("starfall moonforge", "Notes for the Starfall launch").is_none());
        assert_eq!(score("   ", "anything"), Some(0));
    }

    #[test]
    fn case_and_letters_in_order() {
        assert!(score("INVOICE", "invoice 2041").is_some());
        assert!(score("inv pd", "Invoice paid").is_some());
        assert!(score("mfg", "Moonforge").is_some());
        // Letters too far apart don't count.
        assert!(score("mfg", "my long sentence about nothing in particular, going").is_none());
        assert!(score("xyz", "Moonforge").is_none());
    }

    #[test]
    fn better_matches_rank_higher() {
        // Substring beats scattered letters.
        assert!(score("forge", "Moonforge") > score("forge", "for great egg"));
        // Start of a word beats the middle of one.
        assert!(score("light", "Light theme") > score("light", "Twilight theme"));
        // Earlier beats later.
        assert!(score("key", "key in front") > score("key", "and later the key"));
        // Adjacent letters beat spread-out ones.
        assert!(score("mire", "Mirelle") > score("mire", "my iron reel"));
    }

    #[test]
    fn handles_unicode() {
        assert!(score("čaj", "Zelený ČAJ").is_some());
        assert!(score("ß", "Straße").is_some());
    }

    #[test]
    fn filters_round_trip_by_index() {
        for f in Filter::ALL {
            assert_eq!(Filter::from_index(f.index()), f);
        }
        assert_eq!(Filter::from_index(99), Filter::All);
    }
}
