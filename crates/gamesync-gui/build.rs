//! Embed the GameSync icon into the Windows executable so File Explorer, the
//! taskbar, and the installers show the app logo on the `.exe` itself. (egui
//! sets the *window* icon at runtime; this is the *file* icon, which lives in a
//! Windows resource compiled in here.)
//!
//! No-op on macOS/Linux — there the icon comes from the `.app`/`.dmg` and the
//! `.deb`/`.AppImage` desktop entry instead. Failures to embed are downgraded to
//! a warning so a missing resource compiler never breaks the build.
fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed Windows icon: {e}");
        }
    }
}
