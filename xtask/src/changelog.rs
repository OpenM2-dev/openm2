//! A release's changelog: the commits since the previous release, grouped by
//! their Conventional Commits type, as Markdown for the release's notes.

use anyhow::Result;

use crate::util::output;

/// Prints the changelog of `to` (a tag or any commit): the commits since
/// `from`, or since the last `v*` tag before `to` when `from` is not given,
/// or every commit when there is none. With `repo_url`
/// (`https://github.com/OWNER/REPO`), it ends with a link to every commit.
pub fn print(to: &str, from: Option<&str>, repo_url: Option<&str>) -> Result<()> {
    let from = match from {
        Some(from) => Some(from.to_owned()),
        None => previous_tag(to),
    };
    let range = match &from {
        Some(from) => format!("{from}..{to}"),
        None => to.to_owned(),
    };
    let log = output("git", &["log", "--no-merges", "--format=%s", &range])?;
    let subjects: Vec<&str> = log.lines().collect();
    print!("{}", notes(&subjects, from.as_deref(), to, repo_url));
    Ok(())
}

/// The last `v*` tag reachable from `to`'s parents, if any.
fn previous_tag(to: &str) -> Option<String> {
    let parent = format!("{to}^");
    output(
        "git",
        &[
            "describe",
            "--tags",
            "--abbrev=0",
            "--match",
            "v[0-9]*",
            &parent,
        ],
    )
    .ok()
    .map(|tag| tag.trim().to_owned())
    .filter(|tag| !tag.is_empty())
}

/// A changelog's sections, in the order they are printed.
const SECTIONS: [&str; 3] = ["Features", "Fixes", "Other changes"];

/// Which section a commit type goes into; `None` for the commits a player
/// doesn't see (docs, tests, CI, formatting, the release commits).
fn section(kind: &str, scope: Option<&str>) -> Option<usize> {
    match kind {
        "feat" => Some(0),
        "fix" => Some(1),
        "docs" | "test" | "ci" | "style" => None,
        "chore" if scope == Some("release") => None,
        _ => Some(2),
    }
}

/// A commit subject read as `type(scope)!: summary`. A subject in another
/// form is an "Other changes" line as it is.
struct Subject<'a> {
    kind: Option<&'a str>,
    scope: Option<&'a str>,
    breaking: bool,
    summary: &'a str,
}

impl<'a> Subject<'a> {
    fn parse(subject: &'a str) -> Self {
        let plain = Subject {
            kind: None,
            scope: None,
            breaking: false,
            summary: subject.trim(),
        };
        let Some((head, summary)) = subject.split_once(": ") else {
            return plain;
        };
        let (head, breaking) = match head.strip_suffix('!') {
            Some(head) => (head, true),
            None => (head, false),
        };
        let (kind, scope) = match head.split_once('(') {
            Some((kind, rest)) => match rest.strip_suffix(')') {
                Some(scope) if !scope.is_empty() => (kind, Some(scope)),
                _ => return plain,
            },
            None => (head, None),
        };
        if kind.is_empty() || !kind.bytes().all(|b| b.is_ascii_lowercase()) {
            return plain;
        }
        Subject {
            kind: Some(kind),
            scope,
            breaking,
            summary: summary.trim(),
        }
    }

    fn line(&self) -> String {
        let breaking = if self.breaking { " (breaking)" } else { "" };
        match self.scope {
            Some(scope) => format!("- **{scope}:** {}{breaking}", self.summary),
            None => format!("- {}{breaking}", self.summary),
        }
    }
}

/// The changelog as Markdown, from the commit subjects newest first. The
/// same subject twice (a commit and its rebased copy) shows once.
fn notes(subjects: &[&str], from: Option<&str>, to: &str, repo_url: Option<&str>) -> String {
    let mut sections: [Vec<String>; 3] = Default::default();
    for subject in subjects.iter().map(|s| Subject::parse(s)) {
        if subject.summary.is_empty() {
            continue;
        }
        let place = match subject.kind {
            Some(kind) => section(kind, subject.scope),
            None => Some(2),
        };
        if let Some(lines) = place.and_then(|place| sections.get_mut(place)) {
            let line = subject.line();
            if !lines.contains(&line) {
                lines.push(line);
            }
        }
    }

    let mut out = String::from("## What's changed\n");
    if sections.iter().all(Vec::is_empty) {
        out.push_str(
            "\nNothing a player or an operator would notice: documentation, tests and upkeep.\n",
        );
    }
    for (title, lines) in SECTIONS.iter().zip(&sections) {
        if lines.is_empty() {
            continue;
        }
        out.push_str(&format!("\n### {title}\n"));
        for line in lines {
            out.push_str(line);
            out.push('\n');
        }
    }
    if let Some(url) = repo_url {
        let url = url.trim_end_matches('/');
        match from {
            Some(from) => out.push_str(&format!("\nEvery commit: {url}/compare/{from}...{to}\n")),
            None => out.push_str(&format!("\nEvery commit: {url}/commits/{to}\n")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_commits_by_type() {
        let subjects = [
            "chore(release): version 0.17.0",
            "feat(play): skill looks, damage numbers and misses",
            "fix(play): pick items up by a click",
            "docs(specs): the upgrade window",
            "refactor(core): one queue for effects",
            "feat: servers by address",
            "test(server): warps",
            "fix(play): pick items up by a click",
        ];
        let notes = notes(
            &subjects,
            Some("v0.16.0"),
            "v0.17.0",
            Some("https://github.com/OpenM2-dev/openm2/"),
        );
        assert_eq!(
            notes,
            "## What's changed\n\
             \n### Features\n\
             - **play:** skill looks, damage numbers and misses\n\
             - servers by address\n\
             \n### Fixes\n\
             - **play:** pick items up by a click\n\
             \n### Other changes\n\
             - **core:** one queue for effects\n\
             \nEvery commit: https://github.com/OpenM2-dev/openm2/compare/v0.16.0...v0.17.0\n"
        );
    }

    #[test]
    fn keeps_other_subjects_and_marks_breaking_ones() {
        let subjects = [
            "Merge the launcher's look",
            "feat(protocol)!: version 4",
            "fix CI",
        ];
        let notes = notes(&subjects, None, "v1.0.0", None);
        assert_eq!(
            notes,
            "## What's changed\n\
             \n### Features\n\
             - **protocol:** version 4 (breaking)\n\
             \n### Other changes\n\
             - Merge the launcher's look\n\
             - fix CI\n"
        );
    }

    #[test]
    fn says_so_when_nothing_shows() {
        let notes = notes(
            &["docs: typos", "chore(release): version 1.0.1"],
            Some("v1.0.0"),
            "v1.0.1",
            None,
        );
        assert!(notes.contains("Nothing a player or an operator would notice"));
        assert!(!notes.contains("###"));
    }

    #[test]
    fn links_every_commit_of_a_first_release() {
        let notes = notes(
            &["feat: the first one"],
            None,
            "v0.1.0",
            Some("https://example.org/r"),
        );
        assert!(notes.ends_with("\nEvery commit: https://example.org/r/commits/v0.1.0\n"));
    }

    #[test]
    fn reads_odd_subjects_as_they_are() {
        for subject in ["Feat(x): y", "feat(): y", "feat(x: y", ": y", "no colon"] {
            let parsed = Subject::parse(subject);
            assert_eq!(parsed.kind, None, "{subject}");
            assert_eq!(parsed.summary, subject.trim(), "{subject}");
        }
    }
}
