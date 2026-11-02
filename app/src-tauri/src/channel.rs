//! Which app this build is: the released typeshelf, or typeshelfDev, the preview of the
//! dev branch (`pnpm preview`). The dev channel has its own data folder, plugin port,
//! registry tag and logon task, so both run side by side, and it never auto-updates.
//! Set at compile time by scripts/preview.ts (TYPESHELF_CHANNEL, TYPESHELF_LABEL).

pub const IS_DEV: bool = match option_env!("TYPESHELF_CHANNEL") {
    Some(c) => same(c, "dev"),
    None => false,
};

/// String equality usable in a const.
const fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Name for the data folder and OS entries.
pub const APP: &str = if IS_DEV { "typeshelf-dev" } else { "typeshelf" };

/// Fixed so the Adobe plugin (released app only) can find it.
pub const PORT: u16 = if IS_DEV { 47216 } else { 47215 };

/// The dev build a preview leads up to (0.2.0.3-preview), shown in the UI; empty for releases.
pub const BUILD_LABEL: &str = match option_env!("TYPESHELF_LABEL") {
    Some(l) => l,
    None => "",
};
