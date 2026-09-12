#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod assets;
mod config;
mod coords;
mod daemon;
mod desktop;
mod display;
mod document;
mod editor;
mod frontmatter;
mod images;
mod mermaid;
mod mode;
mod motion;
mod notion;
mod palette;
mod slash;
mod surface;
mod syntax;
mod tabs;
mod theme;
mod toast;
mod tree;
mod undo;
mod video;
mod wysiwyg;

use std::collections::HashSet;
use std::ffi::OsString;
use std::io::IsTerminal as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use gpui::{
    point, px, size, AnyWindowHandle, App, AppContext as _, BorrowAppContext as _, Entity, Global,
    QuitMode, Styled as _, TitlebarOptions, WeakEntity, WindowBounds, WindowId, WindowOptions,
};
use gpui_component::{ActiveTheme as _, Root};

use crate::config::Config;
use crate::editor::bind_keys;
use crate::tabs::{bind_tab_keys, WorkspaceShell};
use crate::theme::Palette;

fn main() {
    if let Err(err) = run() {
        eprintln!("crabmd: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = Args::parse()?;
    if args.help {
        attach_stdio();
        print_help();
        return Ok(());
    }
    if args.list_themes {
        attach_stdio();
        for name in theme::list_theme_names() {
            println!("{name}");
        }
        return Ok(());
    }
    if args.install_desktop {
        attach_stdio();
        let path = desktop::install()?;
        println!("installed {}", path.display());
        return Ok(());
    }
    if args.uninstall_desktop {
        attach_stdio();
        desktop::uninstall()?;
        println!("removed desktop app");
        return Ok(());
    }
    // LS launches stay attached; terminal calls detach unless `-w`.
    let wait = args.wait || stay_attached();
    if !wait {
        // Live daemon: forward and exit. Otherwise cold-start the daemon.
        let forward = args
            .path
            .as_deref()
            .map(absolutize)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        if daemon::try_forward(&forward, args.line, args.col, args.behavior.as_str()) {
            return Ok(());
        }
        #[cfg(target_os = "macos")]
        if cold_start_via_open(&args, &forward) {
            return Ok(());
        }
        detach_and_reexec()?;
        return Ok(());
    }
    // macOS: cask owns /Applications; Linux/Windows auto-install.
    #[cfg(not(target_os = "macos"))]
    desktop::ensure_installed();
    let mut config = config::load();
    let palette = if args.theme_from_cli {
        theme::load_named(&args.theme)?
    } else {
        match theme::load_named(&config.theme) {
            Ok(p) => p,
            Err(_) => theme::load_named(theme::DEFAULT_THEME)?,
        }
    };
    if args.theme_from_cli {
        config.theme = palette.name.clone();
    }
    let (path, source, initial) = load_open_target(args.path.clone(), args.line, args.col)?;
    let delay_untitled = args.path.is_none() && stay_attached();
    launch(path, source, palette, config, initial, delay_untitled);
    Ok(())
}

/// True when started from inside `*.app/Contents/MacOS` (LS launch or a
/// resolved cask shim). Terminal launches still detach via `stay_attached`.
fn launched_from_app_bundle() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    if desktop::exe_is_inside_app(&exe) {
        return true;
    }
    let canonical = std::fs::canonicalize(&exe).unwrap_or(exe);
    desktop::exe_is_inside_app(&canonical)
}

fn stay_attached() -> bool {
    launched_from_app_bundle() && !std::io::stdin().is_terminal()
}

/// GUI subsystem binaries have no console. Re-attach for `--help` / install.
fn attach_stdio() {
    #[cfg(all(windows, not(debug_assertions)))]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn AttachConsole(dw_process_id: u32) -> i32;
        }
        const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
        unsafe {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

fn load_open_target(
    path: Option<PathBuf>,
    line: Option<usize>,
    col: Option<usize>,
) -> Result<(PathBuf, String, Option<(usize, usize)>)> {
    match path {
        None => Ok((crate::tabs::untitled_path(), String::new(), None)),
        Some(path) => {
            let path = std::fs::canonicalize(&path).unwrap_or(path);
            ensure_file(&path)?;
            let source = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            let initial = match (line, col) {
                (Some(line), col) => Some((line, col.unwrap_or(1))),
                (None, _) => None,
            };
            Ok((path, source, initial))
        }
    }
}

/// Spawn a detached `-w` child with the same args, then let the parent exit.
fn detach_and_reexec() -> Result<()> {
    let exe = std::env::current_exe().context("resolving crabmd executable")?;
    let mut child_args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    child_args.insert(0, "-w".into());
    let mut cmd = std::process::Command::new(exe);
    // Close stdio so panics/logs from the GUI child never leak back into the
    // launching terminal after the parent returns.
    cmd.args(&child_args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Own process group so the shell job can end with the parent.
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }
    cmd.spawn().context("launching crabmd window")?;
    Ok(())
}

/// `path[:line[:col]]` for `--args`. Caller must absolutize: an
/// `open`-launched app starts with cwd `/`.
pub(crate) fn format_open_file_arg(path: &Path, line: Option<usize>, col: Option<usize>) -> String {
    let mut s = path.to_string_lossy().into_owned();
    if let Some(line) = line {
        s.push_str(&format!(":{line}"));
        if let Some(col) = col {
            s.push_str(&format!(":{col}"));
        }
    }
    s
}

/// CLI argv for `open … --args` (abs path, behavior, theme, wait).
pub(crate) fn build_bundle_open_argv(args: &Args) -> Vec<OsString> {
    let mut out: Vec<OsString> = Vec::new();
    match args.behavior {
        OpenBehavior::New => out.push("-n".into()),
        OpenBehavior::Existing => out.push("-e".into()),
        OpenBehavior::Add => out.push("-a".into()),
        OpenBehavior::Reuse => out.push("-r".into()),
    }
    if args.theme_from_cli {
        out.push("--theme".into());
        out.push(args.theme.clone().into());
    }
    if args.wait {
        out.push("-w".into());
    }
    if let Some(path) = args.path.as_deref() {
        let abs = absolutize(path);
        out.push(format_open_file_arg(&abs, args.line, args.col).into());
    }
    out
}

/// Low-level `open <bundle> --args …`. No running-app check; prefer
/// `cold_start_via_open`, which avoids ignored `--args` and drops.
fn launch_bundle_via_open(bundle: &Path, args: &Args) -> bool {
    let argv = build_bundle_open_argv(args);
    let mut cmd = std::process::Command::new("/usr/bin/open");
    cmd.arg(bundle).arg("--args").args(&argv);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    matches!(cmd.status(), Ok(s) if s.success())
}

/// macOS cold start with delivery guarantees.
///
/// `open --args` is ignored when the app is already running, so a running
/// but not-yet-listening app must be reached via the socket (wait + forward),
/// not via `open`. Simultaneous starters elect one winner with the
/// coldstart lock; losers wait-forward instead of issuing a second `open`
/// whose `--args` would be dropped. The winner's `--args` is the single
/// delivery (no duplicate forward); Finder events still arrive via
/// `on_open_urls` and are drained in `launch`.
#[cfg(target_os = "macos")]
fn cold_start_via_open(args: &Args, forward: &str) -> bool {
    use std::time::Duration;
    const WAIT: Duration = Duration::from_secs(8);
    let Some(bundle) = desktop::find_launch_app_bundle() else {
        return false;
    };
    let lock_path = daemon::coldstart_lock_path();
    if is_app_process_running() || lock_path.exists() {
        if daemon::wait_and_forward(forward, args.line, args.col, args.behavior.as_str(), WAIT) {
            return true;
        }
        if lock_path.exists() && !daemon::coldstart_lock_is_stale(Duration::from_secs(30)) && is_app_process_running() {
            // Launching app never listened; detach fallback below opens the
            // file in a new process rather than dropping it.
            return false;
        }
        if lock_path.exists() && daemon::coldstart_lock_is_stale(Duration::from_secs(30)) {
            daemon::clear_coldstart_lock();
        } else if is_app_process_running() {
            return false;
        }
    }
    if !daemon::try_acquire_coldstart_lock() {
        // Lost the election; the winner's daemon will listen shortly.
        return daemon::wait_and_forward(forward, args.line, args.col, args.behavior.as_str(), WAIT);
    }
    if launch_bundle_via_open(&bundle, args) {
        return true;
    }
    daemon::clear_coldstart_lock();
    false
}

/// Best-effort: is another `crabmd` process alive (excluding self)?
/// Used only to avoid `open --args` when it would be ignored.
#[cfg(target_os = "macos")]
fn is_app_process_running() -> bool {
    let self_pid = std::process::id().to_string();
    if let Ok(out) = std::process::Command::new("pgrep").args(["-x", "crabmd"]).output() {
        if out.status.success() {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                if line.trim() != self_pid && !line.trim().is_empty() {
                    return true;
                }
            }
        }
    }
    let Ok(out) = std::process::Command::new("ps").args(["-ax", "-o", "pid=,comm="]).output() else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut parts = line.split_whitespace();
        let (Some(pid), Some(comm)) = (parts.next(), parts.next()) else {
            continue;
        };
        if pid == self_pid {
            continue;
        }
        if comm.contains("crabmd") || comm.contains("CrabMD") {
            return true;
        }
    }
    false
}

