use super::model::PackageChoice;

fn fuzzy_score_token(token: &str, candidate: &str) -> Option<i64> {
    let token = token.trim();
    if token.is_empty() {
        return Some(0);
    }

    let token_chars = token
        .chars()
        .map(|character| character.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let candidate_chars = candidate.chars().collect::<Vec<_>>();
    if candidate_chars.is_empty() {
        return None;
    }

    let lowered_candidate = candidate_chars
        .iter()
        .map(|character| character.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let lowered_candidate_string = lowered_candidate.iter().collect::<String>();
    let lowered_token = token_chars.iter().collect::<String>();

    let mut score = 0i64;
    let mut search_from = 0usize;
    let mut first_match = None;
    let mut previous_match = None;

    for token_char in token_chars {
        let mut found = None;
        for index in search_from..lowered_candidate.len() {
            if lowered_candidate[index] == token_char {
                found = Some(index);
                break;
            }
        }

        let index = found?;
        if first_match.is_none() {
            first_match = Some(index);
        }

        score += 10;
        if index == 0 {
            score += 15;
        } else {
            let previous_character = candidate_chars[index - 1];
            let current_character = candidate_chars[index];
            if !previous_character.is_ascii_alphanumeric() {
                score += 12;
            } else if current_character.is_ascii_uppercase()
                && previous_character.is_ascii_lowercase()
            {
                score += 8;
            }
        }

        if let Some(previous_index) = previous_match {
            if index == previous_index + 1 {
                score += 18;
            } else {
                score -= (index - previous_index - 1) as i64;
            }
        }

        previous_match = Some(index);
        search_from = index + 1;
    }

    if let Some(index) = first_match {
        score -= index as i64;
    }
    score -= (candidate_chars
        .len()
        .saturating_sub(lowered_token.chars().count())) as i64;
    if lowered_candidate_string.contains(&lowered_token) {
        score += 30;
    }

    Some(score)
}

pub(super) fn fuzzy_score(query: &str, candidate: &str) -> Option<i64> {
    let tokens = query
        .split_whitespace()
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        return Some(0);
    }

    let mut total = 0i64;
    for token in tokens {
        total += fuzzy_score_token(token, candidate)?;
    }
    Some(total)
}

pub(super) fn package_search_score(
    query: &str,
    repository: &str,
    package: &PackageChoice,
) -> Option<i64> {
    let dependency_text = package.dependencies.join(" ");
    [
        fuzzy_score(query, &package.pkgname).map(|score| score + 60),
        fuzzy_score(query, &package.path).map(|score| score + 30),
        fuzzy_score(query, &dependency_text).map(|score| score + 15),
        fuzzy_score(
            query,
            &format!("{} {} {}", package.pkgname, package.path, dependency_text),
        )
        .map(|score| score + 5),
        fuzzy_score(query, repository),
    ]
    .into_iter()
    .flatten()
    .max()
}

#[cfg(test)]
mod tests {
    use super::fuzzy_score;

    #[test]
    fn fuzzy_score_matches_subsequence_tokens() {
        assert!(fuzzy_score("can drv", "CANDriver").is_some());
        assert!(fuzzy_score("mdi", "MotorDrivers::DJI").is_some());
        assert!(fuzzy_score("xyz", "MotorDrivers::DJI").is_none());
    }
}
