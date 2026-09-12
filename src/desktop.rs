//! Register CrabMD in the OS app launcher (Spotlight, Start Menu, GNOME/KDE).
//!
//! cargo / brew formula / npm / curl only put a binary on PATH. First GUI
//! launch (or `crabmd --install-desktop`) writes a real app entry. The Homebrew
//! cask already ships `/Applications/CrabMD.app` — we leave that alone.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

#[cfg(target_os = "macos")]
use crate::assets::APP_ICON_ICNS;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::assets::APP_ICON_PNG;

const APP_NAME: &str = "CrabMD";
const BUNDLE_ID: &str = "ai.blankeos.crabmd";

/// Best-effort; never fail a GUI launch. macOS is a no-op (cask owns
/// /Applications); Linux/Windows auto-install.
pub fn ensure_installed() {
    #[cfg(target_os = "macos")]
    {
        return;
    }
    #[cfg(not(target_os = "macos"))]
    {
        let Ok(exe) = current_exe() else {
            return;
        };
        if is_dev_binary(&exe) {
            return;
        }
        if let Err(err) = platform_install(&exe) {
            eprintln!("crabmd: desktop app: {err:#}");
        }
    }
}

pub fn install() -> Result<PathBuf> {
    let exe = current_exe()?;
    if is_dev_binary(&exe) {
        anyhow::bail!("refusing to register a cargo build; use an installed binary");
    }
    platform_install(&exe)?.context("desktop install produced no path")
}

pub fn uninstall() -> Result<()> {
    uninstall_inner()
}

fn current_exe() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("resolving crabmd executable")?;
    Ok(fs::canonicalize(&exe).unwrap_or(exe))
}

/// `cargo r` / `just dev` binaries die on rebuild; don't point Spotlight at them.
pub(crate) fn is_dev_binary(exe: &Path) -> bool {
    let s = exe.to_string_lossy();
    let markers = [
        "/target/debug/",
        "/target/release/",
        "\\target\\debug\\",
        "\\target\\release\\",
    ];
    markers.iter().any(|m| s.contains(m))
}

/// Homebrew Cellar paths vanish on upgrade; `opt/crabmd` is the stable symlink.
pub(crate) fn stable_exe(exe: &Path) -> PathBuf {
    let s = exe.to_string_lossy().replace('\\', "/");
    if let Some(idx) = s.find("/Cellar/crabmd/") {
        let opt = PathBuf::from(format!("{}/opt/crabmd/bin/crabmd", &s[..idx]));
        if opt.exists() {
            return opt;
        }
    }
    exe.to_path_buf()
}