/// Test hook: pure decision for the cold-start path.
#[cfg(test)]
pub(crate) fn cold_start_decision(app_running: bool, lock_exists: bool) -> &'static str {
    if app_running || lock_exists {
        "wait-forward"
    } else {
        "elect-starter"
    }
}

fn ensure_file(path: &std::path::Path) -> Result<()> {
    if path.exists() {
        anyhow::ensure!(path.is_file(), "{} is not a file", path.display());
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
    }
    std::fs::write(path, "").with_context(|| format!("creating {}", path.display()))?;
    eprintln!("created {}", path.display());
    Ok(())
}

fn launch(
    path: PathBuf,
    source: String,
    palette: Palette,
    config: Config,
    initial: Option<(usize, usize)>,
    delay_untitled: bool,
) {
    let app = gpui_platform::application()
        .with_assets(crate::assets::Assets)
        // Stay alive with zero windows (daemon) so the next `crabmd file`
        // forwards over the socket instead of cold-booting. cmd-q quits.
        .with_quit_mode(QuitMode::Explicit);

    // Finder / Dock drop files via `application:openURLs:` — often after
    // launch, and the callback has no `&mut App`. Queue until we settle
    // the first window (argv vs Apple Event vs untitled).
    let pending = std::rc::Rc::new(std::cell::RefCell::new(PendingOpens::default()));
    let settled = std::rc::Rc::new(std::cell::Cell::new(false));

    let pending_cb = pending.clone();
    let settled_cb = settled.clone();
    app.on_open_urls(move |urls| {
        pending_cb.borrow_mut().urls.extend(urls);
        if !settled_cb.get() {
            return;
        }
        let Some(async_app) = pending_cb.borrow().async_app.clone() else {
            return;
        };
        let urls = std::mem::take(&mut pending_cb.borrow_mut().urls);
        let _ = async_app.update(|cx| open_urls(cx, urls));
    });
    let settled_reopen = settled.clone();
    app.on_reopen(move |cx| {
        if !settled_reopen.get() {
            return;
        }
        // Dock/Spotlight with no windows opens untitled; otherwise focus.
        // Liveness needs weak + window-handle (weak can outlive the window).
        // Verified: GPUI `activate_window` uses `makeKeyAndOrderFront:`,
        // which deminiaturizes, so it restores minimized windows.
        let live = live_shells(cx);
        if live.is_empty() {
            open_untitled_window(cx);
        } else {
            let ordered: Vec<WindowId> = live.iter().map(|(_, h)| h.window_id()).collect();
            let active = cx.active_window().map(|h| h.window_id());
            let target_id = pick_live_target(active, &ordered);
            let target = target_id
                .and_then(|id| live.iter().find(|(_, h)| h.window_id() == id))
                .map(|(_, h)| *h)
                .or_else(|| live.first().map(|(_, h)| *h));
            if let Some(handle) = target {
                if handle
                    .update(cx, |_, window, _| window.activate_window())
                    .is_err()
                {
                    prune_shell_registry(cx);
                    open_untitled_window(cx);
                } else {
                    cx.activate(true);
                }
            } else {
                cx.activate(true);
            }
        }
    });

    app.run(move |cx| {
        pending.borrow_mut().async_app = Some(cx.to_async());
        gpui_component::init(cx);
        crate::assets::load_bundled_fonts(cx);
        bind_keys(cx);
        bind_tab_keys(cx);
        // Remote `http(s)` images (`img(SharedUri)`) download through
        // this client — without it GPUI uses a null client and every
        // remote photo silently never loads (same setup as GPUI's own
        // image example).
        if let Ok(client) = reqwest_client::ReqwestClient::user_agent("crabmd") {
            cx.set_http_client(std::sync::Arc::new(client));
        }
        crate::assets::apply_dock_icon();
        crate::editor::apply_palette(&palette, cx);

        cx.set_global(ShellRegistry { shells: Vec::new() });
        // Eagerly drop closed windows so a fast Spotlight reopen never
        // observes a stale handle between `remove_window()` and the next
        // prune-on-access.
        let _closed_sub = cx.on_window_closed(|cx, _| prune_shell_registry(cx));
        std::mem::forget(_closed_sub);
        // Single-instance socket. A lost bind race just opens without
        // registering; only the owner (Some) may delete the socket on quit.
        let ipc_rx = daemon::start_listener();
        let ipc_owned = ipc_rx.is_some();
        if delay_untitled {
            let palette = palette.clone();
            let config = config.clone();
            let pending = pending.clone();
            let settled = settled.clone();
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(250))
                    .await;
                let _ = cx.update(|cx| {
                    let urls = std::mem::take(&mut pending.borrow_mut().urls);
                    if urls.is_empty() {
                        open_editor_window(
                            crate::tabs::untitled_path(),
                            String::new(),
                            palette,
                            config,
                            None,
                            cx,
                        );
                    } else {
                        open_urls(cx, urls);
                    }
                    settled.set(true);
                });
            })
            .detach();
        } else {
            open_editor_window(path, source, palette, config, initial, cx);
            let urls = std::mem::take(&mut pending.borrow_mut().urls);
            open_urls(cx, urls);
            settled.set(true);
        }
        if let Some(rx) = ipc_rx {
            cx.spawn(async move |cx| loop {
                while let Ok(req) = rx.try_recv() {
                    let _ = cx.update(|cx| handle_open(cx, req));
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;
            })
            .detach();
        }
        // cmd-q cleanup by the socket owner only. Secondaries (`None`,
        // e.g. `--wait` one-shots that lost the race) must never delete
        // the live daemon's file. A crash leaves a stale file, which the
        // next launch detects (connect fails) and replaces.
        let _quit_sub = cx.on_app_quit(move |_| async move {
            if ipc_owned {
                daemon::cleanup();
            }
        });
        std::mem::forget(_quit_sub);
    });
}

