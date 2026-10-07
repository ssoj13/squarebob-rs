//! Icons are egui-phosphor through `egui_widgets_config::icons`, never raw Unicode glyphs: the
//! default fonts miss most of them, so they render as tofu squares. This scans the UI sources
//! (the app and the egui widget crates) for symbol-block characters in code (comments are free).
//! Terminal output lives in `xtask` and is out of scope.

use std::path::{Path, PathBuf};

/// Blocks whose characters are icons in disguise: miscellaneous technical (media controls),
/// geometric shapes, miscellaneous symbols, dingbats, supplemental arrows-A / arrows-B /
/// symbols and arrows, the emoji planes (which hold geometric shapes extended, U+1F780..1F7FF)
/// and the fullwidth forms. The plain arrows block (U+2190..21FF) is left out:
/// `->` style arrows inside prose hints are text, and the default fonts have them.
fn is_icon_char(c: char) -> bool {
    matches!(c as u32,
        0x2300..=0x23FF | 0x25A0..=0x25FF | 0x2600..=0x26FF | 0x2700..=0x27BF |
        0x27F0..=0x27FF | 0x2900..=0x297F | 0x2B00..=0x2BFF | 0x1F000..=0x1FFFF | 0xFF00..=0xFFEF)
}

/// The code of one line: everything before a `//` that is outside a string or char literal.
fn code_of(line: &str) -> String {
    let mut out = String::new();
    let (mut in_str, mut escaped) = (false, false);
    let mut prev = '\0';
    for c in line.chars() {
        if in_str {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
            out.push(c);
        } else if c == '/' && prev == '/' {
            out.pop();
            break;
        } else {
            out.push(c);
        }
        prev = c;
    }
    out
}

/// A `\u{XXXX}` escape of an icon-block character in code.
fn has_icon_escape(code: &str) -> bool {
    let mut rest = code;
    while let Some(i) = rest.find("\\u{") {
        let tail = &rest[i + 3..];
        if let Some(end) = tail.find('}')
            && let Ok(cp) = u32::from_str_radix(&tail[..end], 16)
            && char::from_u32(cp).is_some_and(is_icon_char)
        {
            return true;
        }
        rest = &rest[i + 3..];
    }
    false
}

/// UI sources: the app, and the crates that draw egui.
const UI_DIRS: [&str; 4] = [
    "src",
    "crates/playa-ae/src",
    "crates/squarebob-widgets/src",
    "crates/media-encoder/src",
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            // Vendored upstream study material and build output are not ours.
            if name != "target" && name != "_ref" && name != "reference" {
                rust_files(&path, out);
            }
        } else if name.ends_with(".rs") {
            out.push(path);
        }
    }
}

#[test]
fn icon_blocks_cover_arrows_and_shapes_but_not_prose_arrows() {
    // Code points, not literals: the scan below would flag a literal in this very file.
    let icons = [0x27F6, 0x27F0, 0x27FF, 0x2905, 0x297F, 0x23F5, 0x1F7E0];
    let prose = [0x2190, 0x2192, 0x21FF, 0x61];
    let is = |cp: u32| char::from_u32(cp).is_some_and(is_icon_char);
    assert!(icons.iter().all(|&cp| is(cp)), "icon blocks");
    assert!(
        prose.iter().all(|&cp| !is(cp)),
        "prose arrows stay allowed in text"
    );
}

#[test]
fn no_raw_unicode_icon_literals_in_any_crate() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for dir in UI_DIRS {
        rust_files(&root.join(dir), &mut files);
    }
    assert!(
        files.len() > 20,
        "the scan found no sources: {}",
        files.len()
    );
    let mut bad = Vec::new();
    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        for (n, line) in text.lines().enumerate() {
            let code = code_of(line);
            if code.chars().any(is_icon_char) || has_icon_escape(&code) {
                bad.push(format!("{}:{}: {}", file.display(), n + 1, line.trim()));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "raw Unicode icons (use egui_widgets_config::icons):\n{}",
        bad.join("\n")
    );
}