/// Copy via a sibling temp + rename so we never truncate a running binary.
#[cfg(target_os = "macos")]
fn copy_if_stale(src: &Path, dst: &Path) -> Result<bool> {
    if dst.exists() {
        let src_meta = fs::metadata(src)?;
        let dst_meta = fs::metadata(dst)?;
        if src_meta.len() == dst_meta.len() {
            if let (Ok(s), Ok(d)) = (src_meta.modified(), dst_meta.modified()) {
                if s <= d {
                    return Ok(false);
                }
            }
        }
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = dst.with_file_name(format!(
        ".{}.new",
        dst.file_name().and_then(|n| n.to_str()).unwrap_or("bin")
    ));
    fs::copy(src, &tmp).with_context(|| format!("copy {} → {}", src.display(), tmp.display()))?;
    if let Err(err) = fs::rename(&tmp, dst) {
        let _ = fs::remove_file(&tmp);
        return Err(err).with_context(|| format!("replace {}", dst.display()));
    }
    Ok(true)
}

fn write_if_changed(path: &Path, bytes: &[u8]) -> Result<()> {
    if path.exists() {
        if let Ok(existing) = fs::read(path) {
            if existing == bytes {
                return Ok(());
            }
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn png_resized(px: u32) -> Result<Vec<u8>> {
    let img = image::load_from_memory(APP_ICON_PNG).context("decoding app-icon.png")?;
    let resized = img.resize_exact(px, px, image::imageops::FilterType::Triangle);
    let mut buf = Vec::new();
    resized
        .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
        .context("encoding png icon")?;
    Ok(buf)
}

/// Vista+ ICO that embeds a PNG payload (256×256).
#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
pub(crate) fn png_to_ico(png: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(22 + png.len());
    out.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
    out.push(0); // 256
    out.push(0);
    out.push(0);
    out.push(0);
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&(png.len() as u32).to_le_bytes());
    out.extend_from_slice(&22u32.to_le_bytes());
    out.extend_from_slice(png);
    out
}

#[cfg(target_os = "macos")]
fn platform_install(exe: &Path) -> Result<Option<PathBuf>> {
    let applications = PathBuf::from("/Applications").join(format!("{APP_NAME}.app"));
    let home_app = home_dir()?
        .join("Applications")
        .join(format!("{APP_NAME}.app"));

    if exe_is_inside_app(exe) {
        refresh_bundle(exe)?;
        return Ok(Some(
            app_root_from_exe(exe).unwrap_or_else(|| exe.to_path_buf()),
        ));
    }
    // Cask already owns Launch Services. Don't fight it with a user copy.
    if applications.exists() {
        if home_app.exists() {
            let _ = fs::remove_dir_all(&home_app);
        }
        return Ok(Some(applications));
    }
    let src = stable_exe(exe);
    write_macos_app(&home_app, &src)?;
    Ok(Some(home_app))
}

/// True when `exe` is inside `*.app/Contents/MacOS/`.
#[cfg_attr(not(any(test, target_os = "macos")), allow(dead_code))]
pub(crate) fn exe_is_inside_app(exe: &Path) -> bool {
    exe.to_string_lossy().contains(".app/Contents/MacOS/")
}

/// `…/CrabMD.app/Contents/MacOS/crabmd` → `…/CrabMD.app`.
#[cfg_attr(not(any(test, target_os = "macos")), allow(dead_code))]
pub(crate) fn app_root_from_exe(exe: &Path) -> Option<PathBuf> {
    exe.parent()?.parent()?.parent().map(Path::to_path_buf)
}

#[cfg_attr(not(any(test, target_os = "macos")), allow(dead_code))]
pub(crate) fn bundle_from_exe(exe: &Path) -> Option<PathBuf> {
    if exe_is_inside_app(exe) {
        app_root_from_exe(exe)
    } else {
        None
    }
}

/// `Foo.app` with `Contents/Info.plist` (no plist parsing on fast path).
#[cfg_attr(not(any(test, target_os = "macos")), allow(dead_code))]
pub(crate) fn is_valid_app_bundle(bundle: &Path) -> bool {
    bundle.extension().and_then(|e| e.to_str()) == Some("app")
        && bundle.join("Contents/Info.plist").is_file()
}

/// First present candidate: exe bundle, then /Applications, then ~/Applications.
#[cfg_attr(not(any(test, target_os = "macos")), allow(dead_code))]
pub(crate) fn pick_bundle_in_order(candidates: &[Option<PathBuf>]) -> Option<PathBuf> {
    candidates.iter().filter_map(|c| c.clone()).next()
}

/// Bundle for `open` cold start: canonical exe bundle, /Applications, ~/Applications.
#[cfg(target_os = "macos")]
pub(crate) fn find_launch_app_bundle() -> Option<PathBuf> {
    let exe_bundle = std::env::current_exe()
        .ok()
        .map(|p| fs::canonicalize(&p).unwrap_or(p))
        .and_then(|exe| bundle_from_exe(&exe))
        .filter(|b| is_valid_app_bundle(b));
    let applications = PathBuf::from("/Applications").join(format!("{APP_NAME}.app"));
    let applications = is_valid_app_bundle(&applications).then(|| applications);
    let home_bundle = home_dir()
        .ok()
        .map(|h| h.join("Applications").join(format!("{APP_NAME}.app")))
        .filter(|b| is_valid_app_bundle(b));
    pick_bundle_in_order(&[exe_bundle, applications, home_bundle])
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn find_launch_app_bundle() -> Option<PathBuf> {
    None
}

#[cfg(target_os = "macos")]
fn refresh_bundle(bundle_exe: &Path) -> Result<()> {
    let Some(app) = app_root_from_exe(bundle_exe) else {
        return Ok(());
    };
    let origin_path = app.join("Contents/Resources/origin");
    let origin = fs::read_to_string(&origin_path)
        .ok()
        .map(|s| PathBuf::from(s.trim()))
        .filter(|p| p.is_file());
    let Some(origin) = origin else {
        return Ok(());
    };
    let src = stable_exe(&origin);
    if copy_if_stale(&src, bundle_exe)? {
        adhoc_sign(&app);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn adhoc_sign(app: &Path) {
    let _ = std::process::Command::new("codesign")
        .args(["--force", "--deep", "--sign", "-", &app.to_string_lossy()])
        .status();
}

#[cfg(target_os = "macos")]
fn write_macos_app(app: &Path, src: &Path) -> Result<()> {
    let macos = app.join("Contents/MacOS");
    let resources = app.join("Contents/Resources");
    fs::create_dir_all(&macos)?;
    fs::create_dir_all(&resources)?;
    let dest_exe = macos.join("crabmd");
    let copied = copy_if_stale(src, &dest_exe)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dest_exe, fs::Permissions::from_mode(0o755))?;
    }
    write_if_changed(&resources.join("AppIcon.icns"), APP_ICON_ICNS)?;
    write_if_changed(&resources.join("origin"), src.to_string_lossy().as_bytes())?;
    write_if_changed(&app.join("Contents/PkgInfo"), b"APPL????")?;
    let version = env!("CARGO_PKG_VERSION");
    write_if_changed(
        &app.join("Contents/Info.plist"),
        macos_plist(version).as_bytes(),
    )?;
    if copied || !app.join("Contents/_CodeSignature").exists() {
        adhoc_sign(app);
    }
    let lsregister = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";
    let _ = std::process::Command::new(lsregister)
        .args(["-f", &app.to_string_lossy()])
        .status();
    Ok(())
}

#[cfg(target_os = "macos")]
fn macos_plist(version: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleDisplayName</key><string>{APP_NAME}</string>
  <key>CFBundleExecutable</key><string>crabmd</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundleIdentifier</key><string>{BUNDLE_ID}</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>{APP_NAME}</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>{version}</string>
  <key>CFBundleVersion</key><string>{version}</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeExtensions</key>
      <array>
        <string>md</string>
        <string>markdown</string>
        <string>mdown</string>
        <string>mkd</string>
      </array>
      <key>CFBundleTypeName</key><string>Markdown document</string>
      <key>CFBundleTypeRole</key><string>Editor</string>
      <key>LSHandlerRank</key><string>Alternate</string>
    </dict>
  </array>
</dict>
</plist>
"#
    )
}

#[cfg(target_os = "macos")]
fn uninstall_inner() -> Result<()> {
    let home_app = home_dir()?
        .join("Applications")
        .join(format!("{APP_NAME}.app"));
    if home_app.exists() {
        fs::remove_dir_all(&home_app)
            .with_context(|| format!("removing {}", home_app.display()))?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn platform_install(exe: &Path) -> Result<Option<PathBuf>> {
    let src = stable_exe(exe);
    let data = xdg_data_home()?;
    let desktop_path = data
        .join("applications")
        .join(format!("{BUNDLE_ID}.desktop"));
    let icon_path = data.join("icons/hicolor/256x256/apps").join("crabmd.png");
    let pixmap = data.join("pixmaps").join("crabmd.png");
    let png = png_resized(256)?;
    write_if_changed(&icon_path, &png)?;
    write_if_changed(&pixmap, &png)?;
    write_if_changed(&desktop_path, linux_desktop(&src).as_bytes())?;
    let _ = std::process::Command::new("update-desktop-database")
        .arg(data.join("applications"))
        .status();
    Ok(Some(desktop_path))
}

#[cfg(target_os = "linux")]
pub(crate) fn linux_desktop(exe: &Path) -> String {
    let exec = shell_quote(&exe.to_string_lossy());
    format!(
        "\
[Desktop Entry]
Type=Application
Name={APP_NAME}
Comment=A fast native markdown writer
Exec={exec} %F
Icon=crabmd
Terminal=false
Categories=Office;TextEditor;
MimeType=text/markdown;text/x-markdown;
StartupWMClass={BUNDLE_ID}
StartupNotify=true
"
    )
}

#[cfg(target_os = "linux")]
fn uninstall_inner() -> Result<()> {
    let data = xdg_data_home()?;
    for rel in [
        format!("applications/{BUNDLE_ID}.desktop"),
        "icons/hicolor/256x256/apps/crabmd.png".into(),
        "pixmaps/crabmd.png".into(),
    ] {
        let p = data.join(rel);
        if p.exists() {
            fs::remove_file(&p).with_context(|| format!("removing {}", p.display()))?;
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn platform_install(exe: &Path) -> Result<Option<PathBuf>> {
    let src = stable_exe(exe);
    let programs = windows_programs_dir()?;
    let data = windows_data_dir()?;
    let ico_path = data.join("crabmd.ico");
    let png = png_resized(256)?;
    write_if_changed(&ico_path, &png_to_ico(&png))?;
    let lnk = programs.join(format!("{APP_NAME}.lnk"));
    write_windows_shortcut(&lnk, &src, &ico_path)?;
    Ok(Some(lnk))
}

#[cfg(target_os = "windows")]
fn write_windows_shortcut(lnk: &Path, exe: &Path, ico: &Path) -> Result<()> {
    if let Some(parent) = lnk.parent() {
        fs::create_dir_all(parent)?;
    }
    let work = exe.parent().unwrap_or(Path::new("."));
    let script = format!(
        "$s = (New-Object -ComObject WScript.Shell).CreateShortcut('{}');\
         $s.TargetPath = '{}';\
         $s.WorkingDirectory = '{}';\
         $s.IconLocation = '{},0';\
         $s.Description = 'CrabMD';\
         $s.Save()",
        ps_single(&lnk.to_string_lossy()),
        ps_single(&exe.to_string_lossy()),
        ps_single(&work.to_string_lossy()),
        ps_single(&ico.to_string_lossy()),
    );
    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .status()
        .context("running powershell to create Start Menu shortcut")?;
    anyhow::ensure!(status.success(), "powershell shortcut failed: {status}");
    Ok(())
}

#[cfg(target_os = "windows")]
fn ps_single(s: &str) -> String {
    s.replace('\'', "''")
}

#[cfg(target_os = "windows")]
fn uninstall_inner() -> Result<()> {
    let lnk = windows_programs_dir()?.join(format!("{APP_NAME}.lnk"));
    if lnk.exists() {
        fs::remove_file(&lnk)?;
    }
    let ico = windows_data_dir()?.join("crabmd.ico");
    if ico.exists() {
        fs::remove_file(&ico)?;
    }
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn platform_install(_exe: &Path) -> Result<Option<PathBuf>> {
    Ok(None)
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn uninstall_inner() -> Result<()> {
    Ok(())
}

pub(crate) fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .context("HOME / USERPROFILE not set")
}

#[cfg(target_os = "linux")]
fn xdg_data_home() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME") {
        return Ok(PathBuf::from(dir));
    }
    Ok(home_dir()?.join(".local/share"))
}

#[cfg(target_os = "windows")]
fn windows_programs_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("APPDATA") {
        return Ok(PathBuf::from(dir).join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    Ok(home_dir()?.join(r"AppData\Roaming\Microsoft\Windows\Start Menu\Programs"))
}

#[cfg(target_os = "windows")]
fn windows_data_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("LOCALAPPDATA") {
        return Ok(PathBuf::from(dir).join("crabmd"));
    }
    Ok(home_dir()?.join(r"AppData\Local\crabmd"))
}

#[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
pub(crate) fn shell_quote(s: &str) -> String {
    if s.bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-' | b':' | b'+'))
    {
        s.to_string()
    } else {
        format!("\"{}\"", s.replace('"', "\\\""))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_binaries_detected() {
        assert!(is_dev_binary(Path::new(
            "/Users/me/crabmd/target/debug/crabmd"
        )));
        assert!(is_dev_binary(Path::new(
            r"C:\src\crabmd\target\release\crabmd.exe"
        )));
        assert!(!is_dev_binary(Path::new("/opt/homebrew/bin/crabmd")));
        assert!(!is_dev_binary(Path::new("/home/me/.cargo/bin/crabmd")));
    }

    #[test]
    fn cellar_rewrites_to_opt() {
        let cellar = Path::new("/opt/homebrew/Cellar/crabmd/0.0.2/bin/crabmd");
        let got = stable_exe(cellar);
        if got != cellar {
            assert!(got.ends_with("opt/crabmd/bin/crabmd"));
        }
    }

    #[test]
    fn ico_header_is_png_wrapped() {
        let png = b"\x89PNG\r\n\x1a\n";
        let ico = png_to_ico(png);
        assert_eq!(&ico[0..6], &[0, 0, 1, 0, 1, 0]);
        assert_eq!(&ico[22..], png);
        let size = u32::from_le_bytes(ico[14..18].try_into().unwrap());
        assert_eq!(size as usize, png.len());
    }

    #[test]
    fn shell_quote_spaces() {
        assert_eq!(shell_quote("/usr/bin/crabmd"), "/usr/bin/crabmd");
        assert_eq!(shell_quote("/home/a b/crabmd"), "\"/home/a b/crabmd\"");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn desktop_file_points_at_exe() {
        let body = linux_desktop(Path::new("/usr/bin/crabmd"));
        assert!(body.contains("Exec=/usr/bin/crabmd %F"));
        assert!(body.contains("StartupWMClass=ai.blankeos.crabmd"));
    }

    #[test]
    fn exe_inside_app_detection() {
        // Raw LS launch + resolved cask shim both count as inside-app.
        assert!(exe_is_inside_app(Path::new(
            "/Applications/CrabMD.app/Contents/MacOS/crabmd"
        )));
        assert!(exe_is_inside_app(Path::new(
            "/Users/me/Applications/CrabMD.app/Contents/MacOS/crabmd"
        )));
        assert!(!exe_is_inside_app(Path::new("/opt/homebrew/bin/crabmd")));
        assert!(!exe_is_inside_app(Path::new(
            "/opt/homebrew/Cellar/crabmd/0.0.3/bin/crabmd"
        )));
        assert!(!exe_is_inside_app(Path::new("/usr/local/bin/crabmd")));
    }

    #[test]
    fn bundle_root_from_exe() {
        assert_eq!(
            bundle_from_exe(Path::new("/Applications/CrabMD.app/Contents/MacOS/crabmd")),
            Some(PathBuf::from("/Applications/CrabMD.app"))
        );
        assert_eq!(
            app_root_from_exe(Path::new(
                "/Users/me/Applications/CrabMD.app/Contents/MacOS/crabmd"
            )),
            Some(PathBuf::from("/Users/me/Applications/CrabMD.app"))
        );
        assert_eq!(bundle_from_exe(Path::new("/opt/homebrew/bin/crabmd")), None);
    }

    #[test]
    fn bundle_priority_prefers_exe_identity() {
        let exe = Some(PathBuf::from("/Applications/CrabMD.app"));
        let sys = Some(PathBuf::from("/Applications/CrabMD.app"));
        let home = Some(PathBuf::from("/Users/me/Applications/CrabMD.app"));
        // Exe bundle wins even when system/home copies exist (same identity).
        assert_eq!(
            pick_bundle_in_order(&[exe.clone(), sys.clone(), home.clone()]),
            exe
        );
        // No exe bundle → system before home.
        assert_eq!(
            pick_bundle_in_order(&[None, sys.clone(), home.clone()]),
            sys
        );
        assert_eq!(pick_bundle_in_order(&[None, None, home.clone()]), home);
        assert_eq!(pick_bundle_in_order(&[None, None, None]), None);
    }

    #[test]
    fn valid_bundle_needs_info_plist() {
        let dir = std::env::temp_dir().join(format!("crabmd-bundle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bundle = dir.join("CrabMD.app");
        let plist = bundle.join("Contents/Info.plist");
        assert!(!is_valid_app_bundle(&bundle));
        std::fs::create_dir_all(plist.parent().unwrap()).unwrap();
        std::fs::write(&plist, b"plist").unwrap();
        assert!(is_valid_app_bundle(&bundle));
        // Wrong extension never counts, even with a plist inside.
        let not_app = dir.join("CrabMD");
        std::fs::create_dir_all(not_app.join("Contents")).unwrap();
        std::fs::write(not_app.join("Contents/Info.plist"), b"plist").unwrap();
        assert!(!is_valid_app_bundle(&not_app));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