#[derive(Default)]
struct PendingOpens {
    async_app: Option<gpui::AsyncApp>,
    urls: Vec<String>,
}

fn open_urls(cx: &mut App, urls: Vec<String>) {
    for url in urls {
        if let Some(path) = path_from_open_url(&url) {
            handle_open(
                cx,
                daemon::OpenRequest {
                    path: path.to_string_lossy().into_owned(),
                    line: None,
                    col: None,
                    behavior: "existing".into(),
                },
            );
        }
    }
}

/// Live shells + windows for single-instance routing.
pub(crate) struct ShellRegistry {
    pub(crate) shells: Vec<(WeakEntity<WorkspaceShell>, AnyWindowHandle)>,
}

impl Global for ShellRegistry {}

/// Live when the weak upgrades AND the window id is in `cx.windows()`.
pub(crate) fn is_shell_entry_live(
    weak_alive: bool,
    id: WindowId,
    live_ids: &HashSet<WindowId>,
) -> bool {
    weak_alive && live_ids.contains(&id)
}

/// Prefer the active window when live, else the first live window.
/// Minimized apps report `active_window() == None` but still have a live
/// window to `activate_window()` + `activate(true)`.
pub(crate) fn pick_live_target(
    active: Option<WindowId>,
    ordered_live: &[WindowId],
) -> Option<WindowId> {
    if let Some(active) = active {
        if ordered_live.contains(&active) {
            return Some(active);
        }
    }
    ordered_live.first().copied()
}

