//! Catalogue model of the App Store: merges the repository index with the
//! installed-package database and answers the questions the window asks
//! (which apps does this section show, which have updates). Pure logic.

use crate::display::icons::Icon;
use crate::pkg::package::{AppKind, RepoPackage};
use crate::pkg::version;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Category shown for installed packages the catalogue does not know.
pub const UNLISTED_CATEGORY: &str = "Other";

/// One app in the store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// Version the catalogue offers; empty when the catalogue lacks it.
    pub version: String,
    pub description: String,
    pub details: String,
    pub category: String,
    pub icon: Icon,
    pub kind: AppKind,
    /// Program "Open" starts.
    pub exec: String,
    /// Download size in bytes.
    pub size: u64,
    pub installed_size: u64,
    pub dependencies: Vec<String>,
    /// Version on this machine, when installed.
    pub installed: Option<String>,
}

/// What the state button of an entry does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Available,
    Installed,
    UpdateAvailable,
}

impl Status {
    /// Label of the primary button.
    #[must_use]
    pub const fn button(self) -> &'static str {
        match self {
            Status::Available => "Install",
            Status::Installed => "Open",
            Status::UpdateAvailable => "Update",
        }
    }
}

impl Entry {
    /// Install state, comparing versions numerically.
    #[must_use]
    pub fn status(&self) -> Status {
        match &self.installed {
            None => Status::Available,
            Some(have) if !self.version.is_empty() && version::is_newer(&self.version, have) => {
                Status::UpdateAvailable
            }
            Some(_) => Status::Installed,
        }
    }
}

/// An installed package as the database records it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledInfo {
    pub name: String,
    pub version: String,
    pub description: String,
    pub size: u64,
    /// First program the package installed, if any.
    pub exec: String,
}

/// Sidebar section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Section {
    Discover,
    Category(String),
    Installed,
    Updates,
}

impl Section {
    /// Sidebar label and content title.
    #[must_use]
    pub fn title(&self) -> &str {
        match self {
            Section::Discover => "Discover",
            Section::Category(name) => name,
            Section::Installed => "Installed",
            Section::Updates => "Updates",
        }
    }
}

/// Map an index `icon` name to a sprite; unknown names get the generic tile.
#[must_use]
pub fn icon_from_name(name: &str) -> Icon {
    match name {
        "AppWelcome" => Icon::AppWelcome,
        "AppPython" => Icon::AppPython,
        "AppTerminal" => Icon::AppTerminal,
        "AppSystemMonitor" => Icon::AppSystemMonitor,
        "AppEditor" => Icon::AppEditor,
        "AppFiles" => Icon::AppFiles,
        "AppBrowser" => Icon::AppBrowser,
        "AppCalculator" => Icon::AppCalculator,
        "AppImages" => Icon::AppImages,
        "AppNotes" => Icon::AppNotes,
        "AppNetwork" => Icon::AppNetwork,
        "AppSettings" => Icon::AppSettings,
        "AppTaskManager" => Icon::AppTaskManager,
        _ => Icon::AppGeneric,
    }
}

fn from_catalogue(pkg: &RepoPackage) -> Entry {
    let meta = &pkg.store;
    let category = if meta.category.is_empty() {
        UNLISTED_CATEGORY
    } else {
        &meta.category
    };
    Entry {
        name: pkg.name.clone(),
        version: pkg.version.clone(),
        description: pkg.description.clone(),
        details: meta.details.clone(),
        category: category.to_string(),
        icon: icon_from_name(&meta.icon),
        kind: meta.kind,
        exec: meta.exec.clone(),
        size: pkg.size,
        installed_size: meta.installed_size,
        dependencies: pkg.dependencies.clone(),
        installed: None,
    }
}

fn from_installed(info: &InstalledInfo) -> Entry {
    Entry {
        name: info.name.clone(),
        version: String::new(),
        description: info.description.clone(),
        details: String::new(),
        category: UNLISTED_CATEGORY.to_string(),
        icon: Icon::AppGeneric,
        kind: AppKind::Gui,
        exec: info.exec.clone(),
        size: info.size,
        installed_size: info.size,
        dependencies: Vec::new(),
        installed: Some(info.version.clone()),
    }
}

/// Merge the catalogue with the installed database: one entry per name,
/// sorted by name. An installed package the catalogue does not list (or
/// the whole catalogue when offline) stays visible.
#[must_use]
pub fn merge(catalogue: &[RepoPackage], installed: &[InstalledInfo]) -> Vec<Entry> {
    let mut entries: Vec<Entry> = catalogue.iter().map(from_catalogue).collect();
    for info in installed {
        match entries.iter_mut().find(|e| e.name == info.name) {
            Some(entry) => {
                entry.installed = Some(info.version.clone());
                if entry.exec.is_empty() {
                    entry.exec = info.exec.clone();
                }
            }
            None => entries.push(from_installed(info)),
        }
    }
    entries.sort_by_key(|e| e.name.to_lowercase());
    entries
}

/// Number of entries with a newer catalogue version.
#[must_use]
pub fn update_count(entries: &[Entry]) -> usize {
    entries
        .iter()
        .filter(|e| e.status() == Status::UpdateAvailable)
        .count()
}

