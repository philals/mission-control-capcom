//! What a repository's merge bot (Kodiak) is set up to do, read from its `.kodiak.toml`: the label
//! that turns automerge on (and the ones that keep a branch updated), and whether it updates every PR.
//! Repos differ (`automerge 🚀`, `automerge`, both, `autoupdate`…), so the names are read, never guessed.
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Kodiak {
    /// Labels that make Kodiak merge a PR once it is approved and green.
    pub automerge: Vec<String>,
    /// Labels that make Kodiak keep a PR's branch up to date.
    pub autoupdate: Vec<String>,
    /// Kodiak updates every PR's branch, labelled or not.
    pub update_always: bool,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    fn into_vec(self) -> Vec<String> {
        match self {
            OneOrMany::One(one) => vec![one],
            OneOrMany::Many(many) => many,
        }
    }
}

#[derive(Deserialize, Default)]
struct Merge {
    automerge_label: Option<OneOrMany>,
}

#[derive(Deserialize, Default)]
struct Update {
    always: Option<bool>,
    autoupdate_label: Option<OneOrMany>,
}

#[derive(Deserialize, Default)]
struct Raw {
    merge: Option<Merge>,
    update: Option<Update>,
}

/// The settings in the text of a `.kodiak.toml`. A config with no automerge label means Kodiak's own
/// default, `automerge`.
pub fn parse(text: &str) -> Option<Kodiak> {
    let raw: Raw = toml::from_str(text).ok()?;
    let update = raw.update.unwrap_or_default();
    let automerge = raw.merge.and_then(|m| m.automerge_label).map(OneOrMany::into_vec).unwrap_or_else(|| vec!["automerge".to_string()]);
    Some(Kodiak {
        automerge: automerge.into_iter().filter(|l| !l.trim().is_empty()).collect(),
        autoupdate: update.autoupdate_label.map(OneOrMany::into_vec).unwrap_or_default().into_iter().filter(|l| !l.trim().is_empty()).collect(),
        update_always: update.always.unwrap_or(false),
    })
}

/// The settings in GitHub's answer for a file (`GET /repos/…/contents/…`): JSON with the file base64-encoded.
pub fn from_contents(json: &str) -> Option<Kodiak> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let encoded: String = value.get("content")?.as_str()?.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).ok()?;
    parse(&String::from_utf8(bytes).ok()?)
}

/// What looking for a repo's config found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    Config(Kodiak),
    /// No `.kodiak.toml`: the repo has no Kodiak.
    Nothing,
    /// Could not find out (network, permissions): ask again soon.
    Unknown,
}

/// Look in the two places Kodiak reads its config from.
pub fn fetch(repo: &str) -> Found {
    for file in [".kodiak.toml", ".github/.kodiak.toml"] {
        match capcom::github::rest(&format!("repos/{repo}/contents/{file}")) {
            Ok(json) => {
                return from_contents(&json).map_or(Found::Unknown, Found::Config);
            }
            Err(e) if format!("{e:#}").contains("404") => continue,
            Err(_) => return Found::Unknown,
        }
    }
    Found::Nothing
}

const KEEP: Duration = Duration::from_secs(6 * 3600);
const RETRY: Duration = Duration::from_secs(5 * 60);

/// Each repo's config, looked up once in a while: it changes rarely and every PR of the repo shares it.
#[derive(Default)]
pub struct Cache {
    seen: Mutex<HashMap<String, (Instant, Found)>>,
}