fn prune_shell_registry(cx: &mut App) {
    cx.update_global::<ShellRegistry, _>(|reg, cx| {
        let live: HashSet<WindowId> = cx.windows().iter().map(|w| w.window_id()).collect();
        reg.shells
            .retain(|(w, h)| is_shell_entry_live(w.upgrade().is_some(), h.window_id(), &live));
    });
}

fn live_shells(cx: &mut App) -> Vec<(Entity<WorkspaceShell>, AnyWindowHandle)> {
    cx.update_global::<ShellRegistry, _>(|reg, cx| {
        let live: HashSet<WindowId> = cx.windows().iter().map(|w| w.window_id()).collect();
        reg.shells
            .retain(|(w, h)| is_shell_entry_live(w.upgrade().is_some(), h.window_id(), &live));
        reg.shells
            .iter()
            .filter_map(|(w, h)| w.upgrade().map(|s| (s, *h)))
            .collect()
    })
}

fn open_untitled_window(cx: &mut App) {
    let config = config::load();
    let Ok(palette) =
        theme::load_named(&config.theme).or_else(|_| theme::load_named(theme::DEFAULT_THEME))
    else {
        return;
    };
    open_editor_window(
        crate::tabs::untitled_path(),
        String::new(),
        palette,
        config,
        None,
        cx,
    );
}

