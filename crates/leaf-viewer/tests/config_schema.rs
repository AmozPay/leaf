mod common;

use common::{Fixture, page, snapshot};
use leaf_viewer::{Error, SearchRequest, WikiConfig, WikiIndex};
use std::fs;

fn open_err(root: &std::path::Path, db: &std::path::Path) -> Error {
    match WikiIndex::open(WikiConfig::new(root, db)) {
        Ok(_) => panic!("expected {db:?} to be rejected"),
        Err(err) => err,
    }
}

#[test]
fn rejects_cache_inside_raw_without_creating_anything() {
    let wiki = Fixture::new();
    let before = snapshot(&wiki.root);
    assert!(matches!(
        open_err(&wiki.root, &wiki.root.join("raw/index.db")),
        Error::InvalidConfig(_)
    ));
    assert!(matches!(
        open_err(&wiki.root, &wiki.root.join("raw/new-dir/index.db")),
        Error::InvalidConfig(_)
    ));
    assert!(matches!(
        open_err(&wiki.root, &wiki.root.join("concepts/../raw/index.db")),
        Error::InvalidConfig(_)
    ));
    assert!(!wiki.root.join("raw/new-dir").exists());
    assert!(!wiki.root.join("raw/index.db").exists());
    assert_eq!(snapshot(&wiki.root), before);
}

#[test]
fn requires_an_existing_database_directory_and_wiki_root() {
    let wiki = Fixture::new();
    assert!(matches!(
        open_err(&wiki.root, &wiki.base.join("missing/index.db")),
        Error::InvalidConfig(_)
    ));
    assert!(!wiki.base.join("missing").exists());
    assert!(matches!(
        open_err(&wiki.base.join("no-wiki"), &wiki.db),
        Error::InvalidConfig(_)
    ));
}

#[cfg(unix)]
#[test]
fn rejects_database_symlink_into_raw_sources() {
    use std::os::unix::fs::symlink;
    let wiki = Fixture::new();
    let raw_file = wiki.root.join("raw/source.db");
    rusqlite::Connection::open(&raw_file)
        .unwrap()
        .execute_batch("CREATE TABLE data (v TEXT);")
        .unwrap();
    let before = fs::read(&raw_file).unwrap();
    symlink(&raw_file, &wiki.db).unwrap();
    assert!(matches!(
        open_err(&wiki.root, &wiki.db),
        Error::InvalidConfig(_)
    ));
    assert_eq!(fs::read(raw_file).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn rejects_dangling_database_symlink_into_raw() {
    use std::os::unix::fs::symlink;
    let wiki = Fixture::new();
    let target = wiki.root.join("raw/created-by-sqlite.db");
    symlink(&target, &wiki.db).unwrap();
    assert!(matches!(
        open_err(&wiki.root, &wiki.db),
        Error::InvalidConfig(_)
    ));
    assert!(!target.exists());
}

#[cfg(unix)]
#[test]
fn rejects_database_directory_symlinked_into_raw() {
    use std::os::unix::fs::symlink;
    let wiki = Fixture::new();
    symlink(wiki.root.join("raw/chatgpt"), wiki.base.join("cache")).unwrap();
    let before = snapshot(&wiki.root);
    assert!(matches!(
        open_err(&wiki.root, &wiki.base.join("cache/index.db")),
        Error::InvalidConfig(_)
    ));
    assert_eq!(snapshot(&wiki.root), before);
}

#[cfg(unix)]
#[test]
fn rejects_database_in_the_target_of_a_symlinked_raw_directory() {
    use std::os::unix::fs::symlink;
    let wiki = Fixture::new();
    let sources = wiki.base.join("sources");
    fs::create_dir_all(&sources).unwrap();
    fs::remove_dir_all(wiki.root.join("raw")).unwrap();
    symlink(&sources, wiki.root.join("raw")).unwrap();
    assert!(matches!(
        open_err(&wiki.root, &sources.join("index.db")),
        Error::InvalidConfig(_)
    ));
    assert!(!sources.join("index.db").exists());
}

#[test]
fn index_survives_close_and_reopen() {
    let wiki = Fixture::new();
    wiki.write("index.md", "[[concepts/topic]]");
    wiki.write("concepts/topic.md", &page("Topic", "River delta survey."));
    drop(wiki.synced());
    let mut reopened = wiki.open();
    let response = reopened
        .search(&SearchRequest::new("river", 3, 1000))
        .unwrap();
    assert_eq!(response.hits.len(), 1);
    assert_eq!(reopened.sync().unwrap().skipped, 1);
}

#[test]
fn foreign_databases_are_rejected_and_left_unchanged() {
    let wiki = Fixture::new();
    rusqlite::Connection::open(&wiki.db)
        .unwrap()
        .execute_batch("CREATE TABLE notes (v TEXT); INSERT INTO notes VALUES ('keep');")
        .unwrap();
    let before = fs::read(&wiki.db).unwrap();
    assert!(matches!(
        open_err(&wiki.root, &wiki.db),
        Error::ForeignDatabase(_)
    ));
    let rebuild = WikiIndex::open(wiki.config().rebuild_incompatible_cache(true));
    assert!(matches!(rebuild, Err(Error::ForeignDatabase(_))));
    assert_eq!(fs::read(&wiki.db).unwrap(), before);
}

#[test]
fn unversioned_exploratory_cache_is_not_adopted() {
    let wiki = Fixture::new();
    rusqlite::Connection::open(&wiki.db)
        .unwrap()
        .execute_batch(
            "CREATE TABLE page (path TEXT PRIMARY KEY, title TEXT NOT NULL, hash TEXT NOT NULL);
             PRAGMA user_version=1;",
        )
        .unwrap();
    let before = fs::read(&wiki.db).unwrap();
    assert!(matches!(
        open_err(&wiki.root, &wiki.db),
        Error::ForeignDatabase(_)
    ));
    assert_eq!(fs::read(&wiki.db).unwrap(), before);
}

#[test]
fn incompatible_schema_is_rejected_unless_rebuild_is_requested() {
    let wiki = Fixture::new();
    wiki.write("index.md", "[[concepts/topic]]");
    wiki.write("concepts/topic.md", &page("Topic", "River delta survey."));
    drop(wiki.synced());
    rusqlite::Connection::open(&wiki.db)
        .unwrap()
        .execute_batch("PRAGMA user_version=999;")
        .unwrap();
    assert!(matches!(
        open_err(&wiki.root, &wiki.db),
        Error::IncompatibleSchema { found: 999, .. }
    ));

    let mut rebuilt = WikiIndex::open(wiki.config().rebuild_incompatible_cache(true)).unwrap();
    assert_eq!(rebuilt.rebuilt_from(), Some(999));
    assert_eq!(rebuilt.stats().unwrap().pages, 0);
    assert_eq!(rebuilt.sync().unwrap().added, 1);
    drop(rebuilt);
    let reopened = wiki.open();
    assert_eq!(reopened.rebuilt_from(), None);
    assert_eq!(reopened.stats().unwrap().pages, 1);
}
