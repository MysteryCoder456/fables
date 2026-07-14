/// Case-insensitive subsequence match with a lightweight fzf-style score:
/// +10 per matched character, +15 when it's contiguous with the previous
/// match, +20 when the match starts at position 0 (prefix bonus). Returns
/// `None` when `query` is not a subsequence of `candidate`.
pub fn subsequence_score(query: &str, candidate: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(0);
    }
    let q: Vec<char> = query.to_lowercase().chars().collect();
    let c: Vec<char> = candidate.to_lowercase().chars().collect();
    let mut qi = 0;
    let mut score: i64 = 0;
    let mut last_match: Option<usize> = None;
    for (ci, ch) in c.iter().enumerate() {
        if qi >= q.len() {
            break;
        }
        if *ch == q[qi] {
            score += 10;
            if last_match == Some(ci.wrapping_sub(1)) {
                score += 15;
            }
            if ci == 0 {
                score += 20;
            }
            last_match = Some(ci);
            qi += 1;
        }
    }
    (qi == q.len()).then_some(score)
}

/// Filters `items` to those whose label is a subsequence match for `query`,
/// stable-sorted descending by `subsequence_score` (ties keep `items`' order).
pub fn subsequence_rank<T>(query: &str, items: Vec<(T, String)>) -> Vec<T> {
    let mut scored: Vec<(i64, T)> = items
        .into_iter()
        .filter_map(|(item, label)| subsequence_score(query, &label).map(|s| (s, item)))
        .collect();
    scored.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
    scored.into_iter().map(|(_, item)| item).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_matches_everything_with_zero_score() {
        assert_eq!(subsequence_score("", "anything"), Some(0));
    }

    #[test]
    fn non_subsequence_is_none() {
        assert_eq!(subsequence_score("xyz", "queue"), None);
    }

    #[test]
    fn out_of_order_subsequence_is_none() {
        assert_eq!(subsequence_score("eq", "queue"), None);
    }

    #[test]
    fn scattered_subsequence_matches() {
        assert!(subsequence_score("qee", "queue").is_some());
    }

    #[test]
    fn contiguous_prefix_match_scores_higher_than_scattered_match() {
        let contiguous = subsequence_score("que", "queue").unwrap();
        let scattered = subsequence_score("que", "q_u_e_ntity").unwrap();
        assert!(contiguous > scattered, "{contiguous} should beat {scattered}");
    }

    #[test]
    fn rank_filters_non_matches_and_orders_by_score() {
        let items = vec![
            ("no-match", "zzz".to_string()),
            ("scattered", "q_u_e".to_string()),
            ("exact-prefix", "queue".to_string()),
        ];
        let ranked = subsequence_rank("que", items);
        assert_eq!(ranked, vec!["exact-prefix", "scattered"]);
    }
}