/// Route one forwarded `crabmd <file:line:col>` into this process.
fn handle_open(cx: &mut App, req: daemon::OpenRequest) {
    let (path, source, initial) = if req.path.is_empty() {
        (crate::tabs::untitled_path(), String::new(), None)
    } else {
        let mut path = PathBuf::from(&req.path);
        path = std::fs::canonicalize(&path).unwrap_or(path);
        if ensure_file(&path).is_err() {
            return;
        }
        let Ok(source) = std::fs::read_to_string(&path) else {
            return;
        };
        let initial = match (req.line, req.col) {
            (Some(line), col) => Some((line, col.unwrap_or(1))),
            (None, _) => None,
        };
        (path, source, initial)
    };
    let live = live_shells(cx);
    let ordered: Vec<WindowId> = live.iter().map(|(_, h)| h.window_id()).collect();
    let active = cx.active_window().map(|h| h.window_id());
    let target: Option<(Entity<WorkspaceShell>, AnyWindowHandle)> =
        pick_live_target(active, &ordered)
            .and_then(|id| live.iter().find(|(_, h)| h.window_id() == id))
            .map(|(w, h)| (w.clone(), *h));
    match target {
        Some((shell, handle)) if req.behavior != "new" => {
            let opened = handle
                .update(cx, |_, window, cx| {
                    shell.update(cx, |s, cx| {
                        s.open_tab(path.clone(), source.clone(), initial, window, cx);
                    });
                    window.activate_window();
                })
                .is_ok();
            if opened {
                cx.activate(true);
            } else {
                prune_shell_registry(cx);
                let config = config::load();
                let palette = match theme::load_named(&config.theme) {
                    Ok(p) => p,
                    Err(_) => match theme::load_named(theme::DEFAULT_THEME) {
                        Ok(p) => p,
                        Err(_) => return,
                    },
                };
                open_editor_window(path, source, palette, config, initial, cx);
            }
        }
        _ => {
            let config = config::load();
            let palette = match theme::load_named(&config.theme) {
                Ok(p) => p,
                Err(_) => match theme::load_named(theme::DEFAULT_THEME) {
                    Ok(p) => p,
                    Err(_) => return,
                },
            };
            open_editor_window(path, source, palette, config, initial, cx);
        }
    }
}

fn window_options(cx: &mut App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(880.), px(1000.)), cx)),
        window_min_size: Some(size(px(520.), px(380.))),
        titlebar: Some(TitlebarOptions {
            title: None,
            appears_transparent: true,
            traffic_light_position: Some(point(px(9.0), px(9.0))),
        }),
        app_owns_titlebar_drag: true,
        app_id: Some("ai.blankeos.crabmd".into()),
        icon: crate::assets::window_icon(),
        ..Default::default()
    }
}

/// Open one OS window hosting a tab shell. Used by CLI launch, cmd-shift-n,
/// and (next) the single-instance daemon's `-n` path.
pub(crate) fn open_editor_window(
    path: PathBuf,
    source: String,
    palette: Palette,
    config: Config,
    initial: Option<(usize, usize)>,
    cx: &mut App,
) {
    let window_options = window_options(cx);
    cx.activate(true);
    cx.spawn(async move |cx| {
        cx.open_window(window_options, |window, cx| {
            window.activate_window();
            let shell = WorkspaceShell::view(path, source, palette, config, initial, window, cx);
            cx.new(|cx| Root::new(shell, window, cx).bg(cx.theme().background))
        })
        .expect("failed to open window");
    })
    .detach();
}

struct Args {
    help: bool,
    list_themes: bool,
    install_desktop: bool,
    uninstall_desktop: bool,
    theme: String,
    theme_from_cli: bool,
    wait: bool,
    behavior: OpenBehavior,
    path: Option<PathBuf>,
    /// 1-based source line from `file.md:line[:col]` (zed-style).
    line: Option<usize>,
    /// 1-based source column from `file.md:line:col`.
    col: Option<usize>,
}

/// Zed-style open behavior. The default reuses the running window (fast
/// tab); `-n` forces a new window in the same process.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum OpenBehavior {
    New,
    #[default]
    Existing,
    Add,
    Reuse,
}

impl OpenBehavior {
    fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Existing => "existing",
            Self::Add => "add",
            Self::Reuse => "reuse",
        }
    }
}

