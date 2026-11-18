//! The icon inventory: which Tabler glyph, colour and sizes make each icon.

/// Sizes (px) rendered for every app tile.
pub const TILE_SIZES: [u32; 3] = [48, 32, 24];
/// Sizes (px) rendered for every file-type icon.
pub const FILE_SIZES: [u32; 3] = [48, 24, 16];
/// Sizes (px) rendered for every symbolic glyph mask.
pub const GLYPH_SIZES: [u32; 3] = [16, 20, 24];

/// A full-colour app tile.
pub struct App {
    pub variant: &'static str,
    pub glyph: &'static str,
    /// Accent colour as `rrggbb`; the tile gradient is derived from it.
    pub color: &'static str,
}

/// How a file-type icon is drawn.
pub enum FileKind {
    Folder,
    FolderOpen,
    /// Amber folder with a white Tabler glyph on its front.
    FolderGlyph(&'static str),
    /// Page with fold. `tinted` pages are filled with the colour and carry a
    /// white glyph; otherwise the page is light grey and the glyph coloured.
    Page {
        glyph: &'static str,
        color: &'static str,
        tinted: bool,
    },
    Trash {
        full: bool,
    },
}

/// A full-colour file-type icon.
pub struct FileIcon {
    pub variant: &'static str,
    pub kind: FileKind,
}

/// A symbolic glyph (alpha mask).
pub struct Sym {
    pub variant: &'static str,
    pub glyph: &'static str,
}

const fn app(variant: &'static str, glyph: &'static str, color: &'static str) -> App {
    App {
        variant,
        glyph,
        color,
    }
}

const fn page(variant: &'static str, glyph: &'static str, color: &'static str) -> FileIcon {
    FileIcon {
        variant,
        kind: FileKind::Page {
            glyph,
            color,
            tinted: false,
        },
    }
}

const fn sym(variant: &'static str, glyph: &'static str) -> Sym {
    Sym { variant, glyph }
}

const fn folder(variant: &'static str, kind: FileKind) -> FileIcon {
    FileIcon { variant, kind }
}

pub const APPS: &[App] = &[
    app("AppTerminal", "terminal-2", "2b3040"),
    app("AppFiles", "folder", "e8941a"),
    app("AppEditor", "file-code", "8b5cf6"),
    app("AppCalculator", "calculator", "f26b21"),
    app("AppTaskManager", "activity", "16a34a"),
    app("AppSystemMonitor", "chart-area-line", "0d9488"),
    app("AppSettings", "settings", "6b7280"),
    app("AppNetwork", "network", "4f46e5"),
    app("AppStore", "building-store", "db2777"),
    app("AppImages", "photo", "e11d48"),
    app("AppBrowser", "world", "0ea5e9"),
    app("AppNotes", "notes", "ca8a04"),
    app("AppAbout", "info-circle", "0369a1"),
    app("AppWelcome", "confetti", "d946ef"),
    app("AppPython", "brand-python", "3776ab"),
    app("AppGeneric", "app-window", "475569"),
];

pub fn files() -> Vec<FileIcon> {
    vec![
        folder("FileFolder", FileKind::Folder),
        folder("FileFolderOpen", FileKind::FolderOpen),
        folder("FileHome", FileKind::FolderGlyph("home")),
        folder("FileDesktop", FileKind::FolderGlyph("device-desktop")),
        folder("FileDocuments", FileKind::FolderGlyph("file-text")),
        folder("FileDownloads", FileKind::FolderGlyph("download")),
        folder("FilePictures", FileKind::FolderGlyph("photo")),
        folder("FileMusic", FileKind::FolderGlyph("music")),
        folder("FileTrashEmpty", FileKind::Trash { full: false }),
        folder("FileTrashFull", FileKind::Trash { full: true }),
        page("FileGeneric", "file", "64748b"),
        page("FileText", "align-left", "475569"),
        page("FileCode", "code", "7c3aed"),
        page("FileImage", "photo", "e11d48"),
        page("FileAudio", "music", "0d9488"),
        page("FileVideo", "movie", "db2777"),
        page("FileArchive", "archive", "b45309"),
        page("FileExecutable", "binary", "15803d"),
        FileIcon {
            variant: "FilePdf",
            kind: FileKind::Page {
                glyph: "book",
                color: "dc2626",
                tinted: true,
            },
        },
        page("FileLink", "link", "2563eb"),
    ]
}

pub const GLYPHS: &[Sym] = &[
    sym("Wifi", "wifi"),
    sym("WifiOff", "wifi-off"),
    sym("Ethernet", "network"),
    sym("Volume", "volume"),
    sym("VolumeHigh", "volume-2"),
    sym("VolumeMuted", "volume-3"),
    sym("Bell", "bell"),
    sym("Battery", "battery"),
    sym("Power", "power"),
    sym("Lock", "lock"),
    sym("Logout", "logout"),
    sym("Refresh", "refresh"),
    sym("Search", "search"),
    sym("Menu", "menu-2"),
    sym("Close", "x"),
    sym("Minimize", "minus"),
    sym("Maximize", "square"),
    sym("Restore", "copy"),
    sym("ChevronLeft", "chevron-left"),
    sym("ChevronRight", "chevron-right"),
    sym("ChevronUp", "chevron-up"),
    sym("ChevronDown", "chevron-down"),
    sym("ArrowLeft", "arrow-left"),
    sym("ArrowRight", "arrow-right"),
    sym("ArrowUp", "arrow-up"),
    sym("Home", "home"),
    sym("Plus", "plus"),
    sym("Check", "check"),
    sym("Trash", "trash"),
    sym("Folder", "folder"),
    sym("File", "file"),
    sym("Settings", "settings"),
    sym("Grid", "layout-grid"),
    sym("Desktop", "device-desktop"),
];