impl Cache {
    /// The config of each repo (None for a repo without one or one we could not find out about). The ones
    /// not looked up lately are fetched now, side by side.
    pub fn lookup(&self, repos: &[String], fetch: &(dyn Fn(&str) -> Found + Sync)) -> HashMap<String, Option<Kodiak>> {
        let stale: Vec<&String> = {
            let seen = self.seen.lock().expect("kodiak cache");
            repos
                .iter()
                .filter(|r| match seen.get(*r) {
                    Some((at, Found::Unknown)) => at.elapsed() > RETRY,
                    Some((at, _)) => at.elapsed() > KEEP,
                    None => true,
                })
                .collect()
        };
        let fetched: Vec<(String, Found)> = std::thread::scope(|scope| {
            let handles: Vec<_> = stale.iter().map(|repo| scope.spawn(move || ((*repo).clone(), fetch(repo)))).collect();
            handles.into_iter().filter_map(|h| h.join().ok()).collect()
        });
        let mut seen = self.seen.lock().expect("kodiak cache");
        for (repo, found) in fetched {
            seen.insert(repo, (Instant::now(), found));
        }
        repos
            .iter()
            .map(|r| (r.clone(), seen.get(r).and_then(|(_, found)| if let Found::Config(k) = found { Some(k.clone()) } else { None })))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn labels_may_be_one_name_or_a_list_and_update_always_is_read() {
        let admin = parse("[merge]\nautomerge_label = \"automerge 🚀\"\n[update]\nautoupdate_label = \"autoupdate\"\n").unwrap();
        assert_eq!((admin.automerge.as_slice(), admin.autoupdate.as_slice(), admin.update_always), (&["automerge 🚀".to_string()][..], &["autoupdate".to_string()][..], false));
        let titles = parse("[merge]\nautomerge_label = [\"automerge 🚀\", \"automerge\"]\npriority_merge_label = \"to the moon 🚀🚀\"\n[update]\nautoupdate_label = \"autoupdate\"\n").unwrap();
        assert_eq!(titles.automerge, vec!["automerge 🚀".to_string(), "automerge".to_string()]);
        let async_ = parse("[merge]\nautomerge_label = \"automerge\"\n[update]\nalways = true\nautoupdate_label = \"autoupdate\"\n").unwrap();
        assert!(async_.update_always && async_.autoupdate == vec!["autoupdate".to_string()]);
        let domain = parse("[merge]\nautomerge_label = [\"automerge 🚀\", \"automerge\"]\n").unwrap();
        assert!(domain.autoupdate.is_empty() && !domain.update_always, "no [update] section: Kodiak does not update branches there");
    }

    #[test]
    fn a_config_that_names_no_automerge_label_means_kodiaks_default_and_a_broken_one_means_none() {
        assert_eq!(parse("version = 1\n").unwrap().automerge, vec!["automerge".to_string()]);
        assert_eq!(parse("[merge]\nautomerge_label = \"\"\n").unwrap().automerge, Vec::<String>::new());
        assert_eq!(parse("this is [not toml"), None);
    }

    #[test]
    fn a_github_contents_answer_is_decoded_even_with_the_line_breaks_github_puts_in_the_base64() {
        let text = "[merge]\nautomerge_label = \"automerge 🚀\"\n";
        let encoded = base64::engine::general_purpose::STANDARD.encode(text);
        let (a, b) = encoded.split_at(10);
        let json = format!("{{\"name\":\".kodiak.toml\",\"encoding\":\"base64\",\"content\":\"{a}\\n{b}\\n\"}}");
        assert_eq!(from_contents(&json).unwrap().automerge, vec!["automerge 🚀".to_string()]);
        assert_eq!(from_contents("{\"message\":\"Not Found\"}"), None);
        assert_eq!(from_contents("nope"), None);
    }

    #[test]
    fn each_repo_is_looked_up_once_and_a_failure_is_retried_but_not_at_once() {
        let calls = AtomicUsize::new(0);
        let fetch = |repo: &str| {
            calls.fetch_add(1, Ordering::SeqCst);
            match repo {
                "acme/api" => Found::Config(Kodiak { automerge: vec!["automerge".into()], ..Kodiak::default() }),
                "acme/none" => Found::Nothing,
                _ => Found::Unknown,
            }
        };
        let cache = Cache::default();
        let repos = ["acme/api".to_string(), "acme/none".to_string(), "acme/down".to_string()];
        let first = cache.lookup(&repos, &fetch);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert!(first["acme/api"].is_some() && first["acme/none"].is_none() && first["acme/down"].is_none());
        cache.lookup(&repos, &fetch);
        assert_eq!(calls.load(Ordering::SeqCst), 3, "kept for hours, and a failure is not hammered");
        cache.seen.lock().unwrap().get_mut("acme/down").unwrap().0 = Instant::now() - Duration::from_secs(6 * 60);
        cache.lookup(&repos, &fetch);
        assert_eq!(calls.load(Ordering::SeqCst), 4, "only the failed one is tried again after a few minutes");
    }
}