impl Args {
    fn parse() -> Result<Self> {
        let mut help = false;
        let mut list_themes = false;
        let mut install_desktop = false;
        let mut uninstall_desktop = false;
        let mut theme = theme::DEFAULT_THEME.to_string();
        let mut theme_from_cli = false;
        let mut wait = false;
        let mut behavior = OpenBehavior::New;
        let mut path = None;
        let mut line = None;
        let mut col = None;
        let mut iter = std::env::args().skip(1).peekable();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-h" | "--help" => help = true,
                "--list-themes" => list_themes = true,
                "--install-desktop" => install_desktop = true,
                "--uninstall-desktop" => uninstall_desktop = true,
                "-w" | "--wait" => wait = true,
                "-n" | "--new" => behavior = OpenBehavior::New,
                "-e" | "--existing" => behavior = OpenBehavior::Existing,
                "-a" | "--add" => behavior = OpenBehavior::Add,
                "-r" | "--reuse" => behavior = OpenBehavior::Reuse,
                "--theme" | "-t" => {
                    theme = iter
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("--theme requires a name"))?;
                    theme_from_cli = true;
                }
                flag if flag.starts_with("--theme=") => {
                    theme = flag.trim_start_matches("--theme=").to_string();
                    theme_from_cli = true;
                }
                arg if arg.starts_with('-') => {
                    anyhow::bail!("unknown flag `{arg}`\n\n{HELP}");
                }
                _ => {
                    if path.is_some() {
                        anyhow::bail!("unexpected extra argument `{arg}`");
                    }
                    let (file, l, c) = split_file_position(&arg);
                    path = Some(PathBuf::from(file));
                    line = l;
                    col = c;
                }
            }
        }
        Ok(Self {
            help,
            list_themes,
            install_desktop,
            uninstall_desktop,
            theme,
            theme_from_cli,
            wait,
            behavior,
            path,
            line,
            col,
        })
    }
}