/// Sidebar sections in order: Discover, each category, Installed, Updates.
#[must_use]
pub fn sections(entries: &[Entry]) -> Vec<Section> {
    let mut names: Vec<&str> = entries.iter().map(|e| e.category.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    let mut out = alloc::vec![Section::Discover];
    out.extend(names.into_iter().map(|n| Section::Category(n.to_string())));
    out.push(Section::Installed);
    out.push(Section::Updates);
    out
}

fn in_section(entry: &Entry, section: &Section) -> bool {
    match section {
        Section::Discover => true,
        Section::Category(name) => &entry.category == name,
        Section::Installed => entry.installed.is_some(),
        Section::Updates => entry.status() == Status::UpdateAvailable,
    }
}

fn matches_query(entry: &Entry, query: &str) -> bool {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return true;
    }
    [&entry.name, &entry.description, &entry.category]
        .iter()
        .any(|field| field.to_lowercase().contains(&needle))
}

/// Indices of the entries shown for `section` and the search `query`.
#[must_use]
pub fn visible(entries: &[Entry], section: &Section, query: &str) -> Vec<usize> {
    (0..entries.len())
        .filter(|&i| in_section(&entries[i], section) && matches_query(&entries[i], query))
        .collect()
}

/// "12.3 KB" style size for the list and details pane.
#[must_use]
pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    if bytes >= MB {
        alloc::format!("{}.{} MB", bytes / MB, bytes % MB * 10 / MB)
    } else if bytes >= KB {
        alloc::format!("{}.{} KB", bytes / KB, bytes % KB * 10 / KB)
    } else {
        alloc::format!("{bytes} B")
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::pkg::package::{PackageArch, StoreMeta};

    fn repo(name: &str, ver: &str, category: &str) -> RepoPackage {
        RepoPackage {
            name: name.to_string(),
            version: ver.to_string(),
            arch: PackageArch::Aarch64,
            description: alloc::format!("{name} description"),
            dependencies: Vec::new(),
            size: 2048,
            checksum: [0; 32],
            download_url: String::new(),
            store: StoreMeta {
                category: category.to_string(),
                icon: "AppPython".to_string(),
                ..StoreMeta::default()
            },
        }
    }

    fn have(name: &str, ver: &str) -> InstalledInfo {
        InstalledInfo {
            name: name.to_string(),
            version: ver.to_string(),
            description: "local".to_string(),
            size: 10,
            exec: alloc::format!("/apps/{name}"),
        }
    }

    #[test]
    fn merge_marks_installed_and_sorts_by_name() {
        let cat = [
            repo("sieve", "1.0.0", "Developer"),
            repo("hello", "1.0.0", "Utilities"),
        ];
        let all = merge(&cat, &[have("hello", "1.0.0")]);
        assert_eq!(all[0].name, "hello");
        assert_eq!(all[0].status(), Status::Installed);
        assert_eq!(all[1].status(), Status::Available);
        assert_eq!(all[0].exec, "/apps/hello");
    }

    #[test]
    fn ten_is_newer_than_nine() {
        let cat = [repo("a", "1.10.0", "Games")];
        let all = merge(&cat, &[have("a", "1.9.0")]);
        assert_eq!(all[0].status(), Status::UpdateAvailable);
        assert_eq!(update_count(&all), 1);
        let same = merge(&cat, &[have("a", "1.10.0")]);
        assert_eq!(same[0].status(), Status::Installed);
        let older = merge(&[repo("a", "1.9.0", "Games")], &[have("a", "1.10.0")]);
        assert_eq!(older[0].status(), Status::Installed);
    }

    #[test]
    fn offline_catalogue_still_lists_installed_apps() {
        let all = merge(&[], &[have("hello", "1.0.0")]);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].status(), Status::Installed);
        assert_eq!(all[0].category, UNLISTED_CATEGORY);
        assert_eq!(visible(&all, &Section::Installed, ""), [0]);
    }

    #[test]
    fn sections_list_categories_between_discover_and_installed() {
        let cat = [
            repo("a", "1", "Games"),
            repo("b", "1", "Developer"),
            repo("c", "1", "Games"),
        ];
        let s = sections(&merge(&cat, &[]));
        let titles: Vec<&str> = s.iter().map(Section::title).collect();
        assert_eq!(
            titles,
            ["Discover", "Developer", "Games", "Installed", "Updates"]
        );
    }

    #[test]
    fn visible_filters_by_section_and_query() {
        let cat = [repo("sieve", "1", "Developer"), repo("mines", "1", "Games")];
        let all = merge(&cat, &[have("mines", "1")]);
        assert_eq!(visible(&all, &Section::Discover, "").len(), 2);
        let games = Section::Category("Games".to_string());
        assert_eq!(visible(&all, &games, ""), [0]);
        assert_eq!(visible(&all, &Section::Installed, ""), [0]);
        assert!(visible(&all, &Section::Updates, "").is_empty());
        assert_eq!(visible(&all, &Section::Discover, "  SIE "), [1]);
        assert_eq!(visible(&all, &Section::Discover, "games"), [0]);
        assert!(visible(&all, &Section::Discover, "zzz").is_empty());
    }

    #[test]
    fn unknown_icon_name_falls_back_to_generic() {
        assert_eq!(icon_from_name("AppPython"), Icon::AppPython);
        assert_eq!(icon_from_name("Nope"), Icon::AppGeneric);
    }

    #[test]
    fn sizes_are_formatted_with_one_decimal() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(3 * 1024 * 1024), "3.0 MB");
    }
}