/// Make `path` absolute without touching the filesystem (the daemon does
/// `ensure_file` + read, so the fast CLI path stays pure).
fn absolutize(path: &std::path::Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

/// Split zed-style `file.md:line:col` (or `file.md:line`) into its parts.
/// Trailing `:digits` suffixes are treated as line/column; anything else is
/// kept verbatim as the file path (so `a:b.md:4` -> `a:b.md` line 4, while
/// `notes.md:abc` stays a plain path).
fn split_file_position(arg: &str) -> (String, Option<usize>, Option<usize>) {
    let mut rest = arg;
    let mut nums: Vec<usize> = Vec::new();
    for _ in 0..2 {
        let Some(ix) = rest.rfind(':') else {
            break;
        };
        let tail = &rest[ix + 1..];
        if tail.is_empty() || !tail.bytes().all(|b| b.is_ascii_digit()) {
            break;
        }
        let Ok(n) = tail.parse::<usize>() else {
            break;
        };
        nums.push(n);
        rest = &rest[..ix];
    }
    if rest.is_empty() {
        return (arg.to_string(), None, None);
    }
    match nums.len() {
        // Pushed col first, then line.
        2 => (rest.to_string(), Some(nums[1].max(1)), Some(nums[0].max(1))),
        1 => (rest.to_string(), Some(nums[0].max(1)), None),
        _ => (arg.to_string(), None, None),
    }
}

/// Finder / LaunchServices pass `file:///Users/me/notes.md` (sometimes
/// `file://localhost/Users/...`). Bare absolute paths are accepted too.
fn path_from_open_url(raw: &str) -> Option<PathBuf> {
    let decoded = if let Some(rest) = raw.strip_prefix("file:") {
        let rest = rest.trim_start_matches("//");
        let path = if let Some(p) = rest.strip_prefix("localhost") {
            p
        } else if rest.starts_with('/') {
            rest
        } else {
            rest.find('/').map(|i| &rest[i..]).unwrap_or(rest)
        };
        percent_decode(path)
    } else if Path::new(raw).is_absolute() {
        raw.to_string()
    } else {
        return None;
    };
    Some(windows_file_url_path(decoded))
}

/// `file:///C:/notes.md` becomes `/C:/notes.md` after slash-stripping.
fn windows_file_url_path(path: String) -> PathBuf {
    let b = path.as_bytes();
    if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' {
        PathBuf::from(&path[1..])
    } else {
        PathBuf::from(path)
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

const HELP: &str = "\
crabmd — a fast native markdown writer

Usage:
  crabmd
  crabmd <file.md>
  crabmd <file.md>:<line>[:<col>]   (zed-style jump; hidden markup picks nearest)
  crabmd --theme <name> <file.md>
  crabmd -w <file.md>
  crabmd --list-themes
  crabmd --install-desktop
  crabmd --uninstall-desktop

No path opens an untitled buffer (same as cmd-t). If <file.md> does not
exist, an empty markdown file is created.

Single instance (zed-style): the first call owns the process (one dock
icon); later calls forward over a socket and exit in ~ms. Default opens a
tab in the running window; -n opens a new window in the same process.

Flags:
  -w, --wait     Block the terminal until the window closes (default: detach)
  -t, --theme    Theme name (see --list-themes)
  -n, --new      New window in the running process
  -e, --existing Open a tab in the existing window (default)
  -a, --add      Same as -e (tab in the focused window)
  -r, --reuse    Same as -e (reuses the window, no new process)
  --install-desktop    Add CrabMD to Spotlight / Start Menu / app grid
  --uninstall-desktop  Remove the user-level desktop entry (not the brew cask)
  -h, --help     Show this help

Themes (OpenCode JSON, default: from ~/.config/crabmd/config.toml or opencode):
  see --list-themes (all crabcode themes + grokday/groknight included)

Keys (Helix / Vim normal — tabbed buffers):
  cmd-alt-left/right  prev / next tab (ctrl-alt- works too)
  cmd-t         new tab (untitled)
  cmd-w         close tab (prompts when dirty)
  cmd-shift-n   new window
  cmd-shift-w   close window (prompts when any tab is dirty)
  cmd-q         quit the app (prompts per window, removes the socket)
  cmd/ctrl-s    save (explicit write; no autosave)
  cmd-shift-p   command palette (theme, editor, full width, source)
  cmd-shift-v   toggle markdown source view
  cmd-k t       theme picker (zed-style chord; cmd may stay held for t)
  cmd-,         settings
  cmd-f         find
  :w / :write   write from the command-line; :q closes the tab, :wq saves and closes it, :qa quits
  :bn / :bp     next / previous tab (buffer)
  /             insert a GFM block (headings, lists, code, table, quote, hr, alerts)
  h/j/k/l       h/l stay on the line; j/k wrap-aware file lines
  w/b/e W/B/E   word / WORD; 0/^/$ line start / first non-blank / line end
  gg/G          first / last line of the document
  i/a  I/A      insert at caret / after, line start (first non-blank) / line end
  o/O           open line below / above inside the block
  v             Helix select / Vim visual; Vim V = visual line
  d             delete selection (Helix: also current char); Vim dd = line, D = to EOL
  c             change: visual deletes + insert; Vim cc, ciw/caw/ci-quote/ci-brace
  m             Helix match: miw/maw/mi-quote/ma-brace/... select object
  viw/vaw       visual inner/around word; v/c + i/a + quotes/parens/braces too
  > < =         indent / dedent / auto-indent: >> << ==, visual >, >j >G gg=G
  %             Helix select all (Vim: ggVG)
  x             Helix: select line (repeat extends). Vim: delete character
  u / U         undo / redo (Helix); Vim redo is ctrl-r
  escape        normal (collapse visual)
  drop/paste    image + video files beside the markdown file
";

fn print_help() {
    println!("{HELP}");
}

#[cfg(test)]
mod tests {
    use super::{
        build_bundle_open_argv, cold_start_decision, format_open_file_arg, is_shell_entry_live,
        path_from_open_url, pick_live_target, split_file_position, Args, OpenBehavior,
    };
    use gpui::WindowId;
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

    #[test]
    fn zed_style_positions() {
        assert_eq!(
            split_file_position("notes.md:10:3"),
            ("notes.md".to_string(), Some(10), Some(3))
        );
        assert_eq!(
            split_file_position("notes.md:10"),
            ("notes.md".to_string(), Some(10), None)
        );
        assert_eq!(
            split_file_position("notes.md"),
            ("notes.md".to_string(), None, None)
        );
        assert_eq!(
            split_file_position("a:b.md:4"),
            ("a:b.md".to_string(), Some(4), None)
        );
        assert_eq!(
            split_file_position("notes.md:abc"),
            ("notes.md:abc".to_string(), None, None)
        );
    }

    #[test]
    fn file_urls_from_finder() {
        assert_eq!(
            path_from_open_url("file:///Users/me/notes.md"),
            Some(PathBuf::from("/Users/me/notes.md"))
        );
        assert_eq!(
            path_from_open_url("file://localhost/Users/me/notes.md"),
            Some(PathBuf::from("/Users/me/notes.md"))
        );
        assert_eq!(
            path_from_open_url("file:///Users/me/My%20Notes.md"),
            Some(PathBuf::from("/Users/me/My Notes.md"))
        );
        assert_eq!(
            path_from_open_url("/Users/me/notes.md"),
            Some(PathBuf::from("/Users/me/notes.md"))
        );
        assert_eq!(path_from_open_url("notes.md"), None);
        assert_eq!(
            path_from_open_url("file:///C:/notes.md"),
            Some(PathBuf::from("C:/notes.md"))
        );
    }

    fn wid(n: u64) -> WindowId {
        WindowId::from(n)
    }

    #[test]
    fn shell_liveness_needs_weak_and_window() {
        let live: HashSet<WindowId> = [wid(1), wid(2)].into_iter().collect();
        assert!(is_shell_entry_live(true, wid(1), &live));
        assert!(!is_shell_entry_live(true, wid(9), &live));
        assert!(!is_shell_entry_live(false, wid(1), &live));
        assert!(!is_shell_entry_live(false, wid(9), &live));
        let empty: HashSet<WindowId> = HashSet::new();
        assert!(!is_shell_entry_live(true, wid(1), &empty));
    }

    #[test]
    fn reopen_target_prefers_active_else_first() {
        assert_eq!(
            pick_live_target(Some(wid(2)), &[wid(1), wid(2), wid(3)]),
            Some(wid(2))
        );
        assert_eq!(pick_live_target(None, &[wid(7), wid(8)]), Some(wid(7)));
        assert_eq!(
            pick_live_target(Some(wid(99)), &[wid(1), wid(2)]),
            Some(wid(1))
        );
        assert_eq!(pick_live_target(None, &[]), None);
        assert_eq!(pick_live_target(Some(wid(1)), &[]), None);
    }

    #[test]
    fn open_file_arg_formats_positions() {
        assert_eq!(
            format_open_file_arg(Path::new("/tmp/notes.md"), None, None),
            "/tmp/notes.md"
        );
        assert_eq!(
            format_open_file_arg(Path::new("/tmp/notes.md"), Some(10), None),
            "/tmp/notes.md:10"
        );
        assert_eq!(
            format_open_file_arg(Path::new("/tmp/notes.md"), Some(10), Some(3)),
            "/tmp/notes.md:10:3"
        );
        let arg = format_open_file_arg(Path::new("/tmp/a b.md"), Some(4), Some(2));
        assert_eq!(
            split_file_position(&arg),
            ("/tmp/a b.md".to_string(), Some(4), Some(2))
        );
    }

    fn test_args(
        path: Option<&str>,
        line: Option<usize>,
        col: Option<usize>,
        behavior: OpenBehavior,
        theme: Option<&str>,
    ) -> Args {
        Args {
            help: false,
            list_themes: false,
            install_desktop: false,
            uninstall_desktop: false,
            theme: theme.unwrap_or("opencode").to_string(),
            theme_from_cli: theme.is_some(),
            wait: false,
            behavior,
            path: path.map(PathBuf::from),
            line,
            col,
        }
    }

    #[test]
    fn bundle_open_argv_preserves_cli_semantics() {
        let argv = build_bundle_open_argv(&test_args(None, None, None, OpenBehavior::New, None));
        assert!(argv.iter().any(|a| a == "-n"));
        assert!(!argv.iter().any(|a| a.to_string_lossy().contains(".md")));
        let argv = build_bundle_open_argv(&test_args(
            Some("rel/notes.md"),
            Some(10),
            Some(3),
            OpenBehavior::Existing,
            Some("grokday"),
        ));
        let joined: Vec<String> = argv
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(joined.contains(&"-e".to_string()));
        assert!(joined.contains(&"--theme".to_string()));
        assert!(joined.contains(&"grokday".to_string()));
        let file = joined.iter().find(|a| a.contains("notes.md")).unwrap();
        assert!(file.ends_with(":10:3"), "file carries line:col: {file}");
        assert!(
            Path::new(file.trim_end_matches(":10:3")).is_absolute(),
            "CLI paths must be absolutized: {file}"
        );
        let new_argv =
            build_bundle_open_argv(&test_args(None, None, None, OpenBehavior::New, None));
        let existing_argv =
            build_bundle_open_argv(&test_args(None, None, None, OpenBehavior::Existing, None));
        assert!(new_argv.iter().any(|a| a == "-n"));
        assert!(existing_argv.iter().any(|a| a == "-e"));
    }

    #[test]
    fn cold_start_routes_wait_vs_elect() {
        // Running app or held lock: wait-forward (open --args would be ignored).
        assert_eq!(cold_start_decision(true, false), "wait-forward");
        assert_eq!(cold_start_decision(false, true), "wait-forward");
        assert_eq!(cold_start_decision(true, true), "wait-forward");
        // No app, no lock: single elected starter uses open --args once.
        assert_eq!(cold_start_decision(false, false), "elect-starter");
    }
}
